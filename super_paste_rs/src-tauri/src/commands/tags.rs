use serde::{Deserialize, Serialize};
use tauri::State;

use super::{IpcResponse, SimpleResponse};
use crate::app::state::AppState;
use crate::infrastructure::database::repositories::TagRepository;
use crate::types::clipboard::Tag;
use crate::utils::{hash, time};

#[derive(Serialize)]
pub struct TagGroup {
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

#[derive(Serialize)]
pub struct TagStats {
    #[serde(rename = "tagId")]
    pub tag_id: String,
    #[serde(rename = "tagName")]
    pub tag_name: String,
    #[serde(rename = "recordCount")]
    pub record_count: i64,
}

#[derive(Deserialize)]
pub struct CreateTagParams {
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateTagParams {
    #[serde(rename = "tagId")]
    pub tag_id: String,
    pub updates: TagUpdates,
}

#[derive(Deserialize)]
pub struct TagUpdates {
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
}

/// 获取所有标签
#[tauri::command]
pub fn get_all_tags(state: State<AppState>) -> IpcResponse<Vec<TagGroup>> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    match TagRepository::list(&conn) {
        Ok(tags) => {
            let groups: Vec<TagGroup> = tags
                .iter()
                .map(|t| TagGroup {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    color: t.color.clone(),
                    description: t.description.clone(),
                    created_at: t.created_at,
                    updated_at: t.updated_at,
                })
                .collect();
            IpcResponse::ok(groups)
        }
        Err(e) => IpcResponse::err(format!("获取标签失败: {}", e)),
    }
}

/// 创建标签
#[tauri::command]
pub fn create_tag(
    state: State<AppState>,
    name: String,
    color: String,
    description: Option<String>,
) -> IpcResponse<serde_json::Value> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    let now = time::now();
    let tag = Tag {
        id: hash::generate_content_id(),
        name,
        color,
        icon: None,
        description,
        sort_order: 0,
        created_at: now,
        updated_at: now,
    };

    let tag_id = tag.id.clone();

    match TagRepository::insert(&conn, &tag) {
        Ok(_) => {
            let data = serde_json::json!({ "tagId": tag_id });
            IpcResponse::ok(data)
        }
        Err(e) => IpcResponse::err(format!("创建标签失败: {}", e)),
    }
}

/// 更新标签
#[tauri::command]
pub fn update_tag(
    state: State<AppState>,
    tag_id: String,
    updates: TagUpdates,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    // 获取现有标签
    let tags = match TagRepository::list(&conn) {
        Ok(tags) => tags,
        Err(e) => return SimpleResponse::err(format!("获取标签失败: {}", e)),
    };

    let existing = match tags.iter().find(|t| t.id == tag_id) {
        Some(tag) => tag,
        None => return SimpleResponse::err("标签不存在"),
    };

    let now = time::now();
    let updated_tag = Tag {
        id: tag_id,
        name: updates.name.unwrap_or(existing.name.clone()),
        color: updates.color.unwrap_or(existing.color.clone()),
        icon: existing.icon.clone(),
        description: updates.description.or(existing.description.clone()),
        sort_order: existing.sort_order,
        created_at: existing.created_at,
        updated_at: now,
    };

    match TagRepository::update(&conn, &updated_tag) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("更新标签失败: {}", e)),
    }
}

/// 删除标签
#[tauri::command]
pub fn delete_tag(state: State<AppState>, tag_id: String) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    match TagRepository::delete(&conn, &tag_id) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("删除标签失败: {}", e)),
    }
}

/// 获取标签统计
#[tauri::command]
pub fn get_tag_stats(state: State<AppState>) -> IpcResponse<Vec<TagStats>> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    // 获取所有标签
    let tags = match TagRepository::list(&conn) {
        Ok(tags) => tags,
        Err(e) => return IpcResponse::err(format!("获取标签失败: {}", e)),
    };

    // TODO: 统计每个标签关联的记录数
    let stats: Vec<TagStats> = tags
        .iter()
        .map(|t| TagStats {
            tag_id: t.id.clone(),
            tag_name: t.name.clone(),
            record_count: 0,
        })
        .collect();

    IpcResponse::ok(stats)
}

/// 绑定标签到记录
#[tauri::command]
pub fn bind_tag_to_record(
    state: State<AppState>,
    content_id: String,
    tag_id: String,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    match TagRepository::add_to_item(&conn, &content_id, &tag_id) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("绑定标签失败: {}", e)),
    }
}

/// 解绑标签
#[tauri::command]
pub fn unbind_tag_from_record(
    state: State<AppState>,
    content_id: String,
    tag_id: String,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    match TagRepository::remove_from_item(&conn, &content_id, &tag_id) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("解绑标签失败: {}", e)),
    }
}
