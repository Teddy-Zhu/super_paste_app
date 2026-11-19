use std::{collections::HashMap, sync::{Arc, atomic::{AtomicU64, Ordering}}};
use super_paste_core::{Error, Result, storage::Storage};
use super_paste_format::{HistoryEntry, HistoryPage, HistoryQuery};
use tokio::{runtime::Handle, sync::watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Request { pub generation: u64, pub append: bool, pub query: HistoryQuery, target_len: usize, known: Arc<[Arc<HistoryEntry>]>, cancel: CancellationToken }
#[derive(Clone)]
pub struct Reply { pub generation: u64, pub append: bool, pub result: Result<Arc<HistoryPage>> }

/// One accepted SQLite read and one replaceable latest request. A hidden view
/// clears both watch values; completed obsolete reads never repopulate it.
pub struct HistoryReader { requests: watch::Sender<Option<Request>>, replies: watch::Sender<Option<Reply>>, stop: CancellationToken, sequence: AtomicU64 }
impl HistoryReader {
    pub fn new(storage: Arc<Storage>, runtime: &Handle) -> Self {
        let (requests, mut receiver) = watch::channel::<Option<Request>>(None);
        let pending = requests.clone();
        let (replies, _) = watch::channel(None); let output = replies.clone();
        let stop = CancellationToken::new(); let cancelled = stop.clone();
        runtime.spawn(async move {
            loop {
                let request = receiver.borrow_and_update().clone();
                match request {
                    Some(request) => {
                        // The in-flight read owns the hint now. Do not keep a
                        // previous window alive in the completed watch value.
                        pending.send_if_modified(|current| {
                            if let Some(current) = current.as_mut().filter(|current| current.generation == request.generation) {
                                current.known = Arc::from([]);
                            }
                            false
                        });
                        let result = read_window(&storage, request.query, request.target_len, request.known, &request.cancel).await.map(Arc::new);
                        if cancelled.is_cancelled() { break; }
                        {
                            let current = receiver.borrow();
                            if current.as_ref().is_some_and(|current| current.generation == request.generation) {
                                output.send_replace(Some(Reply { generation: request.generation, append: request.append, result }));
                            }
                        }
                        if receiver.has_changed().unwrap_or(false) { continue; }
                    }
                    None => { output.send_replace(None); }
                }
                tokio::select! {
                    _ = cancelled.cancelled() => break,
                    changed = receiver.changed() => if changed.is_err() { break; },
                }
            }
        });
        Self { requests, replies, stop, sequence: AtomicU64::new(0) }
    }
    pub fn subscribe(&self) -> watch::Receiver<Option<Reply>> { self.replies.subscribe() }
    pub fn request(&self, mut query: HistoryQuery, append: bool, target_len: usize, known: Arc<[Arc<HistoryEntry>]>) -> Result<u64> {
        if target_len == 0 { return Err(Error::Invalid("历史读取范围不能为空".into())); }
        query.limit = (query.limit as usize).min(target_len) as u32;
        let generation = self.sequence.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1)).map_err(|_| Error::ResourceLimit("历史查询代次已耗尽".into()))? + 1;
        self.requests.send_modify(|current| {
            if let Some(previous) = current.take() { previous.cancel.cancel(); }
            *current = Some(Request { generation, append, query, target_len, known, cancel: self.stop.child_token() });
            self.replies.send_replace(None);
        });
        Ok(generation)
    }
    pub fn suspend(&self) { self.requests.send_modify(|current| {
        if let Some(previous) = current.take() { previous.cancel.cancel(); }
        self.replies.send_replace(None);
    }); }
}
impl Drop for HistoryReader { fn drop(&mut self) { self.stop.cancel(); self.suspend(); } }

/// Refresh the loaded metadata range without turning SQLite's page bound into
/// a UI history limit. Each page yields the ordered worker to other commands;
/// the request's existing cancellation and generation retire the whole range.
async fn read_window(storage: &Storage, mut query: HistoryQuery, target_len: usize, known: Arc<[Arc<HistoryEntry>]>, cancel: &CancellationToken) -> Result<HistoryPage> {
    let known = Arc::new(known.iter().map(|entry| (entry.id, entry.clone())).collect::<HashMap<_, _>>());
    let mut page = storage.history_reusing(query.clone(), known.clone(), cancel).await?;
    while page.entries.len() < target_len {
        let Some(cursor) = page.next_cursor.take() else { break; };
        query.cursor = Some(cursor);
        query.limit = (query.limit as usize).min(target_len - page.entries.len()) as u32;
        let next = storage.history_reusing(query.clone(), known.clone(), cancel).await?;
        page.entries.extend(next.entries);
        page.next_cursor = next.next_cursor;
    }
    Ok(page)
}
