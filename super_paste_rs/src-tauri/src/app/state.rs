use std::path::PathBuf;
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
    pub db_path: PathBuf,
}

impl AppState {
    pub fn new(db_pool: DatabasePool, config: ConfigManager, db_path: PathBuf) -> Self {
        Self {
            db_pool: Arc::new(db_pool),
            config: Arc::new(config),
            event_bus: Arc::new(EventBus::new()),
            active_window: Arc::new(ActiveWindowTracker::new()),
            db_path,
        }
    }
}
