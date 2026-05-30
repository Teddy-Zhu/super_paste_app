use serde::Serialize;
use tauri::State;

use super::IpcResponse;
use crate::app::state::AppState;
use crate::infrastructure::database::repositories::AppRepository;

#[derive(Serialize)]
pub struct AppInfoResponse {
    pub id: String,
    pub application: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

#[derive(Serialize)]
pub struct AppCacheStatsResponse {
    #[serde(rename = "cacheSize")]
    pub cache_size: usize,
    #[serde(rename = "maxCacheSize")]
    pub max_cache_size: usize,
}

/// 获取所有应用信息
#[tauri::command]
pub fn get_all_apps(state: State<AppState>) -> IpcResponse<Vec<AppInfoResponse>> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    match AppRepository::list(&conn) {
        Ok(apps) => {
            let response: Vec<AppInfoResponse> = apps
                .iter()
                .map(|a| AppInfoResponse {
                    id: a.id.clone(),
                    application: a.name.clone(),
                    path: a.executable_path.clone(),
                    icon: None,
                    created_at: a.first_seen_at,
                    updated_at: a.last_seen_at,
                })
                .collect();
            IpcResponse::ok(response)
        }
        Err(e) => IpcResponse::err(format!("获取应用列表失败: {}", e)),
    }
}

/// 获取应用缓存统计
#[tauri::command]
pub fn get_app_cache_stats() -> IpcResponse<AppCacheStatsResponse> {
    IpcResponse::ok(AppCacheStatsResponse {
        cache_size: 0,
        max_cache_size: 1000,
    })
}
