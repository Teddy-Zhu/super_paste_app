use clipboard_rs::{
    Clipboard, ClipboardContext, ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext,
    WatcherShutdown,
};
use std::sync::{Arc, Mutex};

use crate::services::clipboard::manager::ClipboardManager;

/// 剪贴板监听器的 Handler 实现（实现 ClipboardHandler trait）。
struct ClipboardChangeHandler {
    manager: Arc<ClipboardManager>,
    ctx: ClipboardContext,
}

impl ClipboardChangeHandler {
    fn new(manager: Arc<ClipboardManager>) -> Self {
        let ctx = ClipboardContext::new().expect("无法创建 ClipboardContext");
        Self { manager, ctx }
    }
}

impl ClipboardHandler for ClipboardChangeHandler {
    fn on_clipboard_change(&mut self) {
        tracing::debug!("检测到剪贴板变化");
        if let Err(e) = self.manager.handle_change() {
            tracing::error!("处理剪贴板变化失败: {:?}", e);
        }
    }
}

/// 剪贴板监听服务（封装 ClipboardWatcherContext）。
pub struct ClipboardWatcherService {
    shutdown: Arc<Mutex<Option<WatcherShutdown>>>,
}

impl ClipboardWatcherService {
    /// 启动监听器（在独立线程中运行 watcher.start_watch()）。
    pub fn start(manager: Arc<ClipboardManager>) -> Self {
        let handler = ClipboardChangeHandler::new(manager);
        let mut watcher = ClipboardWatcherContext::new().expect("无法创建 ClipboardWatcher");
        let shutdown = watcher.add_handler(handler).get_shutdown_channel();
        let shutdown = Arc::new(Mutex::new(Some(shutdown)));

        let shutdown_clone = shutdown.clone();
        std::thread::spawn(move || {
            tracing::info!("剪贴板监听器已启动");
            watcher.start_watch();
            // start_watch 阻塞直到 stop 被调用
            tracing::info!("剪贴板监听器已退出");
        });

        Self {
            shutdown: shutdown_clone,
        }
    }

    /// 停止监听（调用 shutdown.stop()）。
    pub fn stop(&self) {
        if let Some(shutdown) = self.shutdown.lock().unwrap().take() {
            shutdown.stop();
            tracing::info!("剪贴板监听器已停止");
        }
    }
}
