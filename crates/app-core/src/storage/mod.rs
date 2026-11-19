use std::{path::Path, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use rusqlite::{OptionalExtension, TransactionBehavior};
use super_paste_format::{Configuration, Digest, INITIAL_SCHEMA, SCHEMA_VERSION, SQLITE_APPLICATION_ID};
use crate::{Error, Result, resources::Reservation};
use tokio_util::sync::CancellationToken;

mod history;
mod tags;
mod statistics;
mod usage;
mod sync;
mod preview;
mod migrations;
#[cfg(test)]
mod tests;
pub use history::{PreparedRecord, UpsertResult};
pub use tags::TagChanges;
pub(crate) use sync::{SyncBaseline, SyncTargetState};

/// One ordered SQLite worker; decoding and network activity never run on its thread.
pub struct Storage { connection: tokio_rusqlite::Connection }
pub struct StorageCounts { pub records: u64, pub tags: u64, pub images: u64, pub encoded_image_bytes: u64 }
impl Storage {
    #[cfg(test)]
    pub(crate) async fn close_for_test(&self) -> Result<()> {
        self.connection.clone().close().await.map_err(|error| Error::Database(error.to_string().into()))
    }

    /// Aggregate metadata only; diagnostic export never fetches stored payloads.
    pub async fn diagnostic_counts(&self) -> Result<StorageCounts> {
        self.call(|connection| Ok(connection.query_row("SELECT (SELECT COUNT(*) FROM history), (SELECT COUNT(*) FROM tags), (SELECT COUNT(*) FROM images), (SELECT COALESCE(SUM(encoded_bytes),0) FROM images)", [], |row| {
            Ok(StorageCounts { records: row.get(0)?, tags: row.get(1)?, images: row.get(2)?, encoded_image_bytes: row.get(3)? })
        })?)).await
    }
    pub async fn open(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        let connection = tokio_rusqlite::Connection::open(path.as_ref().to_path_buf()).await.map_err(|e| Error::Database(e.to_string().into()))?;
        let storage = Arc::new(Self { connection });
        storage.call(|connection| {
            connection.busy_timeout(Duration::from_secs(5))?;
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let application_id: i32 = transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
            let version: i32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            let tables: i64 = transaction.query_row("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |row| row.get(0))?;
            if application_id == 0 && version == 0 && tables == 0 {
                transaction.execute_batch(INITIAL_SCHEMA)?;
                transaction.pragma_update(None, "application_id", SQLITE_APPLICATION_ID)?;
                transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                transaction.execute("INSERT INTO app_meta(key,value) VALUES('format','super-paste-rs'),('revision','0'),('digest-version','1')", [])?;
                let defaults = serde_json::to_string(&Configuration::default())?;
                transaction.execute("INSERT INTO settings(key,value_json,revision) VALUES('configuration',?1,0)", [defaults])?;
            } else {
                if application_id != SQLITE_APPLICATION_ID || !(1..=SCHEMA_VERSION).contains(&version) {
                    return Err(Error::Database(format!("不支持的数据库标识或版本（{application_id:#x}/{version}）").into()));
                }
                let format: String = transaction.query_row("SELECT value FROM app_meta WHERE key='format'", [], |row| row.get(0))?;
                if format != "super-paste-rs" { return Err(Error::Database("数据库格式身份不匹配".into())); }
                let digest: String = transaction.query_row("SELECT value FROM app_meta WHERE key='digest-version'", [], |row| row.get(0))?;
                if digest != "1" { return Err(Error::Database("不支持的内容摘要版本".into())); }
                if version == 1 {
                    migrations::ocr_presence(&transaction)?;
                    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                }
            }
            transaction.commit()?;
            // Do not change journaling or schema before validating the database identity.
            connection.pragma_update(None, "foreign_keys", true)?;
            let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") { return Err(Error::Database("数据库无法启用 WAL".into())); }
            connection.pragma_update(None, "synchronous", "NORMAL")?;
            Ok(())
        }).await?;
        Ok(storage)
    }

    pub(crate) async fn call<F, R>(&self, function: F) -> Result<R>
    where F: FnOnce(&mut rusqlite::Connection) -> Result<R> + Send + 'static, R: Send + 'static {
        self.connection.call(function).await.map_err(|error| match error {
            tokio_rusqlite::Error::Error(error) => error,
            other => Error::Database(other.to_string().into()),
        })
    }

    /// Cancellation belongs to this read on the ordered worker, never to the
    /// connection's next command. Queued obsolete reads skip SQL altogether.
    /// Only read-only callbacks belong here: accepted mutations use `call` and
    /// retain their actual outcome even if their original consumer has left.
    pub(crate) async fn call_read<F, R>(&self, cancel: &CancellationToken, function: F) -> Result<R>
    where F: FnOnce(&rusqlite::Connection) -> Result<R> + Send + 'static, R: Send + 'static {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let cancel = cancel.clone();
        self.call(move |connection| {
            if cancel.is_cancelled() { return Err(Error::Cancelled); }
            let interrupted = Arc::new(AtomicBool::new(false));
            let observed = interrupted.clone(); let requested = cancel.clone();
            connection.progress_handler(1000, Some(move || {
                if !requested.is_cancelled() { return false; }
                observed.store(true, Ordering::Relaxed);
                true
            }))?;
            let mut progress = ReadProgress(Some(connection));
            let result = function(connection);
            // Clear before returning even on a SQL error. The drop guard also
            // handles unwinding; no interrupt handle can race a later write.
            progress.clear()?;
            if interrupted.load(Ordering::Relaxed) || result.is_ok() && cancel.is_cancelled() {
                Err(Error::Cancelled)
            } else { result }
        }).await
    }

    pub(crate) async fn read_image(&self, id: Digest, expected_size: u64, reservation: Reservation, cancel: &CancellationToken) -> Result<(Vec<u8>, Reservation)> {
        let expected_size = i64::try_from(expected_size).map_err(|_| Error::Invalid("图片长度无效".into()))?;
        self.call_read(cancel, move |connection| {
            // The worker may finish a BLOB read after its async observer is
            // dropped. Keep admission with the physical allocation throughout.
            let data = connection.query_row("SELECT data FROM images WHERE id=?1 AND encoded_bytes=?2", rusqlite::params![id.0.as_slice(), expected_size], |row| row.get(0))
                .optional()?.ok_or(Error::NotFound)?;
            Ok((data, reservation))
        }).await
    }

    pub async fn configuration(&self) -> Result<super_paste_format::ConfigSnapshot> {
        self.call(|connection| {
            let (json, revision): (String, u64) = connection.query_row("SELECT value_json,revision FROM settings WHERE key='configuration'", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let configuration: Configuration = serde_json::from_str(&json)?;
            configuration.validate()?;
            Ok(super_paste_format::ConfigSnapshot { value: Arc::new(configuration), revision })
        }).await
    }

    /// Called after producers have stopped; checkpoint failure is propagated to the host.
    pub async fn checkpoint(&self) -> Result<()> {
        self.call(|connection| {
            let (busy, _, _): (i32, i32, i32) = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            if busy != 0 { return Err(Error::Database("数据库仍被占用，无法完成检查点".into())); }
            Ok(())
        }).await
    }
}

struct ReadProgress<'a>(Option<&'a rusqlite::Connection>);
impl ReadProgress<'_> {
    fn clear(&mut self) -> rusqlite::Result<()> {
        if let Some(connection) = self.0 {
            connection.progress_handler(0, None::<fn() -> bool>)?;
            self.0 = None;
        }
        Ok(())
    }
}
impl Drop for ReadProgress<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.clear() { tracing::error!(%error, "清除 SQLite 读取取消回调失败"); }
    }
}

pub(crate) fn revision(transaction: &rusqlite::Transaction<'_>) -> Result<i64> {
    transaction.query_row("UPDATE app_meta SET value=CAST(value AS INTEGER)+1 WHERE key='revision' AND CAST(value AS INTEGER)<9223372036854775807 RETURNING CAST(value AS INTEGER)", [], |row| row.get(0))
        .optional()?.ok_or_else(|| Error::Database("数据库修订号缺失或耗尽".into()))
}
fn collect_orphan_images(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction.execute("DELETE FROM images WHERE NOT EXISTS(SELECT 1 FROM history WHERE history.image_id=images.id)", [])?;
    Ok(())
}
