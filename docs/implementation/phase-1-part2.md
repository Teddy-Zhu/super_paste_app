## 步骤 1.4：实现其他仓库

**创建 `src-tauri/src/infrastructure/database/repositories/tag_repo.rs`**：
```rust
use rusqlite::{Connection, params};
use crate::types::clipboard::Tag;
use crate::result::Result;

pub struct TagRepository;

impl TagRepository {
    pub fn insert(conn: &Connection, tag: &Tag) -> Result<()> {
        conn.execute(
            "INSERT INTO tags (id, name, color, icon, description, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                tag.id, tag.name, tag.color, tag.icon, tag.description,
                tag.sort_order, tag.created_at, tag.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn list(conn: &Connection) -> Result<Vec<Tag>> {
        let mut stmt = conn.prepare(
            "SELECT id, name, color, icon, description, sort_order, created_at, updated_at
             FROM tags ORDER BY sort_order, name"
        )?;

        let tags = stmt.query_map([], |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
                description: row.get(4)?,
                sort_order: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tags)
    }

    pub fn update(conn: &Connection, tag: &Tag) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE tags SET name = ?, color = ?, icon = ?, description = ?, sort_order = ?, updated_at = ?
             WHERE id = ?",
            params![tag.name, tag.color, tag.icon, tag.description, tag.sort_order, now, tag.id],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, id: &str) -> Result<()> {
        conn.execute("DELETE FROM tags WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn add_to_item(conn: &Connection, content_id: &str, tag_id: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO item_tags (content_id, tag_id, created_at) VALUES (?1, ?2, ?3)",
            params![content_id, tag_id, now],
        )?;
        Ok(())
    }

    pub fn remove_from_item(conn: &Connection, content_id: &str, tag_id: &str) -> Result<()> {
        conn.execute(
            "DELETE FROM item_tags WHERE content_id = ? AND tag_id = ?",
            params![content_id, tag_id],
        )?;
        Ok(())
    }

    pub fn get_item_tags(conn: &Connection, content_id: &str) -> Result<Vec<Tag>> {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.name, t.color, t.icon, t.description, t.sort_order, t.created_at, t.updated_at
             FROM tags t
             INNER JOIN item_tags it ON t.id = it.tag_id
             WHERE it.content_id = ?
             ORDER BY t.sort_order, t.name"
        )?;

        let tags = stmt.query_map([content_id], |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
                description: row.get(4)?,
                sort_order: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tags)
    }
}
```

**创建 `src-tauri/src/infrastructure/database/repositories/app_repo.rs`**：
```rust
use rusqlite::{Connection, params};
use crate::types::clipboard::AppInfo;
use crate::result::Result;

pub struct AppRepository;

impl AppRepository {
    pub fn upsert(conn: &Connection, app: &AppInfo) -> Result<()> {
        conn.execute(
            "INSERT INTO apps (id, name, bundle_id, executable_path, icon_hash, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                name = ?2,
                last_seen_at = ?7",
            params![
                app.id, app.name, app.bundle_id, app.executable_path,
                app.icon_hash, app.first_seen_at, app.last_seen_at
            ],
        )?;
        Ok(())
    }

    pub fn get_by_id(conn: &Connection, id: &str) -> Result<Option<AppInfo>> {
        let result = conn.query_row(
            "SELECT id, name, bundle_id, executable_path, icon_hash, first_seen_at, last_seen_at
             FROM apps WHERE id = ?",
            [id],
            |row| {
                Ok(AppInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    bundle_id: row.get(2)?,
                    executable_path: row.get(3)?,
                    icon_hash: row.get(4)?,
                    first_seen_at: row.get(5)?,
                    last_seen_at: row.get(6)?,
                })
            },
        );

        match result {
            Ok(app) => Ok(Some(app)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list(conn: &Connection) -> Result<Vec<AppInfo>> {
        let mut stmt = conn.prepare(
            "SELECT id, name, bundle_id, executable_path, icon_hash, first_seen_at, last_seen_at
             FROM apps ORDER BY last_seen_at DESC"
        )?;

        let apps = stmt.query_map([], |row| {
            Ok(AppInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                bundle_id: row.get(2)?,
                executable_path: row.get(3)?,
                icon_hash: row.get(4)?,
                first_seen_at: row.get(5)?,
                last_seen_at: row.get(6)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(apps)
    }
}
```

