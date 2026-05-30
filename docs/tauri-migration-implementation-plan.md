# SuperPaste Tauri 迁移实施计划

> 完整可执行版本 | 无需兼容性考虑 | 全新实现

---

## 项目位置（重要）

> ⚠️ **全新项目必须创建在子目录 `super_paste_rs/` 中，不要在仓库根目录创建。**

- 现有 Electron 项目保留在仓库根目录 `/home/teddy/workspace/code/super_paste_app/`，作为功能参考。
- Tauri 新项目的根目录为：`/home/teddy/workspace/code/super_paste_app/super_paste_rs/`。
- **本计划及所有子计划中出现的相对路径，均以 `super_paste_rs/` 为根**。例如：
  - 文档写 `src-tauri/src/main.rs` → 实际路径为 `super_paste_rs/src-tauri/src/main.rs`
  - 文档写 `src/main.ts` → 实际路径为 `super_paste_rs/src/main.ts`

最终目录布局：

```
super_paste_app/                 # 仓库根目录（保留现有 Electron 项目）
├── src/                         # 现有 Electron 源码（参考用，不改动）
├── docs/                        # 迁移计划文档
│   ├── tauri-migration-implementation-plan.md   # 本文件（总入口）
│   └── implementation/          # 分阶段详细子计划
└── super_paste_rs/              # ★ 新建：Tauri 项目根目录
    ├── package.json
    ├── src/                     # Vue 前端
    └── src-tauri/               # Rust 后端
```

---

## 技术栈

### 后端（Rust）
- `tauri` v2 - 应用框架
- `clipboard-rs` - 剪贴板操作（支持 Wayland）
- `rusqlite` + `r2d2` - 数据库 + 连接池
- `oar-ocr` - OCR 引擎
- `rxing` + `qrcode` - 二维码
- `reqwest_dav` - WebDAV 同步
- `active-win-pos-rs` - 活动窗口追踪
- `thiserror` - 错误处理
- `tracing` - 日志
- `blake3` - 哈希
- `zstd` - 压缩

### 前端（Vue 3 + TypeScript）
- Vue 3.5 + Composition API
- Pinia 3 - 状态管理
- Vue Router 4 - 路由
- vue-i18n 11 - 国际化
- `@tauri-apps/api` v2 - Tauri API
- `@vueuse/core` - 工具库

### Tauri 插件
- `global-shortcut` - 全局快捷键
- `clipboard-manager` - 剪贴板
- `single-instance` - 单实例
- `autostart` - 开机自启
- `shell` - 外部链接
- `process` - 应用控制
- `dialog` - 对话框
- `updater` - 自动更新

---

## 架构分层

```
前端 Vue 3
  ↓ invoke/listen
Tauri Commands + Events
  ↓
Commands 层（参数校验 + 转发）
  ↓
Services 层（业务逻辑）
  ↓
Infrastructure 层（数据库/配置/事件）
  ↓
System 层（托盘/快捷键/窗口）
```

---

## 数据库 Schema

```sql
-- 应用信息
CREATE TABLE apps (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    bundle_id TEXT,
    executable_path TEXT NOT NULL,
    icon_hash TEXT,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    UNIQUE(bundle_id, executable_path)
) STRICT;

-- 剪贴板历史
CREATE TABLE clipboard_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id TEXT NOT NULL UNIQUE,
    content_hash TEXT NOT NULL,
    primary_type TEXT NOT NULL,
    data_size INTEGER NOT NULL,
    available_formats TEXT NOT NULL DEFAULT '[]',
    source_app_id TEXT,
    is_pinned INTEGER DEFAULT 0,
    is_favorite INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    use_count INTEGER DEFAULT 1,
    FOREIGN KEY (source_app_id) REFERENCES apps(id) ON DELETE SET NULL
) STRICT;

-- 内容存储
CREATE TABLE clipboard_content (
    content_id TEXT PRIMARY KEY,
    text_content TEXT,
    html_content TEXT,
    rtf_content TEXT,
    image_data BLOB,
    files_json TEXT,
    metadata_json TEXT,
    compression_type TEXT DEFAULT 'none',
    FOREIGN KEY (content_id) REFERENCES clipboard_items(content_id) ON DELETE CASCADE
) STRICT;

-- OCR 结果
CREATE TABLE ocr_results (
    content_id TEXT PRIMARY KEY,
    text_content TEXT NOT NULL,
    language TEXT,
    confidence REAL,
    processed_at INTEGER NOT NULL,
    FOREIGN KEY (content_id) REFERENCES clipboard_items(content_id) ON DELETE CASCADE
) STRICT;

-- 标签
CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    color TEXT NOT NULL,
    icon TEXT,
    description TEXT,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- 标签关联
CREATE TABLE item_tags (
    content_id TEXT NOT NULL,
    tag_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (content_id, tag_id),
    FOREIGN KEY (content_id) REFERENCES clipboard_items(content_id) ON DELETE CASCADE,
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
) STRICT;

-- 配置
CREATE TABLE app_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    value_type TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- 使用统计
CREATE TABLE usage_stats (
    date TEXT PRIMARY KEY,
    total_copies INTEGER DEFAULT 0,
    total_pastes INTEGER DEFAULT 0,
    total_searches INTEGER DEFAULT 0,
    stats_json TEXT,
    created_at INTEGER NOT NULL
) STRICT;

-- 全文搜索
CREATE VIRTUAL TABLE clipboard_fts USING fts5(
    content_id UNINDEXED,
    text_content,
    html_content,
    ocr_text,
    tokenize='trigram'
);

-- 索引
CREATE INDEX idx_items_created ON clipboard_items(created_at DESC);
CREATE INDEX idx_items_last_used ON clipboard_items(last_used_at DESC);
CREATE INDEX idx_items_type ON clipboard_items(primary_type, created_at DESC);
CREATE INDEX idx_items_pinned ON clipboard_items(is_pinned) WHERE is_pinned = 1;
CREATE INDEX idx_items_favorite ON clipboard_items(is_favorite) WHERE is_favorite = 1;
CREATE INDEX idx_items_hash ON clipboard_items(content_hash);
```

