use std::sync::{Arc, atomic::AtomicBool, mpsc::Receiver};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

pub(crate) struct EventWatcher {
    pub events: Receiver<Result<(), String>>,
    cleanup: Option<Box<dyn FnOnce() + Send>>,
}

impl EventWatcher {
    pub(crate) fn new(
        events: Receiver<Result<(), String>>,
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
    _app: &tauri::AppHandle,
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
