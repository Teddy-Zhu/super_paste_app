use super::EventWatcher;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidActivateApplicationNotification};
use objc2_foundation::{NSNotification, NSOperationQueue};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tauri::AppHandle;

const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(2);
const MAIN_THREAD_POLL: Duration = Duration::from_millis(50);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Default)]
struct RegistrationState {
    cancelled: bool,
    observer: Option<usize>,
}

pub(super) fn watch(app: &AppHandle, cancelled: Arc<AtomicBool>) -> Result<EventWatcher, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("活跃应用监听已取消".into());
    }

    let registration_cancelled = Arc::new(AtomicBool::new(false));
    let registration = Arc::new(Mutex::new(RegistrationState::default()));
    let (events_tx, events) = mpsc::sync_channel(1);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let callback_events = events_tx.clone();
    let callback_registration = registration.clone();
    let callback_cancelled = cancelled.clone();
    let callback_registration_cancelled = registration_cancelled.clone();

    app.run_on_main_thread(move || {
        if registration_is_cancelled(
            &callback_registration,
            &callback_cancelled,
            &callback_registration_cancelled,
        ) {
            let _ = ready_tx.send(Err("活跃应用监听已取消".into()));
            return;
        }

        let workspace = NSWorkspace::sharedWorkspace();
        let notification_center = workspace.notificationCenter();
        let main_queue = NSOperationQueue::mainQueue();
        let callback = RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
            let _ = callback_events.try_send(Ok(()));
        });
        let observer = unsafe {
            notification_center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
                Some(&main_queue),
                &callback,
            )
        };
        let observer_address = Retained::into_raw(observer) as usize;
        let must_remove = {
            let mut state = callback_registration
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.cancelled
                || callback_cancelled.load(Ordering::Acquire)
                || callback_registration_cancelled.load(Ordering::Acquire)
            {
                state.cancelled = true;
                true
            } else {
                state.observer = Some(observer_address);
                false
            }
        };

        if must_remove {
            remove_observer(observer_address);
            let _ = ready_tx.send(Err("活跃应用监听已取消".into()));
        } else if ready_tx.send(Ok(())).is_err() {
            if let Some(observer_address) = take_observer(&callback_registration, true) {
                remove_observer(observer_address);
            }
        }
    })
    .map_err(|error| error.to_string())?;

    let deadline = Instant::now() + REGISTRATION_TIMEOUT;
    loop {
        if cancelled.load(Ordering::Acquire) {
            registration_cancelled.store(true, Ordering::Release);
            if let Some(observer_address) = take_observer(&registration, true) {
                dispatch_removal(app, observer_address);
            }
            return Err("活跃应用监听已取消".into());
        }

        let now = Instant::now();
        if now >= deadline {
            registration_cancelled.store(true, Ordering::Release);
            if let Some(observer_address) = take_observer(&registration, true) {
                dispatch_removal(app, observer_address);
            }
            return Err("等待 macOS 应用激活通知注册超时".into());
        }

        match ready_rx.recv_timeout((deadline - now).min(MAIN_THREAD_POLL)) {
            Ok(Ok(())) => break,
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                registration_cancelled.store(true, Ordering::Release);
                if let Some(observer_address) = take_observer(&registration, true) {
                    dispatch_removal(app, observer_address);
                }
                return Err("macOS 应用激活通知注册线程意外退出".into());
            }
        }
    }

    if cancelled.load(Ordering::Acquire) {
        registration_cancelled.store(true, Ordering::Release);
        if let Some(observer_address) = take_observer(&registration, true) {
            dispatch_removal(app, observer_address);
        }
        return Err("活跃应用监听已取消".into());
    }

    let observer_address = take_observer(&registration, false)
        .ok_or_else(|| "macOS 应用激活通知观察者未就绪".to_owned())?;
    let app = app.clone();
    Ok(EventWatcher::new(events, move || {
        dispatch_removal(&app, observer_address);
    }))
}

fn registration_is_cancelled(
    registration: &Mutex<RegistrationState>,
    cancelled: &AtomicBool,
    registration_cancelled: &AtomicBool,
) -> bool {
    registration
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cancelled
        || cancelled.load(Ordering::Acquire)
        || registration_cancelled.load(Ordering::Acquire)
}

fn take_observer(registration: &Mutex<RegistrationState>, cancel: bool) -> Option<usize> {
    let mut state = registration
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.cancelled |= cancel;
    state.observer.take()
}

fn remove_observer(observer_address: usize) {
    let Some(observer) =
        (unsafe { Retained::<AnyObject>::from_raw(observer_address as *mut AnyObject) })
    else {
        return;
    };
    let workspace = NSWorkspace::sharedWorkspace();
    let notification_center = workspace.notificationCenter();
    unsafe { notification_center.removeObserver(&observer) };
}

fn dispatch_removal(app: &AppHandle, observer_address: usize) {
    let (removed_tx, removed_rx) = mpsc::sync_channel(1);
    let removal = app.run_on_main_thread(move || {
        remove_observer(observer_address);
        let _ = removed_tx.send(());
    });
    match removal {
        Err(error) => tracing::warn!(
            %error,
            "无法派发 macOS 通知观察者清理；原生通知中心会在进程退出时释放观察者"
        ),
        Ok(()) if removed_rx.recv_timeout(CLEANUP_TIMEOUT).is_err() => {
            tracing::warn!("macOS 主线程暂未执行通知观察者清理；清理仍留在主线程队列中");
        }
        Ok(()) => {}
    }
}
