use crossbeam_channel as mpsc;
// Adapted from Super Paste, fc90106d5a25233fe45a6ec9facd1ad3ffc4823b; MIT.
use std::sync::{Arc, atomic::AtomicBool};

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos;
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

pub(crate) struct EventWatcher {
    pub events: mpsc::Receiver<Result<(), String>>,
    cleanup: Option<Box<dyn FnOnce() + Send>>,
}

impl EventWatcher {
    pub(crate) fn new(
        events: mpsc::Receiver<Result<(), String>>,
        cleanup: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            events,
            cleanup: Some(Box::new(cleanup)),
        }
    }
}

impl Drop for EventWatcher {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

pub(crate) fn watch(
    _app: &crate::main_thread::MainThread,
    cancelled: Arc<AtomicBool>,
) -> Result<EventWatcher, String> {
    #[cfg(target_os = "linux")]
    {
        if clipboard_core::is_wayland_session() {
            return Err("当前 Wayland 桌面没有通用活动窗口事件接口".into());
        }
        return x11::watch(cancelled);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::watch(_app, cancelled);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::watch(_app, cancelled);
    }
    #[allow(unreachable_code)]
    Err("当前平台没有可用的活动窗口事件接口".into())
}
