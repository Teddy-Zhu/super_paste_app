use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use tokio_util::sync::CancellationToken;

pub type PowerEventCallback = Arc<dyn Fn(bool) + Send + Sync + 'static>;

struct Dispatcher {
    sleeping: AtomicBool,
    callback: PowerEventCallback,
}

impl Dispatcher {
    fn new(callback: PowerEventCallback) -> Arc<Self> {
        Arc::new(Self {
            sleeping: AtomicBool::new(false),
            callback,
        })
    }

    fn notify(&self, sleeping: bool) {
        if self.sleeping.swap(sleeping, Ordering::AcqRel) != sleeping {
            (self.callback)(sleeping);
        }
    }
}

pub struct PowerLifecycleAdapter {
    cancellation: CancellationToken,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    #[cfg(target_os = "windows")]
    window: isize,
}

impl PowerLifecycleAdapter {
    pub fn start(callback: PowerEventCallback) -> Result<Self, String> {
        let cancellation = CancellationToken::new();
        let dispatcher = Dispatcher::new(callback);

        #[cfg(target_os = "linux")]
        let worker = linux::start(dispatcher, cancellation.clone())?;
        #[cfg(target_os = "macos")]
        let worker = macos::start(dispatcher, cancellation.clone())?;
        #[cfg(target_os = "windows")]
        let (worker, window) = windows::start(dispatcher)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let worker = {
            let _ = (dispatcher, cancellation.clone());
            return Err("当前平台不支持系统休眠事件".to_owned());
        };

        Ok(Self {
            cancellation,
            worker: Mutex::new(Some(worker)),
            #[cfg(target_os = "windows")]
            window,
        })
    }

    pub fn shutdown(&self) {
        self.cancellation.cancel();
        #[cfg(target_os = "windows")]
        windows::request_shutdown(self.window);

        let worker = match self.worker.lock() {
            Ok(mut worker) => worker.take(),
            Err(poisoned) => {
                tracing::error!("power lifecycle worker lock poisoned during shutdown");
                poisoned.into_inner().take()
            }
        };
        if worker.is_some_and(|worker| worker.join().is_err()) {
            tracing::warn!("power lifecycle worker panicked during shutdown");
        }
    }
}

impl Drop for PowerLifecycleAdapter {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use dbus::{blocking::SyncConnection, channel::MatchingReceiver, message::MatchRule};
    use std::sync::mpsc;
    use std::time::Duration;

