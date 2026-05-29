# 阶段 2-5：完整实施步骤

## 阶段 2：剪贴板核心功能

### 2.1 实现剪贴板监听器
```rust
// src-tauri/src/services/clipboard/watcher.rs
use clipboard_rs::{Clipboard, ClipboardContext, ClipboardWatcher, ClipboardWatcherContext};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct ClipboardWatcherService {
    tx: mpsc::UnboundedSender<()>,
}

impl ClipboardWatcherService {
    pub fn start() -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        
        std::thread::spawn(move || {
            let ctx = ClipboardContext::new().unwrap();
            let watcher = ClipboardWatcher::new(&ctx, move |_| {
                let _ = tx.send(());
            });
            watcher.start_watch();
        });
        
        Self { tx }
    }
}
```

### 2.2 实现剪贴板转换器
```rust
// src-tauri/src/services/clipboard/converter.rs
use clipboard_rs::{Clipboard, ClipboardContext};
use crate::types::clipboard::{ClipboardItem, ClipboardContent};
use crate::utils::{hash, time};

pub struct ClipboardConverter;

impl ClipboardConverter {
    pub fn read_and_convert(source_app_id: Option<String>) -> Result<(ClipboardItem, ClipboardContent)> {
        let ctx = ClipboardContext::new()?;
        let content_id = hash::generate_content_id();
        
        // 读取文本
        let text = ctx.get_text().ok();
        
        // 读取图片
        let image = ctx.get_image().ok();
        
        // 计算哈希
        let hash_data = text.as_ref().map(|t| t.as_bytes())
            .or_else(|| image.as_ref().map(|i| i.as_slice()))
            .unwrap_or(&[]);
        let content_hash = hash::hash_content(hash_data);
        
        // 确定类型
        let primary_type = if text.is_some() { "text" }
        else if image.is_some() { "image" }
        else { "unknown" };
        
        let now = time::now();
        
        let item = ClipboardItem {
            id: None,
            content_id: content_id.clone(),
            content_hash,
            primary_type: primary_type.to_string(),
            data_size: hash_data.len() as i64,
            available_formats: vec![primary_type.to_string()],
            source_app_id,
            is_pinned: false,
            is_favorite: false,
            created_at: now,
            last_used_at: now,
            use_count: 1,
        };
        
        let content = ClipboardContent {
            content_id,
            text_content: text,
            html_content: None,
            rtf_content: None,
            image_data: image,
            files_json: None,
            metadata_json: None,
            compression_type: None,
        };
        
        Ok((item, content))
    }
}
```

### 2.3 实现去重器
```rust
// src-tauri/src/services/clipboard/deduplicator.rs
use crate::infrastructure::database::repositories::ClipboardRepository;
use rusqlite::Connection;

pub struct Deduplicator;

impl Deduplicator {
    pub fn update_if_exists(&self, conn: &Connection, hash: &str) -> Result<bool> {
        if let Some(id) = ClipboardRepository::get_by_hash(conn, hash)? {
            ClipboardRepository::update_last_used(conn, id)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
```

### 2.4 实现剪贴板管理器
```rust
// src-tauri/src/services/clipboard/manager.rs
use crate::infrastructure::{database::pool::DatabasePool, event_bus::EventBus, active_window::ActiveWindowTracker};
use crate::infrastructure::database::repositories::{ClipboardRepository, AppRepository};
use crate::types::events::InternalEvent;

pub struct ClipboardManager;

impl ClipboardManager {
    pub async fn handle_change(
        db_pool: &DatabasePool,
        event_bus: &EventBus,
        active_window: &ActiveWindowTracker,
    ) -> Result<()> {
        // 获取活动窗口
        if let Some(app_info) = active_window.get_active_app_info()? {
            let conn = db_pool.get()?;
            AppRepository::upsert(&conn, &app_info)?;
        }
        
        // 读取剪贴板
        let source_app_id = active_window.get_active_app_id();
        let (item, content) = ClipboardConverter::read_and_convert(source_app_id)?;
        
        // 去重
        let conn = db_pool.get()?;
        let dedup = Deduplicator;
        if dedup.update_if_exists(&conn, &item.content_hash)? {
            return Ok(());
        }
        
        // 存储
        ClipboardRepository::insert(&conn, &item, &content)?;
        
        // 发布事件
        event_bus.publish(InternalEvent::ClipboardChanged {
            content_id: item.content_id.clone(),
            primary_type: item.primary_type.clone(),
        });
        
        Ok(())
    }
}
```

