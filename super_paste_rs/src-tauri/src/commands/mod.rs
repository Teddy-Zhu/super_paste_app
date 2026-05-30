pub mod clipboard;
pub mod config;
pub mod system;
pub mod store;
pub mod tags;
pub mod apps;

use serde::Serialize;

/// 通用 IPC 响应格式（与原 Electron 项目兼容）
#[derive(Serialize)]
pub struct IpcResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apps: Option<serde_json::Value>,
}

impl<T: Serialize> IpcResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
            error: None,
            apps: None,
        }
    }

    pub fn ok_with_message(data: T, message: impl Into<String>) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: Some(message.into()),
            error: None,
            apps: None,
        }
    }

    pub fn ok_with_apps(data: T, apps: serde_json::Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
            error: None,
            apps: Some(apps),
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            message: None,
            error: Some(error.into()),
            apps: None,
        }
    }
}

/// 不需要 data 字段的简单响应
#[derive(Serialize)]
pub struct SimpleResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SimpleResponse {
    pub fn ok() -> Self {
        Self {
            success: true,
            message: None,
            error: None,
        }
    }

    pub fn ok_with_message(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(message.into()),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            message: None,
            error: Some(error.into()),
        }
    }
}
