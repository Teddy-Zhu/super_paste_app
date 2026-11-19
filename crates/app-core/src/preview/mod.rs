mod types;
mod providers;
mod structured;
mod link;
#[cfg(test)]
mod tests;
pub mod rich;
pub use types::{PreviewScope, PreviewKind, PreviewBody, PreviewModel, PreviewNotice, FilePreview, LinkPreview, ColorPreview, TablePreview};
pub use structured::excerpt;
pub use link::ExternalLink;

/// Original preview trim()/RegExp whitespace semantics. This is not the CSS
/// collapsible-space set: Unicode NEL is excluded and BOM is included.
pub fn text_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

use std::{collections::HashSet, sync::{Arc, Mutex}};
use super_paste_format::{Formats, HistoryEntry, RecordId};
use tokio::{runtime::Handle, sync::{OwnedSemaphorePermit, Semaphore}};
use tokio_util::{sync::CancellationToken, task::{TaskTracker, task_tracker::TaskTrackerToken}};
use crate::{Error, Result, resources::{ByteBudget, WorkCache}, storage::Storage};
pub(crate) use types::Input;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key { id: RecordId, revision: u64, scope: PreviewScope }
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum RichFormat { Html, Rtf }
impl RichFormat {
    fn kind(self) -> PreviewKind { match self { Self::Html => PreviewKind::Html, Self::Rtf => PreviewKind::Rtf } }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DocumentKey { id: RecordId, revision: u64, format: RichFormat }
/// Blocking parsers outlive an abandoned async waiter. Keep both admission and
/// shutdown ownership with the actual computation, including shared documents.
struct ParseWork { _permit: OwnedSemaphorePermit, _task: TaskTrackerToken }
/// Volatile results: 16 MiB models, 8 MiB shared documents, 64 MiB live input/results,
/// two parsers, and WorkCache's shared bound of 64 unique pending keys.
pub struct PreviewService {
    storage: Arc<Storage>, runtime: Handle, cache: Arc<WorkCache<Key, PreviewModel>>,
    documents: Arc<WorkCache<DocumentKey, rich::Document>>,
    memory: ByteBudget, workers: Arc<Semaphore>, admission: Mutex<bool>, tasks: TaskTracker,
}
impl PreviewService {
    pub fn new(storage: Arc<Storage>, runtime: Handle) -> Arc<Self> {
        Arc::new(Self { storage, runtime, cache: WorkCache::new(16 * 1024 * 1024), documents: WorkCache::new(8 * 1024 * 1024), memory: ByteBudget::new(64 * 1024 * 1024, "内容预览"), workers: Arc::new(Semaphore::new(2)), admission: Mutex::new(true), tasks: TaskTracker::new() })
    }
    pub async fn get(self: &Arc<Self>, entry: Arc<HistoryEntry>, scope: PreviewScope, cancel: &CancellationToken) -> Result<Arc<PreviewModel>> {
        let (_request, loader) = {
            let accepting = self.admission.lock().map_err(|_| Error::Task("预览接入锁损坏".into()))?;
            if !*accepting || cancel.is_cancelled() { return Err(Error::Cancelled); }
            (self.tasks.token(), self.tasks.token())
        };
        let key = Key { id: entry.id, revision: entry.content_revision, scope }; let service = self.clone();
        let result = self.cache.get_or_load_observed(key, &self.runtime, cancel, move |producer_cancel| async move {
            let permit = service.workers.clone().acquire_owned().await.map_err(|_| Error::Cancelled)?;
            let work = Arc::new(ParseWork { _permit: permit, _task: loader });
            if producer_cancel.is_cancelled() { return Err(Error::Cancelled); }
            let needs_payload = scope == PreviewScope::Details || entry.truncated.intersects(Formats::TEXT | Formats::HTML | Formats::RTF | Formats::FILES);
            let input = if needs_payload { service.storage.preview_input(entry.clone(), scope, service.memory.clone(), &producer_cancel).await? }
                else {
                    let lease = service.memory.reserve(entry.excerpt.byte_len().saturating_add(4096))?;
                    Input { content: entry.excerpt.clone(), primary_type: entry.primary_type, image: entry.image.clone(), text_characters: entry.text_characters, text_clipped: false, html_omitted: false, rtf_omitted: false, scope, lease }
                };
            let prepared = service.run_parser(work.clone(), &producer_cancel, move || Ok(providers::select(input))).await?;
            let (prepared, rich) = if prepared.needs_rich {
                let prepared = Arc::new(prepared);
                let rich = service.resolve_rich(key, &prepared, work.clone(), &producer_cancel).await?;
                // A completed document producer releases its source owner
                // before publishing. The final model takes back the original
                // allocation and budget rather than cloning either one.
                let prepared = Arc::try_unwrap(prepared).map_err(|_| Error::Task("预览输入仍被解析任务持有".into()))?;
                (prepared, Some(rich))
            } else { (prepared, None) };
            service.run_parser(work, &producer_cancel, move || providers::finish(prepared, rich)).await
        }).await?;
        // Cache identity is independent of metadata/OCR revisions. A small hot
        // lookup also prevents a cached result from reviving a deleted record.
        if cancel.is_cancelled() || self.storage.record_revision(key.id, cancel).await? != Some(key.revision) { return Err(Error::Cancelled); }
        Ok(result)
    }
    pub async fn external_link(self: &Arc<Self>, id: RecordId) -> Result<ExternalLink> {
        let (_request, work) = {
            let accepting = self.admission.lock().map_err(|_| Error::Task("预览接入锁损坏".into()))?;
            if !*accepting { return Err(Error::Cancelled); }
            (self.tasks.token(), self.tasks.token())
        };
        let permit = self.workers.clone().acquire_owned().await.map_err(|_| Error::Cancelled)?;
        let (revision, text, lease) = self.storage.external_link_input(id, self.memory.clone()).await?;
        let link = self.runtime.spawn_blocking(move || { let (_work, _permit) = (work, permit); ExternalLink::new(text, lease) }).await??;
        match self.storage.record_revision(id, &CancellationToken::new()).await? {
            Some(current) if current == revision => Ok(link),
            Some(_) => Err(Error::Cancelled), None => Err(Error::NotFound),
        }
    }
    async fn run_parser<T, F>(&self, work: Arc<ParseWork>, cancel: &CancellationToken, parse: F) -> Result<T>
    where T: Send + 'static, F: FnOnce() -> Result<T> + Send + 'static {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let cancel = cancel.clone();
        self.runtime.spawn_blocking(move || {
            let _work = work;
            if cancel.is_cancelled() { return Err(Error::Cancelled); }
            let result = parse()?;
            if cancel.is_cancelled() { Err(Error::Cancelled) } else { Ok(result) }
        }).await?
    }
    async fn resolve_rich(self: &Arc<Self>, key: Key, prepared: &Arc<providers::Prepared>, work: Arc<ParseWork>, cancel: &CancellationToken) -> Result<providers::RichResolution> {
        let input = &prepared.input;
        let mut result = providers::RichResolution { kind: if input.content.html.is_some() || input.html_omitted { PreviewKind::Html } else { PreviewKind::Rtf }, document: None, notice: None };
        for (source, omitted, format) in [(&input.content.html, input.html_omitted, RichFormat::Html), (&input.content.rtf, input.rtf_omitted, RichFormat::Rtf)] {
            if omitted { result.notice = Some(PreviewNotice::RichTooLarge); continue; }
            let Some(source) = source.as_ref().filter(|value| !value.is_empty()).cloned() else { continue; };
            let memory = self.memory.clone(); let service = self.clone(); let work = work.clone(); let owner = prepared.clone();
            let document_key = DocumentKey { id: key.id, revision: key.revision, format };
            // Scope is deliberately absent. Both scopes load the same bounded
            // full HTML/RTF source and reuse one parse, including inline bytes.
            let document = self.documents.get_or_load_observed(document_key, &self.runtime, cancel, move |document_cancel| async move {
                service.run_parser(work, &document_cancel, move || {
                    // A different scope may still need this parse after its
                    // original model waiter leaves. Retain the source's real
                    // input lease until blocking parsing has actually ended.
                    let _source_owner = owner;
                    let mut document = match format { RichFormat::Html => rich::html(&source), RichFormat::Rtf => rich::rtf(&source) }
                        .map_err(|error| match error { Error::ResourceLimit(message) => Error::Invalid(message), other => other })?;
                    document.charge(&memory)?;
                    Ok(document)
                }).await
            }).await;
            match document {
                Ok(document) if document.has_content() => { result.document = Some(document); result.kind = format.kind(); result.notice = None; break; },
                Ok(_) => {},
                Err(Error::Invalid(_)) => { tracing::warn!("受控富文本无法排版"); result.notice = Some(PreviewNotice::RichUnavailable); },
                // Transient resource/task failures must not become a cached
                // plain-text fallback for an otherwise valid document.
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }
    pub fn invalidate(&self, ids: &[RecordId]) -> Result<()> {
        let ids: HashSet<_> = ids.iter().copied().collect();
        let models = self.cache.invalidate_where(|key| ids.contains(&key.id));
        let documents = self.documents.invalidate_where(|key| ids.contains(&key.id));
        models.and(documents)
    }
    pub fn trim(&self) -> Result<()> { let models = self.cache.trim(); let documents = self.documents.trim(); models.and(documents) }
    pub fn live_bytes(&self) -> u64 { self.memory.used() }
    pub async fn shutdown(&self) -> Result<()> {
        let failure = match self.admission.lock() {
            Ok(mut accepting) => { *accepting = false; None },
            Err(poisoned) => { *poisoned.into_inner() = false; Some(Error::Task("预览接入锁损坏".into())) },
        };
        self.workers.close(); self.tasks.close(); self.tasks.wait().await; self.trim()?;
        failure.map_or(Ok(()), Err)
    }
}
