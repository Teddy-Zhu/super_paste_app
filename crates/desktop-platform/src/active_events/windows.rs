use crossbeam_channel as mpsc;
// Adapted from Super Paste, fc90106d5a25233fe45a6ec9facd1ad3ffc4823b; MIT.
use super::EventWatcher;
use std::{
    cell::RefCell,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    thread,
};
use crate::main_thread::MainThread as AppHandle;
use windows::Win32::{
    Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, INFINITE, SetEvent},
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::{
            DispatchMessageW, EVENT_SYSTEM_FOREGROUND, MSG, MWMO_INPUTAVAILABLE,
            MsgWaitForMultipleObjectsEx, PM_NOREMOVE, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
            TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_QUIT,
        },
    },
};

thread_local! {
    static ACTIVATION_EVENTS: RefCell<Option<mpsc::Sender<Result<(), String>>>> = const { RefCell::new(None) };
}

pub(super) fn watch(
    _app: &AppHandle,
    cancelled: std::sync::Arc<AtomicBool>,
) -> Result<EventWatcher, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("活跃窗口监听已取消".into());
    }
    let stop = unsafe { CreateEventW(None, true, false, None) }
        .map_err(|error| format!("无法创建 Windows 前台窗口监听停止事件: {error}"))?;
    // Both owners keep the kernel handle valid until the listener has exited.
    let stop = Arc::new(unsafe { OwnedHandle::from_raw_handle(stop.0) });
    let listener_stop = stop.clone();
    let (events_tx, events) = mpsc::bounded(1);
    let (ready_tx, ready_rx) = mpsc::bounded::<Result<(), String>>(1);
    let thread_events = events_tx.clone();
    let listener_cancelled = cancelled.clone();
    let listener = thread::Builder::new()
        .name("active-window-win-event".into())
        .spawn(move || unsafe {
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            ACTIVATION_EVENTS.with(|slot| *slot.borrow_mut() = Some(thread_events));
            if listener_cancelled.load(Ordering::Acquire) {
                ACTIVATION_EVENTS.with(|slot| *slot.borrow_mut() = None);
                let _ = ready_tx.send(Err("活跃窗口监听已取消".into()));
                return;
            }

            let hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(on_foreground_changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            if hook.is_invalid() {
                ACTIVATION_EVENTS.with(|slot| *slot.borrow_mut() = None);
                let _ = ready_tx.send(Err("Windows 前台窗口事件钩子创建失败".into()));
                return;
            }
            if ready_tx.send(Ok(())).is_ok() {
                if let Err(error) = message_loop(HANDLE(listener_stop.as_raw_handle())) {
                    tracing::error!(%error, "Windows 前台窗口事件监听失败");
                    if let Some(events) = thread_events_for_current() {
                        let _ = events.try_send(Err(error));
                    }
                }
            }

            if !UnhookWinEvent(hook).as_bool() {
                tracing::error!("移除 Windows 前台窗口事件钩子失败");
            }
            ACTIVATION_EVENTS.with(|slot| *slot.borrow_mut() = None);
        })
        .map_err(|error| error.to_string())?;
    let ready = ready_rx
        .recv()
        .map_err(|_| "Windows 前台窗口监听线程意外退出".to_owned())
        .and_then(|result| result);
    if let Err(error) = ready {
        stop_listener(&stop, listener);
        return Err(error);
    }

    Ok(EventWatcher::new(events, move || {
        stop_listener(&stop, listener);
    }))
}

fn message_loop(stop: HANDLE) -> Result<(), String> {
    let handles = [stop];
    let mut message = MSG::default();
    loop {
        // A kernel event cannot be rejected by a full posted-message queue.
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(Some(&handles), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if result == WAIT_OBJECT_0 { return Ok(()); }
        if result == WAIT_FAILED {
            let error = windows::core::Error::from_thread();
            return Err(format!("Windows 前台窗口消息等待失败: {error}"));
        }
        if result.0 != WAIT_OBJECT_0.0 + handles.len() as u32 {
            return Err(format!("Windows 前台窗口消息等待返回异常状态: {}", result.0));
        }
        // Revisit the stop event even while messages arrive continuously.
        // INPUTAVAILABLE also wakes for messages already seen by PeekMessage.
        for _ in 0..64 {
            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() { break; }
            if message.message == WM_QUIT { return Ok(()); }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

fn stop_listener(stop: &OwnedHandle, listener: thread::JoinHandle<()>) {
    if let Err(error) = unsafe { SetEvent(HANDLE(stop.as_raw_handle())) } {
        tracing::error!(%error, "发送 Windows 前台窗口监听停止事件失败");
    }
    if listener.join().is_err() {
        tracing::error!("Windows 前台窗口监听线程异常退出");
    }
}

unsafe extern "system" fn on_foreground_changed(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _window: windows::Win32::Foundation::HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    ACTIVATION_EVENTS.with(|slot| {
        if let Some(events) = slot.borrow().as_ref() {
            let _ = events.try_send(Ok(()));
        }
    });
}

fn thread_events_for_current() -> Option<mpsc::Sender<Result<(), String>>> {
    ACTIVATION_EVENTS.with(|slot| slot.borrow().clone())
}
