PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS config_field_versions (
    key TEXT PRIMARY KEY NOT NULL,
    version INTEGER NOT NULL CHECK(version >= 0)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS apps (
    id TEXT PRIMARY KEY NOT NULL,
    application TEXT NOT NULL,
    path TEXT NOT NULL,
    icon TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(application, path)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS clipboard_history (
    content_id TEXT PRIMARY KEY NOT NULL,
    digest TEXT NOT NULL UNIQUE,
    content_text TEXT,
    content_html TEXT,
    content_image BLOB,
    content_rtf TEXT,
    content_files_json TEXT,
    content_ocr TEXT,
    primary_type TEXT NOT NULL,
    available_formats_json TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    last_used INTEGER NOT NULL,
    source_app TEXT REFERENCES apps(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) WITHOUT ROWID;

-- The record id makes cursor pagination deterministic when timestamps are equal.
CREATE INDEX IF NOT EXISTS clipboard_history_timestamp_cursor ON clipboard_history(timestamp DESC, content_id DESC);
CREATE INDEX IF NOT EXISTS clipboard_history_last_used_cursor ON clipboard_history(last_used DESC, content_id DESC);
DROP INDEX IF EXISTS clipboard_history_timestamp;
DROP INDEX IF EXISTS clipboard_history_last_used;
CREATE INDEX IF NOT EXISTS clipboard_history_type ON clipboard_history(primary_type);
CREATE INDEX IF NOT EXISTS clipboard_history_source ON clipboard_history(source_app);
CREATE INDEX IF NOT EXISTS clipboard_history_ocr ON clipboard_history(content_ocr);

CREATE TABLE IF NOT EXISTS tag_groups (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    color TEXT NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS clipboard_history_tags (
    content_id TEXT NOT NULL REFERENCES clipboard_history(content_id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tag_groups(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(content_id, tag_id)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS clipboard_history_tags_tag ON clipboard_history_tags(tag_id);

CREATE TABLE IF NOT EXISTS copy_statistics (
    hour INTEGER NOT NULL,
    type TEXT NOT NULL,
    source TEXT NOT NULL,
    copies INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    files INTEGER NOT NULL,
    first_copied_at INTEGER NOT NULL,
    last_copied_at INTEGER NOT NULL,
    PRIMARY KEY(hour, type, source)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS copy_statistics_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS ai_ocr_models (
    id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL,
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    UNIQUE(provider, base_url, model)
);

CREATE TABLE IF NOT EXISTS ai_ocr_usage_hourly (
    hour INTEGER NOT NULL,
    model_id INTEGER NOT NULL REFERENCES ai_ocr_models(id) ON DELETE CASCADE,
    requests INTEGER NOT NULL,
    failures INTEGER NOT NULL,
    unknown_usage INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cached_input_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    total_tokens INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    first_used_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    PRIMARY KEY(hour, model_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS ai_ocr_usage_recent (
    id TEXT PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    purpose TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    success INTEGER NOT NULL,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cached_input_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    total_tokens INTEGER
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS ai_ocr_usage_recent_started ON ai_ocr_usage_recent(started_at DESC);

CREATE TABLE IF NOT EXISTS ai_ocr_usage_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS migration_markers (
    id TEXT PRIMARY KEY NOT NULL,
    completed_at INTEGER NOT NULL,
    source_fingerprint TEXT NOT NULL,
    report_json TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS app_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL
) WITHOUT ROWID;
