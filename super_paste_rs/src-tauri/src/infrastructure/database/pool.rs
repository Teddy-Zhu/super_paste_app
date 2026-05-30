use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::PathBuf;

use crate::result::Result;

pub type DatabasePool = Pool<SqliteConnectionManager>;

pub fn create_pool(db_path: PathBuf) -> Result<DatabasePool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -64000;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 30000000000;
             PRAGMA foreign_keys = ON;",
        )
    });

    let pool = Pool::builder().max_size(16).build(manager)?;

    tracing::info!("数据库连接池创建成功");
    Ok(pool)
}