### 2.5 实现系统托盘
```rust
// src-tauri/src/system/tray.rs
use tauri::{AppHandle, Manager, menu::{Menu, MenuItem}, tray::TrayIconBuilder};

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    
    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "quit" => app.exit(0),
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                _ => {}
            }
        })
        .build(app)?;
    
    Ok(())
}
```

### 2.6 实现全局快捷键
```rust
// src-tauri/src/system/shortcuts.rs
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub struct ShortcutManager;

impl ShortcutManager {
    pub fn register_with_fallback(handle: &tauri::AppHandle, preferred: &str) -> Result<String> {
        if let Ok(shortcut) = Self::parse_shortcut(preferred) {
            if Self::try_register(handle, &shortcut).is_ok() {
                return Ok(preferred.to_string());
            }
        }
        
        let fallbacks = vec!["Ctrl+Shift+V", "Ctrl+Alt+V", "Alt+Shift+V"];
        for s in fallbacks {
            if let Ok(shortcut) = Self::parse_shortcut(s) {
                if Self::try_register(handle, &shortcut).is_ok() {
                    let _ = handle.emit("shortcut:changed", s);
                    return Ok(s.to_string());
                }
            }
        }
        
        Err(AppError::config("所有快捷键均被占用"))
    }
    
    fn parse_shortcut(s: &str) -> Result<Shortcut> {
        let parts: Vec<&str> = s.split('+').collect();
        let mut modifiers = Modifiers::empty();
        let mut key_code = None;
        
        for part in parts {
            match part.trim() {
                "Ctrl" | "Control" => modifiers |= Modifiers::CONTROL,
                "Shift" => modifiers |= Modifiers::SHIFT,
                "Alt" => modifiers |= Modifiers::ALT,
                "CmdOrCtrl" => {
                    #[cfg(target_os = "macos")]
                    { modifiers |= Modifiers::SUPER; }
                    #[cfg(not(target_os = "macos"))]
                    { modifiers |= Modifiers::CONTROL; }
                }
                key => key_code = Some(Self::parse_key(key)?),
            }
        }
        
        Ok(Shortcut::new(Some(modifiers), key_code.ok_or_else(|| AppError::config("无效快捷键"))?))
    }
    
    fn parse_key(s: &str) -> Result<Code> {
        match s.to_uppercase().as_str() {
            "V" => Ok(Code::KeyV),
            "C" => Ok(Code::KeyC),
            _ => Err(AppError::config(format!("不支持的按键: {}", s))),
        }
    }
    
    fn try_register(handle: &tauri::AppHandle, shortcut: &Shortcut) -> Result<()> {
        handle.global_shortcut().register(*shortcut)?;
        handle.global_shortcut().on_shortcut(*shortcut, |app, _, event| {
            if event.state == ShortcutState::Pressed {
                let _ = app.emit("toggle-overlay", ());
            }
        })?;
        Ok(())
    }
}
```