    pub fn start(
        dispatcher: Arc<Dispatcher>,
        cancellation: CancellationToken,
    ) -> Result<thread::JoinHandle<()>, String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("power-lifecycle".to_owned())
            .spawn(move || {
                let connection = match SyncConnection::new_system() {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("连接 system D-Bus 失败：{error}")));
                        return;
                    }
                };
                let match_rule =
                    MatchRule::new_signal("org.freedesktop.login1.Manager", "PrepareForSleep")
                        .with_path("/org/freedesktop/login1")
                        .with_sender("org.freedesktop.login1");
                let match_string = match_rule.match_str();
                let signal_match = connection.start_receive(
                    match_rule,
                    Box::new(move |message, _connection| {
                        if let Some(sleeping) = message.get1::<bool>() {
                            dispatcher.notify(sleeping);
                        }
                        true
                    }),
                );
                if let Err(error) = connection.add_match_no_cb(&match_string) {
                    let _ = connection.stop_receive(signal_match);
                    let _ = ready_tx.send(Err(format!("注册系统休眠 D-Bus 监听失败：{error}")));
                    return;
                }
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                while !cancellation.is_cancelled() {
                    if let Err(error) = connection.process(Duration::from_millis(200)) {
                        tracing::warn!(%error, "system sleep D-Bus listener stopped");
                        break;
                    }
                }
                if let Err(error) = connection.remove_match(signal_match) {
                    tracing::warn!(%error, "failed to remove system sleep D-Bus listener");
                }
            })
            .map_err(|error| format!("启动系统休眠监听线程失败：{error}"))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(worker),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = worker.join();
                Err(format!("系统休眠监听线程未能初始化：{error}"))
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::NSNotification;
    use std::{ptr::NonNull, time::Duration};

    pub fn start(
        dispatcher: Arc<Dispatcher>,
        cancellation: CancellationToken,
    ) -> Result<thread::JoinHandle<()>, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("power-lifecycle".to_owned())
            .spawn(move || {
                let workspace = NSWorkspace::sharedWorkspace();
                let center = workspace.notificationCenter();
                let on_sleep = RcBlock::new({
                    let dispatcher = dispatcher.clone();
                    move |_notification: NonNull<NSNotification>| dispatcher.notify(true)
                });
                let on_wake = RcBlock::new(move |_notification: NonNull<NSNotification>| {
                    dispatcher.notify(false)
                });
                let sleep_observer = unsafe {
                    center.addObserverForName_object_queue_usingBlock(
                        Some(NSWorkspaceWillSleepNotification),
                        None,
                        None,
                        &on_sleep,
                    )
                };
                let wake_observer = unsafe {
                    center.addObserverForName_object_queue_usingBlock(
                        Some(NSWorkspaceDidWakeNotification),
                        None,
                        None,
                        &on_wake,
                    )
                };
                if ready_tx.send(()).is_err() {
                    unsafe {
                        center.removeObserver(sleep_observer.as_ref());
                        center.removeObserver(wake_observer.as_ref());
                    }
                    return;
                }
                while !cancellation.is_cancelled() {
                    thread::park_timeout(Duration::from_millis(200));
                }
                unsafe {
                    center.removeObserver(sleep_observer.as_ref());
                    center.removeObserver(wake_observer.as_ref());
                }
            })
            .map_err(|error| format!("启动系统休眠监听线程失败：{error}"))?;

        match ready_rx.recv() {
            Ok(()) => Ok(worker),
            Err(error) => {
                let _ = worker.join();
                Err(format!("macOS 系统休眠监听未能初始化：{error}"))
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::Dispatcher;
    use ::windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
                DispatchMessageW, GetMessageW, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL,
                PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PostMessageW, PostQuitMessage,
                RegisterClassW, TranslateMessage, UnregisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE,
                WM_CLOSE, WM_DESTROY, WM_POWERBROADCAST, WNDCLASSW,
            },
        },
        core::{Error, PCWSTR, w},
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, OnceLock, mpsc},
        thread,
    };

    static CALLBACKS: OnceLock<Mutex<HashMap<isize, Arc<Dispatcher>>>> = OnceLock::new();

    pub fn start(dispatcher: Arc<Dispatcher>) -> Result<(thread::JoinHandle<()>, isize), String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("power-lifecycle".to_owned())
            .spawn(move || unsafe {
                let module = match GetModuleHandleW(PCWSTR::null()) {
                    Ok(module) => HINSTANCE(module.0),
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("获取应用模块句柄失败：{error}")));
                        return;
                    }
                };
                let window_class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(window_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: module,
                    hIcon: Default::default(),
                    hCursor: Default::default(),
                    hbrBackground: Default::default(),
                    lpszMenuName: PCWSTR::null(),
                    lpszClassName: w!("SuperPastePowerLifecycle"),
                };
                if RegisterClassW(&window_class) == 0 {
                    let _ = ready_tx.send(Err(format!(
                        "注册系统电源事件窗口失败：{}",
                        Error::from_thread()
                    )));
                    return;
                }
                let window = match CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    window_class.lpszClassName,
                    PCWSTR::null(),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(module),
                    None,
                ) {
                    Ok(window) => window,
                    Err(error) => {
                        let _ = UnregisterClassW(window_class.lpszClassName, Some(module));
                        let _ = ready_tx.send(Err(format!("创建系统电源事件窗口失败：{error}")));
                        return;
                    }
                };
                CALLBACKS
                    .get_or_init(Default::default)
                    .lock()
                    .expect("power callback map poisoned")
                    .insert(window.0 as isize, dispatcher);
                if ready_tx.send(Ok(window.0 as isize)).is_err() {
                    let _ = DestroyWindow(window);
                    let _ = UnregisterClassW(window_class.lpszClassName, Some(module));
                    return;
                }
                let mut message = MSG::default();
                loop {
                    let result = GetMessageW(&mut message, None, 0, 0).0;
                    if result <= 0 {
                        break;
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                CALLBACKS
                    .get_or_init(Default::default)
                    .lock()
                    .expect("power callback map poisoned")
                    .remove(&(window.0 as isize));
                let _ = DestroyWindow(window);
                let _ = UnregisterClassW(window_class.lpszClassName, Some(module));
            })
            .map_err(|error| format!("启动系统休眠监听线程失败：{error}"))?;

        match ready_rx.recv() {
            Ok(Ok(window)) => Ok((worker, window)),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = worker.join();
                Err(format!("系统休眠监听线程未能初始化：{error}"))
            }
        }
    }

    pub fn request_shutdown(window: isize) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(window as *mut std::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_POWERBROADCAST => {
                let sleeping = match wparam.0 as u32 {
                    PBT_APMSUSPEND => Some(true),
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMECRITICAL | PBT_APMRESUMESUSPEND => {
                        Some(false)
                    }
                    _ => None,
                };
                if let Some(sleeping) = sleeping
                    && let Some(callback) = CALLBACKS
                        .get_or_init(Default::default)
                        .lock()
                        .ok()
                        .and_then(|callbacks| callbacks.get(&(window.0 as isize)).cloned())
                {
                    callback.notify(sleeping);
                    return LRESULT(1);
                }
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
            WM_CLOSE => {
                unsafe {
                    let _ = DestroyWindow(window);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }
}
