# 阶段 0：环境准备与基础设施

> 总入口：[`../tauri-migration-implementation-plan.md`](../tauri-migration-implementation-plan.md) ｜ 本阶段续篇：[`phase-0-part2.md`](./phase-0-part2.md) ｜ 下一阶段：[`phase-1.md`](./phase-1.md)
>
> ⚠️ **本阶段所有命令与路径均以新项目根目录 `super_paste_rs/` 为基准**：新项目创建在仓库子目录 `super_paste_rs/` 中，不要在仓库根目录创建。下文出现的 `src-tauri/...`、`src/...` 实际指 `super_paste_rs/src-tauri/...`、`super_paste_rs/src/...`。

## 步骤 0.1：安装 Rust 工具链

```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 配置环境变量
source $HOME/.cargo/env

# 验证安装
rustc --version
cargo --version

# 安装工具
cargo install cargo-edit
cargo install cargo-watch
cargo install cargo-nextest
```

## 步骤 0.2：安装系统依赖

**Linux (Ubuntu/Debian)**：
```bash
sudo apt update
sudo apt install -y \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    patchelf \
    libssl-dev \
    pkg-config
```

**macOS**：
```bash
xcode-select --install
```

**Windows**：
- 安装 Microsoft Visual Studio C++ Build Tools
- 安装 WebView2 Runtime

## 步骤 0.3：初始化 Tauri 项目

> 在仓库根目录 `super_paste_app/` 下新建子目录 `super_paste_rs/` 作为全新 Tauri 项目，**不要污染根目录的现有 Electron 项目**。后续所有命令都在 `super_paste_rs/` 内执行。

```bash
# 进入仓库根目录
cd /home/teddy/workspace/code/super_paste_app

# 创建并进入新项目子目录
mkdir -p super_paste_rs
cd super_paste_rs

# 初始化前端项目（Vue 3 + TS + Vite）
pnpm create vite . --template vue-ts

# 安装 Tauri CLI
pnpm add -D @tauri-apps/cli@latest

# 初始化 Tauri（在 super_paste_rs/ 内生成 src-tauri/）
pnpm tauri init
# App name: SuperPaste
# Window title: SuperPaste
# Web assets location: ../dist
# Dev server URL: http://localhost:5173
# Frontend dev command: pnpm dev
# Frontend build command: pnpm build
```

完成后，新项目结构应为 `super_paste_rs/`（前端）+ `super_paste_rs/src-tauri/`（Rust 后端）。

**配置 `src-tauri/Cargo.toml`**：
```toml
[package]
name = "superpaste"
version = "0.1.0"
edition = "2021"

[lib]
name = "superpaste_lib"
crate-type = ["lib", "cdylib", "staticlib"]

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-global-shortcut = "2"
tauri-plugin-shell = "2"
tauri-plugin-clipboard-manager = "2"
tauri-plugin-single-instance = "2"
tauri-plugin-autostart = "2"
tauri-plugin-process = "2"
tauri-plugin-dialog = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
tokio = { version = "1", features = ["sync", "rt-multi-thread", "fs"] }
rusqlite = { version = "0.36", features = ["bundled"] }
r2d2 = "0.8"
r2d2_sqlite = "0.30"
parking_lot = "0.12"
chrono = "0.4"
clipboard-rs = "0.2"
active-win-pos-rs = "0.8"
blake3 = "1.5"
uuid = { version = "1", features = ["v7"] }
reqwest = { version = "0.12", features = ["json", "stream"] }
reqwest_dav = "0.2"
oar-ocr = "0.6"
image = "0.25"
qrcode = "0.14"
rxing = "0.6"
zstd = "0.13"
regex = "1"
lazy_static = "1"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
```

