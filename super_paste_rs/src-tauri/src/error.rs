use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
#[serde(tag = "type")]
pub enum AppError {
    #[error("数据库错误")]
    Database {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },

    #[error("剪贴板错误")]
    Clipboard {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },

    #[error("OCR 错误")]
    Ocr {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },

    #[error("同步错误")]
    Sync {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },

    #[error("配置错误")]
    Config {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },

    #[error("未找到")]
    NotFound {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },

    #[error("无效参数")]
    InvalidArgument {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },

    #[error("未知错误")]
    Unknown {
        code: String,
        detail: String,
        #[serde(rename = "userMessage")]
        user_message: String,
    },
}

impl AppError {
    pub fn database(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Database {
            code: "DB_001".to_string(),
            detail,
            user_message: "无法访问数据库，请重启应用".to_string(),
        }
    }

    pub fn clipboard(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Clipboard {
            code: "CLIP_001".to_string(),
            detail,
            user_message: "剪贴板访问失败，请检查系统权限".to_string(),
        }
    }

    pub fn ocr(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Ocr {
            code: "OCR_001".to_string(),
            detail,
            user_message: "OCR 识别失败，请稍后重试".to_string(),
        }
    }

    pub fn sync(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Sync {
            code: "SYNC_001".to_string(),
            detail,
            user_message: "同步失败，请检查网络连接".to_string(),
        }
    }

    pub fn config(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Config {
            code: "CFG_001".to_string(),
            detail,
            user_message: "配置错误，请检查设置".to_string(),
        }
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::NotFound {
            code: "NOT_FOUND".to_string(),
            detail,
            user_message: "未找到请求的资源".to_string(),
        }
    }

    pub fn invalid_argument(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::InvalidArgument {
            code: "ARG_001".to_string(),
            detail,
            user_message: "参数无效，请检查输入".to_string(),
        }
    }

    pub fn unknown(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Unknown {
            code: "UNKNOWN".to_string(),
            detail,
            user_message: "操作失败，请稍后重试".to_string(),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        Self::database(err.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::Unknown {
            code: "IO_001".to_string(),
            detail: err.to_string(),
            user_message: "文件操作失败".to_string(),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        Self::Unknown {
            code: "JSON_001".to_string(),
            detail: err.to_string(),
            user_message: "数据格式错误".to_string(),
        }
    }
}

impl From<r2d2::Error> for AppError {
    fn from(err: r2d2::Error) -> Self {
        Self::database(format!("数据库连接池错误: {}", err))
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        Self::Unknown {
            code: "TAURI_001".to_string(),
            detail: err.to_string(),
            user_message: "系统操作失败".to_string(),
        }
    }
}
