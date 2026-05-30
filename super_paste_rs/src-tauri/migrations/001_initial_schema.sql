-- 应用信息表
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

-- 剪贴板历史表
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

-- 内容存储表
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

-- OCR 结果表
CREATE TABLE ocr_results (
    content_id TEXT PRIMARY KEY,
    text_content TEXT NOT NULL,
    language TEXT,
    confidence REAL,
    processed_at INTEGER NOT NULL,
    FOREIGN KEY (content_id) REFERENCES clipboard_items(content_id) ON DELETE CASCADE
) STRICT;

-- 标签表
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

-- 标签关联表
CREATE TABLE item_tags (
    content_id TEXT NOT NULL,
    tag_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (content_id, tag_id),
    FOREIGN KEY (content_id) REFERENCES clipboard_items(content_id) ON DELETE CASCADE,
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
) STRICT;

-- 配置表
CREATE TABLE app_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    value_type TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- 使用统计表
CREATE TABLE usage_stats (
    date TEXT PRIMARY KEY,
    total_copies INTEGER DEFAULT 0,
    total_pastes INTEGER DEFAULT 0,
    total_searches INTEGER DEFAULT 0,
    stats_json TEXT,
    created_at INTEGER NOT NULL
) STRICT;
