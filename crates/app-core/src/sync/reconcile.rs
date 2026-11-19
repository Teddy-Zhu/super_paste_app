use reqwest::{Method, StatusCode, header::{HeaderMap, HeaderValue, CONTENT_TYPE, IF_MATCH, IF_NONE_MATCH}};
use super_paste_format::{SyncDirection, SyncOutcome, SyncProjection};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use crate::{Error, Result, configuration::{ConfigurationEffects, ConfigurationMutation, ConfigurationState}, storage::{SyncBaseline, SyncTargetState}};
use super::{RunKind, SyncService, outcome, scope, version, document::{Document, Metadata, Payload}, webdav::Target};

impl<E: ConfigurationEffects + 'static> SyncService<E> {
    pub(super) async fn reconcile(&self, target: &Target, kind: RunKind, expected: &ConfigurationState, direction: SyncDirection, cancel: &CancellationToken) -> Result<SyncOutcome> {
        let mut state = self.storage.sync_target(target.key).await?;
        let require_conditionals = if kind == RunKind::Scheduled { direction != SyncDirection::Download } else { direction == SyncDirection::Bidirectional };
        if require_conditionals {
            match state.conditionals {
                Some(true) => {},
                Some(false) => return Err(Error::Invalid("WebDAV 服务未通过条件写入检查，请手动测试连接后重试自动同步".into())),
                None => {
                    let test = target.test_conditionals(cancel).await;
                    // Authentication/network/cancellation failures do not
                    // prove that the server violates conditional writes.
                    if let Some(supported) = test.supported { self.storage.sync_conditionals(target.key, supported).await?; }
                    test.result?;
                }
            }
        }
        target.ensure_collection(cancel).await?;
        let remote = target.read(cancel).await?;
        self.ensure_current(expected, cancel)?;
        let local = Payload::new(SyncProjection::capture(&expected.snapshot.value))?;
        let mut pending = state.pending.as_ref().map(|bytes| Document::decode(bytes, None)).transpose()?;
        match (pending.as_ref(), remote.as_ref()) {
            (Some(waiting), Some(actual)) if waiting.metadata.revision == actual.metadata.revision && waiting.payload.hash == actual.payload.hash => {
                // An earlier PUT may have succeeded despite a lost response.
                // Recover its baseline before comparing subsequent local edits.
                self.establish(target, actual).await?;
                state.baseline = Some(baseline(actual)); state.pending = None; state.conflict = None;
                pending = None;
            },
            (Some(_), _) => {
                // Every path below may establish a new baseline or replace the
                // pending PUT. Preserve the uncertain document first; archive
                // failure leaves it authoritative and aborts the new operation.
                super::conflict::archive(&self.conflicts, target.key, &local, remote.as_ref(), &state,
                    "上次上传结果尚未确认，继续同步前已保留原上传文档").await?;
            },
            _ => {},
        }
        let reusable_pending = pending.as_ref().filter(|document| document.payload.hash == local.hash
            && document.metadata.parent_revision == remote.as_ref().map(|document| document.metadata.revision))
            .map(|document| document.metadata);
        // Retry needs only the checked metadata and original shared wire bytes,
        // not a second decoded settings tree retained during network I/O.
        drop(pending);
        if kind == RunKind::Manual {
            match direction {
                SyncDirection::Upload => return self.upload(target, &local, remote.as_ref(), &state, reusable_pending, true, expected, cancel).await,
                SyncDirection::Download => return self.download(target, &local, remote.as_ref(), expected, cancel).await,
                SyncDirection::Bidirectional => {},
            }
        }
        let Some(base) = &state.baseline else {
            return match remote.as_ref() {
                None if direction != SyncDirection::Download => self.upload(target, &local, None, &state, reusable_pending, false, expected, cancel).await,
                None => Err(Error::Invalid("远端配置不存在，无法按下载方向同步".into())),
                Some(remote) if remote.payload.hash == local.hash => { self.establish(target, remote).await?; Ok(outcome("本机与远端配置相同，已建立同步基线")) },
                Some(remote) => self.conflict(target, &local, Some(remote), &state, "首次连接发现不同配置，双方内容已保留").await,
            };
        };
        let Some(remote) = remote.as_ref() else { return self.conflict(target, &local, None, &state, "远端配置已删除，本机内容已保留").await; };
        if local.hash == remote.payload.hash { self.establish(target, remote).await?; return Ok(outcome("本机与远端配置相同")); }
        let local_changed = local.hash != base.digest;
        let remote_changed = remote.payload.hash != base.digest;
        match (local_changed, remote_changed) {
            (true, true) => self.conflict(target, &local, Some(remote), &state, "本机和远端配置均已变化，未覆盖任一方").await,
            (true, false) if direction == SyncDirection::Download => self.conflict(target, &local, Some(remote), &state, "本机配置已变化，当前下载方向不会覆盖本机内容").await,
            (true, false) => self.upload(target, &local, Some(remote), &state, reusable_pending, false, expected, cancel).await,
            (false, true) if direction == SyncDirection::Upload => self.conflict(target, &local, Some(remote), &state, "远端配置已变化，当前上传方向不会覆盖远端内容").await,
            (false, true) => self.download(target, &local, Some(remote), expected, cancel).await,
            (false, false) => { self.establish(target, remote).await?; Ok(outcome("配置没有变化")) },
        }
    }
    async fn upload(&self, target: &Target, local: &Payload, remote: Option<&Document>, state: &SyncTargetState, pending_metadata: Option<Metadata>, manual: bool, expected: &ConfigurationState, cancel: &CancellationToken) -> Result<SyncOutcome> {
        self.ensure_current(expected, cancel)?;
        if let Some(remote) = remote { if remote.payload.hash == local.hash { self.establish(target, remote).await?; return Ok(outcome("本机与远端配置相同")); } }
        let parent = remote.map(|document| document.metadata.revision);
        let metadata = match pending_metadata { Some(metadata) => metadata, None => Metadata::new(parent)? };
        let bytes = match pending_metadata { Some(_) => state.pending.clone().ok_or_else(|| Error::Task("待核实上传文档缺失".into()))?, None => Document::encode(metadata, local)? };
        let mut headers = HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))]);
        match remote {
            Some(remote) => {
                let previous = state.baseline.as_ref().filter(|base| base.revision == remote.metadata.revision && base.digest == remote.payload.hash).and_then(|base| base.etag.as_deref());
                if let Some(etag) = remote.etag.as_deref().or(previous).filter(|etag| super::webdav::strong_etag(etag)) {
                    headers.insert(IF_MATCH, HeaderValue::from_str(etag).map_err(|_| Error::Invalid("远端 ETag 无效".into()))?);
                } else if !manual { return Err(Error::Invalid("远端没有强 ETag，自动上传无法安全覆盖".into())); }
            },
            None => { headers.insert(IF_NONE_MATCH, HeaderValue::from_static("*")); },
        }
        // Persistence precedes PUT. An uncertain request can be recognized by
        // revision and digest on a later run, including after process restart.
        self.storage.sync_pending(target.key, Some(bytes.clone())).await?;
        self.ensure_current(expected, cancel)?;
        let put = target.send(Method::PUT, &target.document, headers, Some(bytes), cancel).await;
        let precondition = matches!(&put, Ok(response) if response.status() == StatusCode::PRECONDITION_FAILED);
        let error = match put {
            Ok(response) if response.status().is_success() => None,
            Ok(response) => Some(Error::Platform(format!("WebDAV 上传失败，HTTP {}", response.status()).into())),
            Err(error) => Some(error),
        };
        match target.read(cancel).await {
            Ok(Some(actual)) if actual.metadata.revision == metadata.revision && actual.payload.hash == local.hash => {
                self.establish(target, &actual).await?;
                let mut result = outcome("配置已上传"); result.uploaded = true; Ok(result)
            },
            Ok(actual) => {
                let unchanged = match (remote, actual.as_ref()) {
                    (None, None) => true,
                    (Some(before), Some(after)) => before.metadata.revision == after.metadata.revision && before.payload.hash == after.payload.hash,
                    _ => false,
                };
                if !precondition && unchanged {
                    return Err(error.unwrap_or_else(|| Error::Platform("远端提交未确认，待下次同步核实".into())));
                }
                // Preserve the pending document in the audit before clearing
                // it; it may be the only surviving copy of an uncertain PUT.
                let audit = self.storage.sync_target(target.key).await?;
                let message = if precondition { "远端配置在同步期间发生变化" } else { "上传期间远端配置发生变化，双方内容已保留" };
                let result = self.conflict(target, local, actual.as_ref(), &audit, message).await?;
                self.storage.sync_pending(target.key, None).await?;
                Ok(result)
            },
            Err(Error::Cancelled) => Err(Error::Cancelled),
            Err(verify) => Err(Error::Platform(match error {
                Some(error) => format!("{error}；上传结果未确认：{verify}"),
                None => format!("上传结果未确认：{verify}"),
            }.into())),
        }
    }
    async fn download(&self, target: &Target, local: &Payload, remote: Option<&Document>, expected: &ConfigurationState, cancel: &CancellationToken) -> Result<SyncOutcome> {
        let remote = remote.ok_or_else(|| Error::Invalid("远端配置不存在，无法下载".into()))?;
        self.ensure_current(expected, cancel)?;
        if local.hash == remote.payload.hash { self.establish(target, remote).await?; return Ok(outcome("本机与远端配置相同")); }
        let mutation = ConfigurationMutation { client_id: Uuid::new_v4(), mutation_id: Uuid::new_v4(),
            base_versions: scope(RunKind::Manual).into_iter().map(|field| (field, version(expected, field))).collect(), changes: remote.payload.value.changes(), credentials: Vec::new() };
        let committed = self.configuration.commit(mutation, self.effects.clone()).await?;
        // A successful commit is authoritative even when its configuration
        // watch cancels the old schedule or metadata persistence later fails.
        self.update_status(|status| { status.restart_required = true; status.restart_revision = committed.state.snapshot.revision; });
        self.establish(target, remote).await?;
        let mut result = outcome("配置已下载，请重启应用以完全生效"); result.downloaded = true; result.restart_required = true; Ok(result)
    }
    async fn establish(&self, target: &Target, document: &Document) -> Result<()> {
        let timestamp = crate::now_ms()?;
        self.storage.sync_establish(target.key, baseline(document), timestamp).await?;
        self.update_status(|status| status.last_sync_time = timestamp);
        Ok(())
    }
}
fn baseline(document: &Document) -> SyncBaseline { SyncBaseline { revision: document.metadata.revision, digest: document.payload.hash, etag: document.etag.clone() } }