### 2.7 实现 Overlay 窗口定位
```rust
// src-tauri/src/system/overlay.rs
use tauri::{Manager, PhysicalPosition, PhysicalSize};

pub struct OverlayPositioner;

impl OverlayPositioner {
    pub fn calculate_position(
        position: &str,
        window_size: PhysicalSize<u32>,
        screen_size: PhysicalSize<u32>,
    ) -> PhysicalPosition<i32> {
        let margin = 50i32;
        match position {
            "top" => PhysicalPosition::new(
                ((screen_size.width - window_size.width) / 2) as i32, margin
            ),
            "bottom" => PhysicalPosition::new(
                ((screen_size.width - window_size.width) / 2) as i32,
                (screen_size.height - window_size.height) as i32 - margin
            ),
            "left" => PhysicalPosition::new(
                margin, ((screen_size.height - window_size.height) / 2) as i32
            ),
            "right" => PhysicalPosition::new(
                (screen_size.width - window_size.width) as i32 - margin,
                ((screen_size.height - window_size.height) / 2) as i32
            ),
            _ => PhysicalPosition::new(
                ((screen_size.width - window_size.width) / 2) as i32,
                ((screen_size.height - window_size.height) / 2) as i32
            ),
        }
    }
}

pub async fn show_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    let overlay = app.get_webview_window("overlay").ok_or_else(|| AppError::not_found("Overlay 窗口"))?;
    let monitor = overlay.current_monitor()?.ok_or_else(|| AppError::config("无法获取显示器"))?;
    let screen_size = monitor.size();
    let window_size = overlay.outer_size()?;
    let pos = OverlayPositioner::calculate_position(position, window_size, screen_size);
    overlay.set_position(pos)?;
    overlay.show()?;
    overlay.set_focus()?;
    Ok(())
}
```

---

## 阶段 3：高级功能

### 3.1 OCR 模型下载器
```rust
// src-tauri/src/services/ocr/model_downloader.rs
use reqwest::Client;
use std::path::{Path, PathBuf};

const MODEL_CHECKSUMS: &[(&str, &str)] = &[
    ("ppocr_det.onnx", "HASH_HERE"),
    ("ppocr_rec.onnx", "HASH_HERE"),
    ("ppocr_keys_v1.txt", "HASH_HERE"),
];

pub struct ModelDownloader {
    cache_dir: PathBuf,
    cdn_url: String,
    client: Client,
}

impl ModelDownloader {
    pub async fn ensure_models(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.cache_dir).await?;
        
        for (name, expected_hash) in MODEL_CHECKSUMS {
            let local_path = self.cache_dir.join(name);
            
            if local_path.exists() && self.verify_file(&local_path, expected_hash).await? {
                continue;
            }
            
            self.download_model(name, &local_path).await?;
            
            if !self.verify_file(&local_path, expected_hash).await? {
                return Err(AppError::ocr(format!("模型 {} 校验失败", name)));
            }
        }
        
        Ok(())
    }
    
    async fn verify_file(&self, path: &Path, expected_hash: &str) -> Result<bool> {
        let data = tokio::fs::read(path).await?;
        let hash = blake3::hash(&data);
        Ok(hash.to_hex().as_str() == expected_hash)
    }
    
    async fn download_model(&self, name: &str, dest: &Path) -> Result<()> {
        let url = format!("{}/{}", self.cdn_url, name);
        let response = self.client.get(&url).send().await?;
        let bytes = response.bytes().await?;
        tokio::fs::write(dest, bytes).await?;
        Ok(())
    }
}
```

### 3.2 WebDAV 同步
```rust
// src-tauri/src/services/sync/webdav.rs
use reqwest_dav::{Auth, Client, ClientBuilder};

pub struct WebDavSync {
    client: Client,
    remote_path: String,
    local_db_path: PathBuf,
}

impl WebDavSync {
    pub async fn sync_bidirectional(&self, strategy: ConflictStrategy) -> Result<()> {
        let temp_db = std::env::temp_dir().join("superpaste_remote.db");
        let data = self.client.get(&self.remote_path).await?;
        tokio::fs::write(&temp_db, data).await?;
        
        let result = ConflictResolver::merge_databases(&self.local_db_path, &temp_db, strategy)?;
        
        self.upload().await?;
        tokio::fs::remove_file(temp_db).await?;
        
        Ok(())
    }
}
```

