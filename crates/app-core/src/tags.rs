use std::{future::Future, sync::{Arc, Mutex}};
use super_paste_format::{RecordId, Tag, TagId};
use tokio::{runtime::Handle, sync::Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use crate::{Error, Result, events::{AppEvent, EventBus, RecordChange}, now_ms, storage::{Storage, TagChanges}};

pub struct TagService {
    storage: Arc<Storage>, events: EventBus, runtime: Handle,
    admission: Mutex<bool>, pending: Arc<Semaphore>, tasks: TaskTracker,
}
impl TagService {
    pub fn new(storage: Arc<Storage>, events: EventBus, runtime: Handle) -> Arc<Self> {
        Arc::new(Self { storage, events, runtime, admission: Mutex::new(true), pending: Arc::new(Semaphore::new(16)), tasks: TaskTracker::new() })
    }
    pub async fn list(&self, cancel: &CancellationToken) -> Result<Vec<Tag>> { self.storage.tags(cancel).await }
    async fn owned<T: Send + 'static>(&self, operation: impl Future<Output = Result<T>> + Send + 'static) -> Result<T> {
        let task = {
            let accepting = self.admission.lock().map_err(|_| Error::Task("标签接入锁损坏".into()))?;
            if !*accepting { return Err(Error::Cancelled); }
            let permit = self.pending.clone().try_acquire_owned().map_err(|error| match error {
                tokio::sync::TryAcquireError::Closed => Error::Cancelled,
                tokio::sync::TryAcquireError::NoPermits => Error::ResourceLimit("标签操作队列已满".into()),
            })?;
            self.tasks.spawn_on(async move {
                let _permit = permit;
                let result = operation.await;
                if let Err(error) = &result { tracing::warn!(%error, "标签操作失败"); }
                result
            }, &self.runtime)
        };
        task.await?
    }
    pub async fn create(self: &Arc<Self>, name: String, color: String, description: Option<String>) -> Result<TagId> {
        let values = normalize(name, color, description)?;
        let service = self.clone();
        self.owned(async move {
            let id = service.storage.create_tag(values.0, values.1, values.2, now_ms()?).await?;
            service.events.publish(AppEvent::TagsChanged { id: Some(id) });
            Ok(id)
        }).await
    }
    pub async fn update(self: &Arc<Self>, id: TagId, name: String, color: String, description: Option<String>) -> Result<()> {
        let (name, color, description) = normalize(name, color, description)?;
        let service = self.clone();
        self.owned(async move {
            if !service.storage.update_tag(id, TagChanges { name: Some(name), color: Some(color), description: Some(description) }, now_ms()?).await? { return Err(Error::NotFound); }
            service.events.publish(AppEvent::TagsChanged { id: Some(id) });
            Ok(())
        }).await
    }
    pub async fn delete(self: &Arc<Self>, id: TagId) -> Result<()> {
        let service = self.clone();
        self.owned(async move {
            if !service.storage.delete_tag(id).await? { return Err(Error::NotFound); }
            service.events.publish(AppEvent::TagsChanged { id: Some(id) });
            Ok(())
        }).await
    }
    pub async fn set_binding(self: &Arc<Self>, record: RecordId, tag: TagId, bound: bool) -> Result<()> {
        let service = self.clone();
        self.owned(async move {
            let changed = if bound { service.storage.bind_tag(record, tag, now_ms()?).await? } else { service.storage.unbind_tag(record, tag, now_ms()?).await? };
            if changed {
                service.events.publish(AppEvent::TagsChanged { id: Some(tag) });
                service.events.publish(AppEvent::HistoryChanged { ids: Arc::from([record]), change: RecordChange::Metadata });
            }
            Ok(())
        }).await
    }
    pub async fn shutdown(&self) -> Result<()> {
        let failure = match self.admission.lock() {
            Ok(mut accepting) => { *accepting = false; None }
            Err(poisoned) => { *poisoned.into_inner() = false; Some(Error::Task("标签接入锁损坏".into())) }
        };
        self.pending.close(); self.tasks.close(); self.tasks.wait().await;
        failure.map_or(Ok(()), Err)
    }
}
fn normalize(name: String, color: String, description: Option<String>) -> Result<(String, String, Option<String>)> {
    let name = name.trim().to_owned();
    if name.is_empty() { return Err(Error::Invalid("请输入标签名称".into())); }
    if name.encode_utf16().count() > 20 { return Err(Error::Invalid("标签名称不能超过 20 个字符".into())); }
    if color.len() != 7 || !color.starts_with('#') || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Invalid("标签颜色必须是六位十六进制颜色".into()));
    }
    let description = description.map(|description| description.trim().to_owned()).filter(|description| !description.is_empty());
    if description.as_ref().is_some_and(|description| description.encode_utf16().count() > 100) { return Err(Error::Invalid("标签描述不能超过 100 个字符".into())); }
    Ok((name, color.to_ascii_lowercase(), description))
}
