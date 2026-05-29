# 阶段 1：数据库层

## 步骤 1.1：实现数据库迁移系统

**创建 `src-tauri/src/infrastructure/database/migrations.rs`**：
```rust
use rusqlite::Connection;
use crate::result::Result;

pub struct MigrationManager;

impl MigrationManager {
    pub fn run_migrations(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            ) STRICT;"
        )?;

        let current_version = Self::get_current_version(conn)?;
        let migrations = Self::get_migrations();

        for (version, sql) in migrations {
            if version > current_version {
                tracing::info!("应用迁移 v{}", version);
                conn.execute_batch(sql)?;
                Self::record_migration(conn, version)?;
            }
        }

        tracing::info!("数据库迁移完成，当前版本: v{}", Self::get_current_version(conn)?);
        Ok(())
    }

    fn get_current_version(conn: &Connection) -> Result<i32> {
        let version: i32 = conn
            .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |row| row.get(0))
            .unwrap_or(0);
        Ok(version)
    }

    fn record_migration(conn: &Connection, version: i32) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![version, now],
        )?;
        Ok(())
    }

    fn get_migrations() -> Vec<(i32, &'static str)> {
        vec![
            (1, include_str!("../../../migrations/001_initial_schema.sql")),
            (2, include_str!("../../../migrations/002_add_indexes.sql")),
            (3, include_str!("../../../migrations/003_add_fts.sql")),
        ]
    }
}
```

**创建 `src-tauri/migrations/001_initial_schema.sql`**：
```sql
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
```

**创建 `src-tauri/migrations/002_add_indexes.sql`**：
```sql
-- 性能优化索引
CREATE INDEX idx_items_created ON clipboard_items(created_at DESC);
CREATE INDEX idx_items_last_used ON clipboard_items(last_used_at DESC);
CREATE INDEX idx_items_type ON clipboard_items(primary_type, created_at DESC);
CREATE INDEX idx_items_pinned ON clipboard_items(is_pinned) WHERE is_pinned = 1;
CREATE INDEX idx_items_favorite ON clipboard_items(is_favorite) WHERE is_favorite = 1;
CREATE INDEX idx_items_hash ON clipboard_items(content_hash);
CREATE INDEX idx_apps_bundle ON apps(bundle_id);
CREATE INDEX idx_item_tags_tag ON item_tags(tag_id);
```

**创建 `src-tauri/migrations/003_add_fts.sql`**：
```sql
-- 全文搜索表
CREATE VIRTUAL TABLE clipboard_fts USING fts5(
    content_id UNINDEXED,
    text_content,
    html_content,
    ocr_text,
    tokenize='trigram'
);

-- 触发器：插入时同步到 FTS
CREATE TRIGGER clipboard_fts_insert AFTER INSERT ON clipboard_content
BEGIN
    INSERT INTO clipboard_fts(content_id, text_content, html_content, ocr_text)
    VALUES (
        NEW.content_id,
        NEW.text_content,
        NEW.html_content,
        (SELECT text_content FROM ocr_results WHERE content_id = NEW.content_id)
    );
END;

-- 触发器：更新时同步到 FTS
CREATE TRIGGER clipboard_fts_update AFTER UPDATE ON clipboard_content
BEGIN
    UPDATE clipboard_fts
    SET text_content = NEW.text_content,
        html_content = NEW.html_content
    WHERE content_id = NEW.content_id;
END;

-- 触发器：删除时同步到 FTS
CREATE TRIGGER clipboard_fts_delete AFTER DELETE ON clipboard_content
BEGIN
    DELETE FROM clipboard_fts WHERE content_id = OLD.content_id;
END;

-- 触发器：OCR 完成时更新 FTS
CREATE TRIGGER ocr_fts_insert AFTER INSERT ON ocr_results
BEGIN
    UPDATE clipboard_fts
    SET ocr_text = NEW.text_content
    WHERE content_id = NEW.content_id;
END;
```

## 步骤 1.2：实现数据库连接池