**创建 `src-tauri/src/infrastructure/database/repositories/config_repo.rs`**：
```rust
use rusqlite::{Connection, params};
use crate::result::Result;

pub struct ConfigRepository;

impl ConfigRepository {
    pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
        let result = conn.query_row(
            "SELECT value FROM app_config WHERE key = ?",
            [key],
            |row| row.get(0),
        );

        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set(conn: &Connection, key: &str, value: &str, value_type: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO app_config (key, value, value_type, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value = ?2, value_type = ?3, updated_at = ?4",
            params![key, value, value_type, now],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, key: &str) -> Result<()> {
        conn.execute("DELETE FROM app_config WHERE key = ?", [key])?;
        Ok(())
    }

    pub fn get_all(conn: &Connection) -> Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare("SELECT key, value FROM app_config")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
```

**创建 `src-tauri/src/infrastructure/database/repositories/mod.rs`**：
```rust
pub mod clipboard_repo;
pub mod tag_repo;
pub mod app_repo;
pub mod config_repo;

pub use clipboard_repo::ClipboardRepository;
pub use tag_repo::TagRepository;
pub use app_repo::AppRepository;
pub use config_repo::ConfigRepository;
```

## 步骤 1.5：实现性能基准测试

**创建 `src-tauri/benches/database_bench.rs`**：
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use rusqlite::Connection;
use superpaste_lib::infrastructure::database::{
    migrations::MigrationManager,
    repositories::{ClipboardRepository, QueryOptions},
};
use superpaste_lib::types::clipboard::{ClipboardItem, ClipboardContent};

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    MigrationManager::run_migrations(&conn).unwrap();
    conn
}

fn create_test_item(id: usize) -> (ClipboardItem, ClipboardContent) {
    let content_id = format!("test-{}", id);
    let now = chrono::Utc::now().timestamp();
    
    let item = ClipboardItem {
        id: None,
        content_id: content_id.clone(),
        content_hash: format!("hash-{}", id),
        primary_type: "text".to_string(),
        data_size: 100,
        available_formats: vec!["text".to_string()],
        source_app_id: None,
        is_pinned: false,
        is_favorite: false,
        created_at: now,
        last_used_at: now,
        use_count: 1,
    };
    
    let content = ClipboardContent {
        content_id,
        text_content: Some(format!("测试内容 {} - 这是一段用于性能测试的文本", id)),
        html_content: None,
        rtf_content: None,
        image_data: None,
        files_json: None,
        metadata_json: None,
        compression_type: None,
    };
    
    (item, content)
}

