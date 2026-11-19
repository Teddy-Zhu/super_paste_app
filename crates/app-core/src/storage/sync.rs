use bytes::Bytes;
use rusqlite::{OptionalExtension, params};
use super_paste_format::Digest;
use uuid::Uuid;
use crate::{Error, Result};
use super::Storage;

#[derive(Clone)]
pub(crate) struct SyncBaseline { pub revision: Uuid, pub digest: Digest, pub etag: Option<String> }
#[derive(Default)]
pub(crate) struct SyncTargetState {
    pub conditionals: Option<bool>, pub baseline: Option<SyncBaseline>, pub pending: Option<Bytes>, pub conflict: Option<Digest>,
}
impl Storage {
    pub(crate) async fn sync_target(&self, target: Digest) -> Result<SyncTargetState> {
        self.call(move |connection| {
            let row = connection.query_row("SELECT conditionals,baseline_revision,baseline_digest,baseline_etag,pending_document,conflict_digest FROM sync_targets WHERE target=?1", [target.0.as_slice()],
                |row| Ok((row.get::<_, Option<bool>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<Vec<u8>>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<Vec<u8>>>(4)?, row.get::<_, Option<Vec<u8>>>(5)?))).optional()?;
            let Some((conditionals, revision, digest, etag, pending, conflict)) = row else { return Ok(SyncTargetState::default()); };
            let baseline = match (revision, digest) {
                (Some(revision), Some(digest)) => Some(SyncBaseline { revision: Uuid::parse_str(&revision).map_err(|_| corrupt())?, digest: read_digest(digest)?, etag }),
                (None, None) => None, _ => return Err(corrupt()),
            };
            Ok(SyncTargetState { conditionals, baseline, pending: pending.map(Bytes::from), conflict: conflict.map(read_digest).transpose()? })
        }).await
    }
    pub(crate) async fn sync_conditionals(&self, target: Digest, supported: bool) -> Result<()> {
        self.call(move |connection| {
            connection.execute("INSERT INTO sync_targets(target,conditionals) VALUES(?1,?2) ON CONFLICT(target) DO UPDATE SET conditionals=excluded.conditionals", params![target.0.as_slice(), supported])?; Ok(())
        }).await
    }
    pub(crate) async fn sync_pending(&self, target: Digest, document: Option<Bytes>) -> Result<()> {
        if document.as_ref().is_some_and(|bytes| bytes.len() > 1024 * 1024) { return Err(Error::ResourceLimit("待核实同步文档超过大小上限".into())); }
        self.call(move |connection| {
            connection.execute("INSERT INTO sync_targets(target,pending_document) VALUES(?1,?2) ON CONFLICT(target) DO UPDATE SET pending_document=excluded.pending_document", params![target.0.as_slice(), document.as_deref()])?; Ok(())
        }).await
    }
    pub(crate) async fn sync_establish(&self, target: Digest, baseline: SyncBaseline, timestamp: i64) -> Result<()> {
        self.call(move |connection| {
            connection.execute("INSERT INTO sync_targets(target,baseline_revision,baseline_digest,baseline_etag,last_success) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(target) DO UPDATE SET baseline_revision=excluded.baseline_revision,baseline_digest=excluded.baseline_digest,baseline_etag=excluded.baseline_etag,pending_document=NULL,conflict_digest=NULL,last_success=excluded.last_success",
                params![target.0.as_slice(), baseline.revision.to_string(), baseline.digest.0.as_slice(), baseline.etag, timestamp])?; Ok(())
        }).await
    }
    pub(crate) async fn sync_conflict(&self, target: Digest, conflict: Digest) -> Result<()> {
        self.call(move |connection| {
            connection.execute("INSERT INTO sync_targets(target,conflict_digest) VALUES(?1,?2) ON CONFLICT(target) DO UPDATE SET conflict_digest=excluded.conflict_digest", params![target.0.as_slice(), conflict.0.as_slice()])?; Ok(())
        }).await
    }
    pub(crate) async fn sync_last_success(&self) -> Result<i64> {
        self.call(|connection| Ok(connection.query_row("SELECT coalesce(max(last_success),0) FROM sync_targets", [], |row| row.get(0))?)).await
    }
}
fn read_digest(bytes: Vec<u8>) -> Result<Digest> { Ok(Digest(bytes.try_into().map_err(|_| corrupt())?)) }
fn corrupt() -> Error { Error::Database("同步状态格式无效".into()) }
