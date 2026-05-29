## 步骤 0.7：实现事件总线

**创建 `src-tauri/src/infrastructure/event_bus.rs`**：
```rust
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use crate::types::events::InternalEvent;

pub struct EventBus {
    channels: RwLock<HashMap<String, broadcast::Sender<InternalEvent>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    pub fn publish(&self, event: InternalEvent) {
        let event_type = event.event_type();
        let channels = self.channels.read();
        
        if let Some(tx) = channels.get(event_type) {
            let _ = tx.send(event);
        }
    }

    pub fn subscribe(&self, event_type: &str) -> broadcast::Receiver<InternalEvent> {
        let mut channels = self.channels.write();
        
        channels
            .entry(event_type.to_string())
            .or_insert_with(|| broadcast::channel(100).0)
            .subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
```

**创建 `src-tauri/src/types/events.rs`**：
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InternalEvent {
    ClipboardChanged {
        content_id: String,
        primary_type: String,
    },
    OcrCompleted {
        content_id: String,
        text: String,
    },
    SyncStarted,
    SyncCompleted {
        merged_count: usize,
    },
    SyncFailed {
        error: String,
    },
}

impl InternalEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ClipboardChanged { .. } => "clipboard:changed",
            Self::OcrCompleted { .. } => "ocr:completed",
            Self::SyncStarted => "sync:started",
            Self::SyncCompleted { .. } => "sync:completed",
            Self::SyncFailed { .. } => "sync:failed",
        }
    }
}
```

**创建 `src-tauri/src/types/mod.rs`**：
```rust
pub mod clipboard;
pub mod events;
pub mod sync;
```

## 步骤 0.8：实现活动窗口追踪

**创建 `src-tauri/src/infrastructure/active_window.rs`**：
```rust
use active_win_pos_rs::get_active_window;
use crate::types::clipboard::AppInfo;
use crate::result::Result;

pub struct ActiveWindowTracker;

impl ActiveWindowTracker {
    pub fn new() -> Self {
        Self
    }
    
    pub fn get_active_app_id(&self) -> Option<String> {
        match get_active_window() {
            Ok(window) => {
                let id = if let Some(ref bundle) = window.app_name {
                    format!("app_{}", blake3::hash(bundle.as_bytes()).to_hex())
                } else {
                    format!("app_{}", blake3::hash(window.process_path.as_bytes()).to_hex())
                };
                Some(id)
            }
            Err(e) => {
                tracing::debug!("获取活动窗口失败: {}", e);
                None
            }
        }
    }
    
