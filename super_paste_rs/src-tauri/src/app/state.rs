use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::infrastructure::{
    active_window::ActiveWindowTracker, config::manager::ConfigManager,
    database::pool::DatabasePool, event_bus::EventBus,
};

pub struct AppState {
    pub db_pool: Arc<DatabasePool>,
    pub config: Arc<ConfigManager>,
    pub event_bus: Arc<EventBus>,
    pub active_window: Arc<ActiveWindowTracker>,
    /// 剪贴板监听开关（与 ClipboardManager 共享，运行时可切换）
    pub clipboard_listening: Arc<AtomicBool>,
    pub db_path: PathBuf,
}

impl AppState {
    pub fn new(db_pool: DatabasePool, config: ConfigManager, db_path: PathBuf) -> Self {
        let listening = config.get().clipboard_listening;
        Self {
            db_pool: Arc::new(db_pool),
            config: Arc::new(config),
            event_bus: Arc::new(EventBus::new()),
            active_window: Arc::new(ActiveWindowTracker::new()),
            clipboard_listening: Arc::new(AtomicBool::new(listening)),
            db_path,
        }
    }
}
