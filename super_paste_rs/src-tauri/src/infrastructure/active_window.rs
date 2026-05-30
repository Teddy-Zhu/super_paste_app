use active_win_pos_rs::get_active_window;

use crate::result::Result;
use crate::types::clipboard::AppInfo;

pub struct ActiveWindowTracker;

impl ActiveWindowTracker {
    pub fn new() -> Self {
        Self
    }

    /// 计算活动应用的稳定 ID（优先 app_name，否则用可执行文件路径）。
    fn compute_id(app_name: &str, process_path: &str) -> String {
        let key = if !app_name.is_empty() {
            app_name
        } else {
            process_path
        };
        format!("app_{}", blake3::hash(key.as_bytes()).to_hex())
    }

    pub fn get_active_app_id(&self) -> Option<String> {
        match get_active_window() {
            Ok(window) => {
                let path = window.process_path.to_string_lossy();
                Some(Self::compute_id(&window.app_name, &path))
            }
            Err(_) => {
                tracing::debug!("获取活动窗口失败");
                None
            }
        }
    }

    pub fn get_active_app_info(&self) -> Result<Option<AppInfo>> {
        match get_active_window() {
            Ok(window) => {
                let now = chrono::Utc::now().timestamp();
                let executable_path = window.process_path.to_string_lossy().to_string();
                let id = Self::compute_id(&window.app_name, &executable_path);
                let bundle_id = if window.app_name.is_empty() {
                    None
                } else {
                    Some(window.app_name)
                };

                Ok(Some(AppInfo {
                    id,
                    name: window.title,
                    bundle_id,
                    executable_path,
                    icon_hash: None,
                    first_seen_at: now,
                    last_seen_at: now,
                }))
            }
            Err(_) => Ok(None),
        }
    }
}

impl Default for ActiveWindowTracker {
    fn default() -> Self {
        Self::new()
    }
}