**配置 `src-tauri/tauri.conf.json`**：
```json
{
  "$schema": "https://raw.githubusercontent.com/tauri-apps/tauri/dev/crates/tauri-config-schema/schema.json",
  "productName": "SuperPaste",
  "version": "0.1.0",
  "identifier": "com.v2dawn.superpaste",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:5173",
    "beforeDevCommand": "pnpm dev",
    "beforeBuildCommand": "pnpm build"
  },
  "app": {
    "windows": [
      {
        "label": "main",
        "title": "SuperPaste",
        "width": 1000,
        "height": 700,
        "visible": false,
        "center": true,
        "decorations": false,
        "resizable": true,
        "minWidth": 800,
        "minHeight": 600
      },
      {
        "label": "overlay",
        "title": "",
        "width": 380,
        "height": 520,
        "visible": false,
        "center": false,
        "x": 0,
        "y": 0,
        "alwaysOnTop": true,
        "skipTaskbar": true,
        "decorations": false,
        "transparent": true,
        "resizable": false,
        "movable": false,
        "minimizable": false,
        "maximizable": false
      }
    ],
    "trayIcon": {
      "iconPath": "icons/32x32.png",
      "iconAsTemplate": true
    },
    "security": {
      "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' asset: https://asset.localhost"
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```

## 步骤 0.4：实现错误处理体系

**创建 `src-tauri/src/error.rs`**：
```rust
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
            detail: detail.clone(),
            user_message: "无法访问数据库，请重启应用".to_string(),
        }
    }
    
    pub fn clipboard(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Clipboard {
            code: "CLIP_001".to_string(),
            detail: detail.clone(),
            user_message: "剪贴板访问失败，请检查系统权限".to_string(),
        }
    }
    
    pub fn ocr(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Ocr {
            code: "OCR_001".to_string(),
            detail: detail.clone(),
            user_message: "OCR 识别失败，请稍后重试".to_string(),
        }
    }
    
    pub fn sync(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Sync {
            code: "SYNC_001".to_string(),
            detail: detail.clone(),
            user_message: "同步失败，请检查网络连接".to_string(),
        }
    }
    
    pub fn config(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::Config {
            code: "CFG_001".to_string(),
            detail: detail.clone(),
            user_message: "配置错误，请检查设置".to_string(),
        }
    }
    
    pub fn not_found(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::NotFound {
            code: "NOT_FOUND".to_string(),
            detail: detail.clone(),
            user_message: "未找到请求的资源".to_string(),
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

impl From<AppError> for tauri::ipc::InvokeError {
    fn from(err: AppError) -> Self {
        tauri::ipc::InvokeError::from_serde_json(serde_json::to_value(&err).unwrap())
    }
}
```

**创建 `src-tauri/src/result.rs`**：
```rust
use crate::error::AppError;

pub type Result<T> = std::result::Result<T, AppError>;
```

## 步骤 0.5：实现日志系统

**创建 `src-tauri/src/infrastructure/logging.rs`**：
```rust
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use std::path::PathBuf;

pub fn init_logging(log_dir: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&log_dir)?;
    
    let file_appender = RollingFileAppender::new(
        Rotation::DAILY,
        log_dir,
        "superpaste.log"
    );
    
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_writer(std::io::stdout))
        .with(fmt::layer().with_writer(file_appender).with_ansi(false))
        .init();
    
    tracing::info!("日志系统初始化完成");
    Ok(())
}
```

**创建 `src-tauri/src/infrastructure/mod.rs`**：
```rust
pub mod logging;
pub mod database;
pub mod config;
pub mod event_bus;
pub mod active_window;
```

## 步骤 0.6：实现配置管理器

