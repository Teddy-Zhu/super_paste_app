use std::path::Path;
use serde::Serialize;
use serde_json::value::RawValue;
use super_paste_format::{Digest, SyncOutcome};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use crate::{Result, configuration::ConfigurationEffects, storage::SyncTargetState};
use super::{SyncService, document::{self, Document, Envelope, Payload}, outcome, webdav::Target};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Conflict<'a> {
    format_version: u32, created_at: i64, target: String, reason: &'a str,
    local: &'a RawValue, remote: Option<Envelope<'a>>, baseline: Option<Baseline>, pending: Option<&'a RawValue>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Baseline { revision: Uuid, payload_sha256: String, etag: Option<String> }
impl<E: ConfigurationEffects + 'static> SyncService<E> {
    pub(super) async fn conflict(&self, target: &Target, local: &Payload, remote: Option<&Document>, state: &SyncTargetState, message: &str) -> Result<SyncOutcome> {
        let fingerprint = archive(&self.conflicts, target.key, local, remote, state, message).await?;
        let repeated = state.conflict == Some(fingerprint);
        if !repeated { self.storage.sync_conflict(target.key, fingerprint).await?; }
        let mut result = outcome(message); result.conflict = Some(message.into()); result.repeated_conflict = repeated; Ok(result)
    }
}

/// Preserve the snapshot independently of the user-facing conflict status.
/// Reconciliation also needs this before replacing an unconfirmed upload.
pub(super) async fn archive(directory: &Path, target: Digest, local: &Payload, remote: Option<&Document>, state: &SyncTargetState, message: &str) -> Result<Digest> {
    let baseline = state.baseline.as_ref().map(|base| Baseline { revision: base.revision, payload_sha256: document::hex(base.digest), etag: base.etag.clone() });
    let mut signature = format!("{}\0{}\0{}\0{}\0{}\0{}", document::hex(target), document::hex(local.hash),
        remote.map(|document| document.metadata.revision.to_string()).unwrap_or_default(), remote.map(|document| document::hex(document.payload.hash)).unwrap_or_default(),
        baseline.as_ref().map(|base| base.revision.to_string()).unwrap_or_default(), baseline.as_ref().map(|base| base.payload_sha256.as_str()).unwrap_or_default());
    if let Some(pending) = state.pending.as_deref() {
        signature.push('\0');
        signature.push_str(&document::hex(document::digest(pending)));
    }
    let fingerprint = document::digest(signature.as_bytes());
    let repeated = state.conflict == Some(fingerprint);
    if !repeated {
        tokio::fs::create_dir_all(directory).await?;
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; tokio::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).await?; }
        let path = directory.join(format!("{}.json", document::hex(fingerprint)));
        if !tokio::fs::try_exists(&path).await? {
            let pending = state.pending.as_deref().map(serde_json::from_slice::<&RawValue>).transpose()?;
            let audit = Conflict { format_version: 1, created_at: crate::now_ms()?, target: document::hex(target), reason: message,
                local: &local.raw, remote: remote.map(Document::envelope), baseline, pending };
            let bytes = serde_json::to_vec_pretty(&audit)?;
            let temporary = directory.join(format!(".{}.tmp", Uuid::new_v4()));
            let result: Result<()> = async {
                let mut options = tokio::fs::OpenOptions::new(); options.write(true).create_new(true);
                #[cfg(unix)] options.mode(0o600);
                let mut file = options.open(&temporary).await?;
                file.write_all(&bytes).await?; file.sync_all().await?; drop(file);
                tokio::fs::rename(&temporary, &path).await?; Ok(())
            }.await;
            if let Err(error) = result {
                if let Err(cleanup) = tokio::fs::remove_file(&temporary).await { if cleanup.kind() != std::io::ErrorKind::NotFound { tracing::warn!(%cleanup, "同步冲突临时文件清理失败"); } }
                return Err(error);
            }
        }
    }
    Ok(fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::document::Metadata;
    use super_paste_format::{Configuration, SyncProjection};

    #[tokio::test]
    async fn changed_pending_uploads_preserve_distinct_audits() {
        let directory = std::env::temp_dir().join(format!("super-paste-rs-sync-conflict-{}", Uuid::new_v4()));
        let target = document::digest(b"synthetic-sync-target");
        let mut configuration = Configuration::default();
        let local = Payload::new(SyncProjection::capture(&configuration)).expect("local projection");
        let mut state = SyncTargetState::default();
        let original = archive(&directory, target, &local, None, &state, "initial conflict").await.expect("initial audit");
        state.conflict = Some(original);

        // Local and remote stay unchanged while a different, unconfirmed
        // upload appears. It must not be mistaken for the previous backup.
        configuration.settings.max_history_size += 1;
        let upload = Payload::new(SyncProjection::capture(&configuration)).expect("pending projection");
        let first = Document::encode(Metadata::new(None).unwrap(), &upload).expect("first pending upload");
        state.pending = Some(first.clone());
        let with_first = archive(&directory, target, &local, None, &state, "preserve first pending").await.expect("pending audit");
        assert_ne!(original, with_first);
        let path = directory.join(format!("{}.json", document::hex(with_first)));
        let first_audit = tokio::fs::read(&path).await.unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&first_audit).unwrap();
        let expected: serde_json::Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(saved["pending"], expected);
        assert_eq!(saved["local"], serde_json::from_str::<serde_json::Value>(local.raw.get()).unwrap());

        state.conflict = Some(with_first);
        assert_eq!(archive(&directory, target, &local, None, &state, "repeat").await.unwrap(), with_first);
        // Equal payloads with a new upload revision are separate recovery
        // records; retain their metadata as well as their settings.
        let second = Document::encode(Metadata::new(None).unwrap(), &upload).expect("second pending upload");
        state.pending = Some(second.clone());
        let with_second = archive(&directory, target, &local, None, &state, "preserve second pending").await.unwrap();
        assert_ne!(with_first, with_second);
        let saved_second: serde_json::Value = serde_json::from_slice(&tokio::fs::read(directory.join(format!("{}.json", document::hex(with_second)))).await.unwrap()).unwrap();
        assert_eq!(saved_second["pending"], serde_json::from_slice::<serde_json::Value>(&second).unwrap());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), first_audit, "earlier audit was preserved");
        let original_saved: serde_json::Value = serde_json::from_slice(&tokio::fs::read(directory.join(format!("{}.json", document::hex(original)))).await.unwrap()).unwrap();
        assert!(original_saved["pending"].is_null());
        let mut entries = tokio::fs::read_dir(&directory).await.unwrap();
        let mut count = 0;
        while entries.next_entry().await.unwrap().is_some() { count += 1; }
        assert_eq!(count, 3, "duplicates and temporary files were not retained");
        tokio::fs::remove_dir_all(directory).await.expect("remove synthetic audits");
    }
}
