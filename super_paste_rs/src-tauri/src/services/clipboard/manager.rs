use std::sync::Arc;

use crate::infrastructure::active_window::ActiveWindowTracker;
use crate::infrastructure::database::pool::DatabasePool;
use crate::infrastructure::database::repositories::{AppRepository, ClipboardRepository};
use crate::infrastructure::event_bus::EventBus;
use crate::result::Result;
use crate::services::clipboard::converter::ClipboardConverter;
use crate::services::clipboard::deduplicator::Deduplicator;
use crate::types::events::InternalEvent;

pub struct ClipboardManager {
    db_pool: Arc<DatabasePool>,
    event_bus: Arc<EventBus>,
    active_window: Arc<ActiveWindowTracker>,
}

impl ClipboardManager {
    pub fn new(
        db_pool: Arc<DatabasePool>,
        event_bus: Arc<EventBus>,
        active_window: Arc<ActiveWindowTracker>,
    ) -> Self {
        Self {
            db_pool,
            event_bus,
            active_window,
        }
    }

    /// 处理剪贴板变化：读取、去重、存储、发布事件。
    pub fn handle_change(&self) -> Result<()> {
        // 获取活动窗口信息并入库
        if let Some(app_info) = self.active_window.get_active_app_info()? {
            let conn = self.db_pool.get()?;
            AppRepository::upsert(&conn, &app_info)?;
        }

        // 读取剪贴板
        let source_app_id = self.active_window.get_active_app_id();
        let (item, content) = ClipboardConverter::read_and_convert(source_app_id)?;

        // 去重
        let conn = self.db_pool.get()?;
        let dedup = Deduplicator;
        if dedup.update_if_exists(&conn, &item.content_hash)? {
            tracing::debug!("剪贴板内容重复，已更新 last_used");
            return Ok(());
        }

        // 存储
        ClipboardRepository::insert(&conn, &item, &content)?;
        tracing::info!("新剪贴板条目已保存: {}", item.content_id);

        // 发布事件
        self.event_bus.publish(InternalEvent::ClipboardChanged {
            content_id: item.content_id.clone(),
            primary_type: item.primary_type.clone(),
        });

        Ok(())
    }
}