**创建 `src-tauri/src/infrastructure/config/manager.rs`**：
```rust
use parking_lot::RwLock;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::error::AppError;
use crate::result::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub max_history_size: usize,
    pub max_keep_days: i64,
    pub enable_local_ocr: bool,
    pub ocr_mode: String,
    pub ocr_remote_url: String,
    pub clipboard_listening: bool,
    pub overlay_position: String,
    pub global_shortcut: String,
    pub auto_start: bool,
    pub theme: String,
    pub language: String,
    pub sync_enabled: bool,
    pub sync_server_url: String,
    pub sync_username: String,
    pub sync_password: String,
    pub sync_remote_path: String,
    pub sync_strategy: String,
    pub last_sync_at: Option<i64>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            max_history_size: 1000,
            max_keep_days: -1,
            enable_local_ocr: false,
            ocr_mode: "local".to_string(),
            ocr_remote_url: String::new(),
            clipboard_listening: true,
            overlay_position: "bottom".to_string(),
            global_shortcut: "CmdOrCtrl+Shift+V".to_string(),
            auto_start: false,
            theme: "auto".to_string(),
            language: "zh-CN".to_string(),
            sync_enabled: false,
            sync_server_url: String::new(),
            sync_username: String::new(),
            sync_password: String::new(),
            sync_remote_path: "/superpaste/clipboard.db".to_string(),
            sync_strategy: "merge_by_time".to_string(),
            last_sync_at: None,
        }
    }
}

pub struct ConfigManager {
    cache: RwLock<HashMap<String, String>>,
}

impl ConfigManager {
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut stmt = conn.prepare("SELECT key, value FROM app_config")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut map = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }

        Ok(Self { cache: RwLock::new(map) })
    }

    pub fn get(&self) -> AppConfig {
        let cache = self.cache.read();
        AppConfig {
            max_history_size: cache.get("max_history_size")
                .and_then(|v| v.parse().ok()).unwrap_or(1000),
            max_keep_days: cache.get("max_keep_days")
                .and_then(|v| v.parse().ok()).unwrap_or(-1),
            enable_local_ocr: cache.get("enable_local_ocr")
                .map(|v| v == "true").unwrap_or(false),
            ocr_mode: cache.get("ocr_mode")
                .cloned().unwrap_or_else(|| "local".to_string()),
            ocr_remote_url: cache.get("ocr_remote_url")
                .cloned().unwrap_or_default(),
            clipboard_listening: cache.get("clipboard_listening")
                .map(|v| v == "true").unwrap_or(true),
            overlay_position: cache.get("overlay_position")
                .cloned().unwrap_or_else(|| "bottom".to_string()),
            global_shortcut: cache.get("global_shortcut")
                .cloned().unwrap_or_else(|| "CmdOrCtrl+Shift+V".to_string()),
            auto_start: cache.get("auto_start")
                .map(|v| v == "true").unwrap_or(false),
            theme: cache.get("theme")
                .cloned().unwrap_or_else(|| "auto".to_string()),
            language: cache.get("language")
                .cloned().unwrap_or_else(|| "zh-CN".to_string()),
            sync_enabled: cache.get("sync_enabled")
                .map(|v| v == "true").unwrap_or(false),
            sync_server_url: cache.get("sync_server_url")
                .cloned().unwrap_or_default(),
            sync_username: cache.get("sync_username")
                .cloned().unwrap_or_default(),
            sync_password: cache.get("sync_password")
                .cloned().unwrap_or_default(),
            sync_remote_path: cache.get("sync_remote_path")
                .cloned().unwrap_or_else(|| "/superpaste/clipboard.db".to_string()),
            sync_strategy: cache.get("sync_strategy")
                .cloned().unwrap_or_else(|| "merge_by_time".to_string()),
            last_sync_at: cache.get("last_sync_at")
                .and_then(|v| v.parse().ok()),
        }
    }

    pub fn set(&self, conn: &Connection, key: &str, value: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO app_config (key, value, value_type, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?4",
            rusqlite::params![key, value, Self::infer_type(value), now],
        )?;
        self.cache.write().insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn infer_type(value: &str) -> &'static str {
        if value == "true" || value == "false" { "boolean" }
        else if value.parse::<f64>().is_ok() { "number" }
        else if value.starts_with('{') || value.starts_with('[') { "json" }
        else { "string" }
    }
}
```

**创建 `src-tauri/src/infrastructure/config/mod.rs`**：
```rust
pub mod manager;
```

继续下一部分...