### 3.3 冲突解决器
```rust
// src-tauri/src/services/sync/conflict_resolver.rs
pub struct ConflictResolver;

impl ConflictResolver {
    pub fn merge_databases(local: &Path, remote: &Path, strategy: ConflictStrategy) -> Result<MergeResult> {
        let local_conn = Connection::open(local)?;
        let remote_conn = Connection::open(remote)?;
        
        let remote_only = Self::find_remote_only(&local_conn, &remote_conn)?;
        for (item, content) in remote_only {
            ClipboardRepository::insert(&local_conn, &item, &content)?;
        }
        
        let conflicts = Self::find_conflicts(&local_conn, &remote_conn)?;
        for conflict in &conflicts {
            match strategy {
                ConflictStrategy::MergeByTime => {
                    if conflict.remote.last_used_at > conflict.local.last_used_at {
                        Self::update_metadata(&local_conn, &conflict.remote)?;
                    }
                }
                _ => {}
            }
        }
        
        Ok(MergeResult { merged_count: remote_only.len(), conflict_count: conflicts.len() })
    }
}
```

---

## 阶段 4：UI/UX 优化

### 4.1 前端 API 封装
```typescript
// src/api/clipboard.ts
import { invoke } from '@tauri-apps/api/core'
import { handleApiError } from '@/utils/error-handler'

export const clipboardApi = {
  async getHistory(options?: QueryOptions) {
    try {
      return await invoke<ClipboardItem[]>('get_clipboard_history', options)
    } catch (error) {
      handleApiError(error)
      throw error
    }
  },
  
  async deleteItem(id: number) {
    try {
      await invoke('delete_clipboard_item', { id })
    } catch (error) {
      handleApiError(error)
      throw error
    }
  },
}
```

### 4.2 Pinia Store
```typescript
// src/stores/clipboard.ts
import { defineStore } from 'pinia'
import { clipboardApi } from '@/api/clipboard'

export const useClipboardStore = defineStore('clipboard', {
  state: () => ({
    items: [] as ClipboardItem[],
    loading: false,
  }),
  
  actions: {
    async fetchHistory() {
      this.loading = true
      try {
        this.items = await clipboardApi.getHistory()
      } finally {
        this.loading = false
      }
    },
  },
})
```

---

## 阶段 5：打包发布

### 5.1 配置打包
```json
// src-tauri/tauri.conf.json
{
  "bundle": {
    "active": true,
    "targets": ["deb", "appimage", "dmg", "msi"],
    "icon": ["icons/icon.icns", "icons/icon.ico"],
    "identifier": "com.v2dawn.superpaste",
    "publisher": "V2Dawn",
    "copyright": "Copyright © 2026",
    "category": "Utility",
    "shortDescription": "跨平台剪贴板管理工具",
    "longDescription": "功能强大的剪贴板历史管理工具"
  }
}
```

### 5.2 构建命令
```bash
# 开发模式
pnpm tauri dev

# 生产构建
pnpm tauri build

# 指定平台
pnpm tauri build --target x86_64-pc-windows-msvc
pnpm tauri build --target x86_64-apple-darwin
pnpm tauri build --target x86_64-unknown-linux-gnu
```

---

## 验收标准总览

**阶段 2**：
- [ ] 剪贴板监听正常
- [ ] 活动窗口追踪正常
- [ ] 去重功能正常
- [ ] 托盘菜单可用
- [ ] 全局快捷键可用
- [ ] Overlay 窗口定位正确

**阶段 3**：
- [ ] OCR 模型下载校验正常
- [ ] WebDAV 同步正常
- [ ] 冲突解决正常

**阶段 4**：
- [ ] 前端 API 正常
- [ ] 状态管理正常
- [ ] UI 交互流畅

**阶段 5**：
- [ ] 三平台打包成功
- [ ] 安装包可正常安装运行