    pub fn get_active_app_info(&self) -> Result<Option<AppInfo>> {
        match get_active_window() {
            Ok(window) => {
                let now = chrono::Utc::now().timestamp();
                let id = if let Some(ref bundle) = window.app_name {
                    format!("app_{}", blake3::hash(bundle.as_bytes()).to_hex())
                } else {
                    format!("app_{}", blake3::hash(window.process_path.as_bytes()).to_hex())
                };
                
                Ok(Some(AppInfo {
                    id,
                    name: window.title,
                    bundle_id: window.app_name,
                    executable_path: window.process_path,
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
```

## 步骤 0.9：定义类型

**创建 `src-tauri/src/types/clipboard.rs`**：
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: Option<i64>,
    pub content_id: String,
    pub content_hash: String,
    pub primary_type: String,
    pub data_size: i64,
    pub available_formats: Vec<String>,
    pub source_app_id: Option<String>,
    pub is_pinned: bool,
    pub is_favorite: bool,
    pub created_at: i64,
    pub last_used_at: i64,
    pub use_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardContent {
    pub content_id: String,
    pub text_content: Option<String>,
    pub html_content: Option<String>,
    pub rtf_content: Option<String>,
    pub image_data: Option<Vec<u8>>,
    pub files_json: Option<String>,
    pub metadata_json: Option<String>,
    pub compression_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub bundle_id: Option<String>,
    pub executable_path: String,
    pub icon_hash: Option<String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryOptions {
    pub offset: i64,
    pub limit: i64,
    pub primary_type: Option<String>,
    pub tag_id: Option<String>,
    pub is_pinned: Option<bool>,
    pub is_favorite: Option<bool>,
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 50,
            primary_type: None,
            tag_id: None,
            is_pinned: None,
            is_favorite: None,
            start_date: None,
            end_date: None,
        }
    }
}
```

**创建 `src-tauri/src/types/sync.rs`**：
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ConflictStrategy {
    LocalWins,
    RemoteWins,
    MergeByTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled: bool,
    pub server_url: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
    pub strategy: ConflictStrategy,
}
```

## 步骤 0.10：实现应用状态

**创建 `src-tauri/src/app/state.rs`**：
```rust
use std::sync::Arc;
use crate::infrastructure::{
    database::pool::DatabasePool,
    config::manager::ConfigManager,
    event_bus::EventBus,
    active_window::ActiveWindowTracker,
};

pub struct AppState {
    pub db_pool: Arc<DatabasePool>,
    pub config: Arc<ConfigManager>,
    pub event_bus: Arc<EventBus>,
    pub active_window: Arc<ActiveWindowTracker>,
}

impl AppState {
    pub fn new(
        db_pool: DatabasePool,
        config: ConfigManager,
    ) -> Self {
        Self {
            db_pool: Arc::new(db_pool),
            config: Arc::new(config),
            event_bus: Arc::new(EventBus::new()),
            active_window: Arc::new(ActiveWindowTracker::new()),
        }
    }
}
```

**创建 `src-tauri/src/app/mod.rs`**：
```rust
pub mod state;
pub mod setup;
pub mod lifecycle;
```

## 步骤 0.11：实现工具函数

**创建 `src-tauri/src/utils/hash.rs`**：
```rust
pub fn hash_content(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

pub fn generate_content_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
```

**创建 `src-tauri/src/utils/compression.rs`**：
```rust
use crate::result::Result;

pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    Ok(zstd::encode_all(data, 3)?)
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    Ok(zstd::decode_all(data)?)
}

pub fn should_compress(data: &[u8]) -> bool {
    data.len() > 1024
}
```

**创建 `src-tauri/src/utils/time.rs`**：
```rust
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "Invalid timestamp".to_string())
}
```

**创建 `src-tauri/src/utils/performance.rs`**：
```rust
use std::time::Instant;

pub struct PerformanceMonitor {
    start: Instant,
    name: String,
}

impl PerformanceMonitor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            start: Instant::now(),
            name: name.into(),
        }
    }
    
    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }
    
    pub fn log(&self) {
        let duration = self.elapsed();
        tracing::debug!("[性能] {} 耗时: {:?}", self.name, duration);
        
        if duration.as_millis() > 100 {
            tracing::warn!("[性能] {} 耗时过长: {:?}", self.name, duration);
        }
    }
}

impl Drop for PerformanceMonitor {
    fn drop(&mut self) {
        self.log();
    }
}
```

**创建 `src-tauri/src/utils/mod.rs`**：
```rust
pub mod hash;
pub mod compression;
pub mod time;
pub mod performance;
```

## 步骤 0.12：创建常量文件

**创建 `src-tauri/src/constants.rs`**：
```rust
pub const APP_NAME: &str = "SuperPaste";
pub const DB_FILE_NAME: &str = "superpaste.db";
pub const MAX_IMAGE_SIZE: usize = 10 * 1024 * 1024; // 10MB
pub const MAX_TEXT_SIZE: usize = 1024 * 1024; // 1MB
pub const COMPRESSION_THRESHOLD: usize = 1024; // 1KB
pub const DEFAULT_PAGE_SIZE: i64 = 50;
pub const MAX_PAGE_SIZE: i64 = 200;
```

## 步骤 0.13：前端错误处理

**创建 `src/utils/error-handler.ts`**：
```typescript
import { ElMessage, ElNotification } from 'element-plus'

export interface ApiError {
  type: string
  code: string
  detail: string
  userMessage: string
}

export function handleApiError(error: any, silent = false): void {
  const apiError = parseError(error)
  
  if (!silent) {
    ElMessage.error(apiError.userMessage)
  }
  
  console.error(`[${apiError.code}] ${apiError.detail}`)
  
  if (import.meta.env.PROD) {
    reportError(apiError)
  }
}

function parseError(error: any): ApiError {
  if (error && typeof error === 'object' && error.type && error.userMessage) {
    return {
      type: error.type,
      code: error.code || 'UNKNOWN',
      detail: error.detail || error.message || String(error),
      userMessage: error.userMessage,
    }
  }
  
  if (typeof error === 'string') {
    return {
      type: 'Unknown',
      code: 'UNKNOWN',
      detail: error,
      userMessage: error,
    }
  }
  
  return {
    type: 'Unknown',
    code: 'UNKNOWN',
    detail: String(error),
    userMessage: '操作失败，请稍后重试',
  }
}

function reportError(error: ApiError): void {
  console.log('Error reported:', error)
}

export function showErrorNotification(error: any, title = '错误'): void {
  const apiError = parseError(error)
  
  ElNotification({
    title,
    message: apiError.userMessage,
    type: 'error',
    duration: 5000,
  })
  
  console.error(`[${apiError.code}] ${apiError.detail}`)
}

export function isNetworkError(error: any): boolean {
  const apiError = parseError(error)
  return apiError.type === 'Sync' || apiError.code.startsWith('SYNC_')
}

export function isDatabaseError(error: any): boolean {
  const apiError = parseError(error)
  return apiError.type === 'Database' || apiError.code.startsWith('DB_')
}
```

## 验收标准

- [ ] Rust 工具链安装完成
- [ ] 系统依赖安装完成
- [ ] Tauri 项目初始化成功
- [ ] 错误处理体系可用
- [ ] 日志系统正常工作
- [ ] 配置管理器可读写配置
- [ ] 事件总线可发布订阅事件
- [ ] 活动窗口追踪在三平台正常工作
- [ ] 类型定义完整
- [ ] 应用状态管理正常
- [ ] 工具函数可用
- [ ] 前端错误处理正常