fn benchmark_insert(c: &mut Criterion) {
    c.bench_function("insert_single", |b| {
        b.iter_batched(
            || {
                let conn = setup_test_db();
                let (item, content) = create_test_item(1);
                (conn, item, content)
            },
            |(conn, item, content)| {
                ClipboardRepository::insert(&conn, &item, &content).unwrap()
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn benchmark_query(c: &mut Criterion) {
    let conn = setup_test_db();
    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }
    
    c.bench_function("list_50_items", |b| {
        b.iter(|| {
            let options = QueryOptions {
                offset: 0,
                limit: 50,
                ..Default::default()
            };
            ClipboardRepository::list(&conn, &options).unwrap()
        });
    });
}

fn benchmark_search(c: &mut Criterion) {
    let conn = setup_test_db();
    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }
    
    c.bench_function("search_1000_items", |b| {
        b.iter(|| {
            ClipboardRepository::search(&conn, black_box("测试"), 50).unwrap()
        });
    });
}

criterion_group!(benches, benchmark_insert, benchmark_query, benchmark_search);
criterion_main!(benches);
```

**配置 `src-tauri/Cargo.toml`**：
```toml
[[bench]]
name = "database_bench"
harness = false
```

**运行基准测试**：
```bash
cd src-tauri
cargo bench
```

## 步骤 1.6：实现性能测试

**创建 `src-tauri/tests/performance_test.rs`**：
```rust
use rusqlite::Connection;
use superpaste_lib::infrastructure::database::{
    migrations::MigrationManager,
    repositories::{ClipboardRepository, QueryOptions},
};
use superpaste_lib::types::clipboard::{ClipboardItem, ClipboardContent};
use std::time::Instant;

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    MigrationManager::run_migrations(&conn).unwrap();
    conn
}

fn create_test_item(id: usize) -> (ClipboardItem, ClipboardContent) {
    let content_id = format!("test-{}", id);
    let now = chrono::Utc::now().timestamp();
    
    let item = ClipboardItem {
        id: None,
        content_id: content_id.clone(),
        content_hash: format!("hash-{}", id),
        primary_type: "text".to_string(),
        data_size: 100,
        available_formats: vec!["text".to_string()],
        source_app_id: None,
        is_pinned: false,
        is_favorite: false,
        created_at: now,
        last_used_at: now,
        use_count: 1,
    };
    
    let content = ClipboardContent {
        content_id,
        text_content: Some(format!("测试内容 {}", id)),
        html_content: None,
        rtf_content: None,
        image_data: None,
        files_json: None,
        metadata_json: None,
        compression_type: None,
    };
    
    (item, content)
}

#[test]
fn test_search_performance() {
    let conn = setup_test_db();
    
    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }
    
    let start = Instant::now();
    let results = ClipboardRepository::search(&conn, "测试", 50).unwrap();
    let duration = start.elapsed();
    
    println!("搜索 1000 条记录耗时: {:?}", duration);
    assert!(duration.as_millis() < 100, "搜索性能未达标: {:?}", duration);
    assert!(!results.is_empty(), "搜索结果为空");
}

#[test]
fn test_insert_performance() {
    let conn = setup_test_db();
    let (item, content) = create_test_item(1);
    
    let start = Instant::now();
    ClipboardRepository::insert(&conn, &item, &content).unwrap();
    let duration = start.elapsed();
    
    println!("插入单条记录耗时: {:?}", duration);
    assert!(duration.as_millis() < 10, "插入性能未达标: {:?}", duration);
}

#[test]
fn test_query_performance() {
    let conn = setup_test_db();
    
    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }
    
    let options = QueryOptions {
        offset: 0,
        limit: 50,
        ..Default::default()
    };
    
    let start = Instant::now();
    let results = ClipboardRepository::list(&conn, &options).unwrap();
    let duration = start.elapsed();
    
    println!("分页查询 50 条耗时: {:?}", duration);
    assert!(duration.as_millis() < 50, "查询性能未达标: {:?}", duration);
    assert_eq!(results.len(), 50, "查询结果数量不正确");
}
```

**运行性能测试**：
```bash
cd src-tauri
cargo test --test performance_test -- --nocapture
```

## 验收标准

- [ ] 数据库迁移系统正常工作
- [ ] 数据库连接池创建成功
- [ ] 剪贴板仓库 CRUD 操作正常
- [ ] 标签仓库操作正常
- [ ] 应用仓库操作正常
- [ ] 配置仓库操作正常
- [ ] 全文搜索功能正常
- [ ] 基准测试可运行
- [ ] 性能测试通过：
  - 搜索 1000 条记录 < 100ms
  - 插入单条记录 < 10ms
  - 分页查询 50 条 < 50ms