**创建 `src-tauri/src/infrastructure/database/pool.rs`**：
```rust
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::PathBuf;
use crate::result::Result;

pub type DatabasePool = Pool<SqliteConnectionManager>;

pub fn create_pool(db_path: PathBuf) -> Result<DatabasePool> {
    let manager = SqliteConnectionManager::file(db_path)
        .with_init(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA cache_size = -64000;
                 PRAGMA temp_store = MEMORY;
                 PRAGMA mmap_size = 30000000000;
                 PRAGMA foreign_keys = ON;"
            )
        });

    let pool = Pool::builder()
        .max_size(16)
        .build(manager)?;

    tracing::info!("数据库连接池创建成功");
    Ok(pool)
}
```

**创建 `src-tauri/src/infrastructure/database/mod.rs`**：
```rust
pub mod pool;
pub mod migrations;
pub mod repositories;
```

## 步骤 1.3：实现剪贴板仓库

**创建 `src-tauri/src/infrastructure/database/repositories/clipboard_repo.rs`**：
```rust
use rusqlite::{Connection, params};
use crate::types::clipboard::{ClipboardItem, ClipboardContent, QueryOptions};
use crate::result::Result;

pub struct ClipboardRepository;

impl ClipboardRepository {
    pub fn insert(conn: &Connection, item: &ClipboardItem, content: &ClipboardContent) -> Result<i64> {
        let formats_json = serde_json::to_string(&item.available_formats)?;
        
        conn.execute(
            "INSERT INTO clipboard_items (
                content_id, content_hash, primary_type, data_size, available_formats,
                source_app_id, is_pinned, is_favorite, created_at, last_used_at, use_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                item.content_id, item.content_hash, item.primary_type, item.data_size,
                formats_json, item.source_app_id, item.is_pinned as i32, item.is_favorite as i32,
                item.created_at, item.last_used_at, item.use_count
            ],
        )?;

        let id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO clipboard_content (
                content_id, text_content, html_content, rtf_content,
                image_data, files_json, metadata_json, compression_type
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                content.content_id, content.text_content, content.html_content,
                content.rtf_content, content.image_data, content.files_json,
                content.metadata_json, content.compression_type
            ],
        )?;

        Ok(id)
    }

    pub fn list(conn: &Connection, options: &QueryOptions) -> Result<Vec<ClipboardItem>> {
        let mut sql = String::from(
            "SELECT id, content_id, content_hash, primary_type, data_size, available_formats,
                    source_app_id, is_pinned, is_favorite, created_at, last_used_at, use_count
             FROM clipboard_items WHERE 1=1"
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref t) = options.primary_type {
            sql.push_str(" AND primary_type = ?");
            params.push(Box::new(t.clone()));
        }

        if let Some(ref tag_id) = options.tag_id {
            sql.push_str(" AND content_id IN (SELECT content_id FROM item_tags WHERE tag_id = ?)");
            params.push(Box::new(tag_id.clone()));
        }

        if let Some(pinned) = options.is_pinned {
            sql.push_str(" AND is_pinned = ?");
            params.push(Box::new(pinned as i32));
        }

        if let Some(favorite) = options.is_favorite {
            sql.push_str(" AND is_favorite = ?");
            params.push(Box::new(favorite as i32));
        }

        if let Some(start) = options.start_date {
            sql.push_str(" AND created_at >= ?");
            params.push(Box::new(start));
        }

        if let Some(end) = options.end_date {
            sql.push_str(" AND created_at <= ?");
            params.push(Box::new(end));
        }

        sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
        params.push(Box::new(options.limit));
        params.push(Box::new(options.offset));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        
        let items = stmt.query_map(param_refs.as_slice(), |row| {
            let formats_json: String = row.get(5)?;
            let formats: Vec<String> = serde_json::from_str(&formats_json).unwrap_or_default();

            Ok(ClipboardItem {
                id: Some(row.get(0)?),
                content_id: row.get(1)?,
                content_hash: row.get(2)?,
                primary_type: row.get(3)?,
                data_size: row.get(4)?,
                available_formats: formats,
                source_app_id: row.get(6)?,
                is_pinned: row.get::<_, i32>(7)? != 0,
                is_favorite: row.get::<_, i32>(8)? != 0,
                created_at: row.get(9)?,
                last_used_at: row.get(10)?,
                use_count: row.get(11)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(items)
    }

    pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<(ClipboardItem, ClipboardContent)>> {
        let mut stmt = conn.prepare(
            "SELECT i.id, i.content_id, i.content_hash, i.primary_type, i.data_size,
                    i.available_formats, i.source_app_id, i.is_pinned, i.is_favorite,
                    i.created_at, i.last_used_at, i.use_count,
                    c.text_content, c.html_content, c.rtf_content, c.image_data,
                    c.files_json, c.metadata_json, c.compression_type
             FROM clipboard_items i
             LEFT JOIN clipboard_content c ON i.content_id = c.content_id
             WHERE i.id = ?"
        )?;

        let result = stmt.query_row([id], |row| {
            let formats_json: String = row.get(5)?;
            let formats: Vec<String> = serde_json::from_str(&formats_json).unwrap_or_default();

            let item = ClipboardItem {
                id: Some(row.get(0)?),
                content_id: row.get(1)?,
                content_hash: row.get(2)?,
                primary_type: row.get(3)?,
                data_size: row.get(4)?,
                available_formats: formats,
                source_app_id: row.get(6)?,
                is_pinned: row.get::<_, i32>(7)? != 0,
                is_favorite: row.get::<_, i32>(8)? != 0,
                created_at: row.get(9)?,
                last_used_at: row.get(10)?,
                use_count: row.get(11)?,
            };

            let content = ClipboardContent {
                content_id: row.get(1)?,
                text_content: row.get(12)?,
                html_content: row.get(13)?,
                rtf_content: row.get(14)?,
                image_data: row.get(15)?,
                files_json: row.get(16)?,
                metadata_json: row.get(17)?,
                compression_type: row.get(18)?,
            };

            Ok((item, content))
        });

        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn search(conn: &Connection, query: &str, limit: i64) -> Result<Vec<ClipboardItem>> {
        let mut stmt = conn.prepare(
            "SELECT i.id, i.content_id, i.content_hash, i.primary_type, i.data_size,
                    i.available_formats, i.source_app_id, i.is_pinned, i.is_favorite,
                    i.created_at, i.last_used_at, i.use_count
             FROM clipboard_items i
             INNER JOIN clipboard_fts f ON i.content_id = f.content_id
             WHERE clipboard_fts MATCH ?
             ORDER BY rank
             LIMIT ?"
        )?;

        let items = stmt.query_map(params![query, limit], |row| {
            let formats_json: String = row.get(5)?;
            let formats: Vec<String> = serde_json::from_str(&formats_json).unwrap_or_default();

            Ok(ClipboardItem {
                id: Some(row.get(0)?),
                content_id: row.get(1)?,
                content_hash: row.get(2)?,
                primary_type: row.get(3)?,
                data_size: row.get(4)?,
                available_formats: formats,
                source_app_id: row.get(6)?,
                is_pinned: row.get::<_, i32>(7)? != 0,
                is_favorite: row.get::<_, i32>(8)? != 0,
                created_at: row.get(9)?,
                last_used_at: row.get(10)?,
                use_count: row.get(11)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(items)
    }

    pub fn update_last_used(conn: &Connection, id: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE clipboard_items SET last_used_at = ?, use_count = use_count + 1 WHERE id = ?",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn update_flags(conn: &Connection, id: i64, is_pinned: Option<bool>, is_favorite: Option<bool>) -> Result<()> {
        if let Some(pinned) = is_pinned {
            conn.execute(
                "UPDATE clipboard_items SET is_pinned = ? WHERE id = ?",
                params![pinned as i32, id],
            )?;
        }

        if let Some(favorite) = is_favorite {
            conn.execute(
                "UPDATE clipboard_items SET is_favorite = ? WHERE id = ?",
                params![favorite as i32, id],
            )?;
        }

        Ok(())
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM clipboard_items WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn delete_old(conn: &Connection, before_timestamp: i64) -> Result<usize> {
        let count = conn.execute(
            "DELETE FROM clipboard_items WHERE created_at < ? AND is_pinned = 0 AND is_favorite = 0",
            [before_timestamp],
        )?;
        Ok(count)
    }

    pub fn get_by_hash(conn: &Connection, hash: &str) -> Result<Option<i64>> {
        let result = conn.query_row(
            "SELECT id FROM clipboard_items WHERE content_hash = ? ORDER BY created_at DESC LIMIT 1",
            [hash],
            |row| row.get(0),
        );

        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
```

继续下一部分...
