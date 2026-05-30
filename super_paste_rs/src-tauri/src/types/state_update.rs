use serde::{Deserialize, Serialize};

/// 前后端状态同步更新
/// source 标识变更来源，避免循环更新
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateUpdate {
    pub source: String,
    pub data: serde_json::Value,
}

impl StateUpdate {
    pub fn from_backend(data: serde_json::Value) -> Self {
        Self {
            source: "backend".to_string(),
            data,
        }
    }

    pub fn from_frontend(data: serde_json::Value) -> Self {
        Self {
            source: "frontend".to_string(),
            data,
        }
    }

    pub fn is_frontend(&self) -> bool {
        self.source == "frontend"
    }

    pub fn is_backend(&self) -> bool {
        self.source == "backend"
    }
}
