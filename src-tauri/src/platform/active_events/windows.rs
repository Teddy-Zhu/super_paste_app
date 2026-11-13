use super::EventWatcher;
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
};
use tauri::AppHandle;
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::{
            DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW,
            PostThreadMessageW, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_QUIT,
        },
    },
};

thread_local! {
    static ACTIVATION_EVENTS: RefCell<Option<SyncSender<Result<(), String>>>> = const { RefCell::new(None) };
}

pub(super) fn watch(
    _app: &AppHandle,
    cancelled: std::sync::Arc<AtomicBool>,
) -> Result<EventWatcher, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("活跃窗口监听已取消".into());
    }
    let (events_tx, events) = mpsc::sync_channel(1);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let thread_events = events_tx.clone();
    let listener_cancelled = cancelled.clone();
    let listener = thread::Builder::new()
        .name("active-window-win-event".into())
        .spawn(move || unsafe {
            let thread_id = windows::Win32::System::Threading::GetCurrentThreadId();
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
            if ready_tx.send(Ok(thread_id)).is_err() {
                let _ = UnhookWinEvent(hook);
                ACTIVATION_EVENTS.with(|slot| *slot.borrow_mut() = None);
                return;
            }

            loop {
                let result = GetMessageW(&mut message, None, 0, 0);
                if result.0 <= 0 {
                    if result.0 < 0 {
                        let _ = thread_events_for_current().map(|events| {
                            let _ = events.try_send(Err("Windows 前台窗口消息循环失败".into()));
                        });
                    }
                    break;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            let _ = UnhookWinEvent(hook);
            ACTIVATION_EVENTS.with(|slot| *slot.borrow_mut() = None);
        })
        .map_err(|error| error.to_string())?;
    let thread_id = ready_rx
        .recv()
        .map_err(|_| "Windows 前台窗口监听线程意外退出".to_owned())??;

    Ok(EventWatcher::new(events, move || {
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        let _ = listener.join();
    }))
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

fn thread_events_for_current() -> Option<SyncSender<Result<(), String>>> {
    ACTIVATION_EVENTS.with(|slot| slot.borrow().clone())
}