---

## 目录结构

> 以下路径均以新项目根目录 `super_paste_rs/` 为基准。

### Rust 后端
```
src-tauri/src/
├── main.rs
├── lib.rs
├── error.rs
├── result.rs
├── constants.rs
├── app/
│   ├── state.rs
│   ├── setup.rs
│   └── lifecycle.rs
├── commands/
│   ├── mod.rs
│   ├── clipboard.rs
│   ├── tag.rs
│   ├── ocr.rs
│   ├── sync.rs
│   ├── config.rs
│   └── system.rs
├── services/
│   ├── mod.rs
│   ├── clipboard/
│   │   ├── mod.rs
│   │   ├── watcher.rs
│   │   ├── converter.rs
│   │   ├── deduplicator.rs
│   │   └── manager.rs
│   ├── ocr/
│   │   ├── mod.rs
│   │   ├── engine.rs
│   │   ├── processor.rs
│   │   └── model_downloader.rs
│   ├── qrcode/
│   │   └── mod.rs
│   └── sync/
│       ├── mod.rs
│       ├── webdav.rs
│       └── conflict_resolver.rs
├── infrastructure/
│   ├── mod.rs
│   ├── logging.rs
│   ├── active_window.rs
│   ├── event_bus.rs
│   ├── database/
│   │   ├── mod.rs
│   │   ├── pool.rs
│   │   ├── migrations.rs
│   │   └── repositories/
│   │       ├── mod.rs
│   │       ├── clipboard_repo.rs
│   │       ├── tag_repo.rs
│   │       ├── app_repo.rs
│   │       └── config_repo.rs
│   └── config/
│       ├── mod.rs
│       └── manager.rs
├── system/
│   ├── mod.rs
│   ├── tray.rs
│   ├── shortcuts.rs
│   └── overlay.rs
├── types/
│   ├── mod.rs
│   ├── clipboard.rs
│   ├── sync.rs
│   └── events.rs
└── utils/
    ├── mod.rs
    ├── hash.rs
    ├── compression.rs
    ├── time.rs
    └── performance.rs
```

### Vue 前端
```
src/
├── main.ts
├── App.vue
├── router/index.ts
├── api/
│   ├── clipboard.ts
│   ├── tag.ts
│   ├── sync.ts
│   ├── config.ts
│   └── system.ts
├── composables/
│   ├── useClipboardEvents.ts
│   ├── useOverlayWindow.ts
│   ├── useKeyboardNav.ts
│   ├── useShortcutNotification.ts
│   └── useI18n.ts
├── stores/
│   ├── clipboard.ts
│   ├── settings.ts
│   └── sync.ts
├── pages/
│   ├── overlay.vue
│   └── settings/
│       ├── index.vue
│       ├── General.vue
│       ├── Ocr.vue
│       └── Sync.vue
├── components/
│   ├── ClipboardCard.vue
│   ├── SearchBar.vue
│   ├── TagManager.vue
│   └── ...
├── utils/
│   └── error-handler.ts
└── i18n/
    ├── index.ts
    └── locales/
        ├── zh-CN.json
        └── en-US.json
```

---

## 阶段划分与子计划入口

每个阶段的**详细实施步骤、完整代码与验收标准**位于 `docs/implementation/` 目录下的对应文件中。请按顺序阅读和执行：

| 阶段 | 内容 | 详细子计划文件 |
| --- | --- | --- |
| **阶段 0** | 环境准备与基础设施（工具链、错误处理、日志、配置、事件总线、类型、工具函数） | [`implementation/phase-0.md`](./implementation/phase-0.md) → [`implementation/phase-0-part2.md`](./implementation/phase-0-part2.md) |
| **阶段 1** | 数据库层（迁移系统、连接池、各仓库、性能基准与测试） | [`implementation/phase-1.md`](./implementation/phase-1.md) → [`implementation/phase-1-part2.md`](./implementation/phase-1-part2.md) |
| **阶段 2** | 剪贴板核心（监听器、转换器、去重、管理器、托盘、快捷键、Overlay 定位） | [`implementation/phase-2-to-5.md`](./implementation/phase-2-to-5.md)（阶段 2 部分） |
| **阶段 3** | 高级功能（OCR 模型下载、WebDAV 同步、冲突解决） | [`implementation/phase-2-to-5.md`](./implementation/phase-2-to-5.md)（阶段 3 部分） |
| **阶段 4** | UI/UX 优化（前端 API 封装、Pinia Store、交互） | [`implementation/phase-2-to-5.md`](./implementation/phase-2-to-5.md)（阶段 4 部分） |
| **阶段 5** | 打包发布（打包配置、构建命令、三平台产物） | [`implementation/phase-2-to-5.md`](./implementation/phase-2-to-5.md)（阶段 5 部分） |

执行顺序：**阶段 0 → 1 → 2 → 3 → 4 → 5**，每个阶段完成后对照该子计划末尾的「验收标准」勾选确认，再进入下一阶段。

---

## 开始执行

1. 先阅读本文件，确认技术栈、架构分层、数据库 Schema 与目录结构。
2. 按上表进入 [`docs/implementation/phase-0.md`](./implementation/phase-0.md) 开始第一阶段：**在仓库根目录下创建并初始化 `super_paste_rs/` 子目录项目**。
3. 之后依序完成各阶段子计划。
