use tauri::{Emitter, Manager};

use crate::types::state_update::StateUpdate;

/// 前端发送状态变更到后端
/// 后端接收后更新配置，并广播给所有窗口（source = "backend"）
#[tauri::command]
pub fn sync_state_update(app: tauri::AppHandle, update: StateUpdate) {
    // 只处理前端发起的变更
    if !update.is_frontend() {
        return;
    }

    tracing::debug!("收到前端状态变更: {:?}", update.data);

    // 广播给所有窗口（source 标记为 backend，避免循环）
    let broadcast = StateUpdate::from_backend(update.data);
    let _ = app.emit("state-changed", &broadcast);
}

/// 后端发起状态变更并广播给所有窗口
pub fn broadcast_state_change(app: &tauri::AppHandle, data: serde_json::Value) {
    let broadcast = StateUpdate::from_backend(data);
    let _ = app.emit("state-changed", &broadcast);
}
