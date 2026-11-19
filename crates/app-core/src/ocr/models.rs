use super::catalog::{
    ModelInfo, ModelStage, ModelState, OcrModelStatus, all_models, get_model, model_revision,
    model_status, release_url,
};
use crate::events::{EventBus, AppEvent};
use futures_util::{FutureExt, StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    panic::AssertUnwindSafe,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, RwLock, watch},
    time::Instant,
    runtime::Handle,
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use zip::ZipArchive;

const MAX_ARCHIVE_BYTES: u64 = 160_000_000;
const MAX_EXTRACTED_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_TOTAL_BYTES: u64 = 768 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 128;
const MAX_DICTIONARY_BYTES: u64 = 16 * 1024 * 1024;
// Installed manifests contain three file identities, not model data. Bound
// the read itself so a damaged file cannot grow with the installed weights.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const DOWNLOAD_CANCELLED: &str = "模型下载已取消";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FileIntegrity {
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelManifest {
    format: u32,
    model_id: String,
    revision: String,
    files: HashMap<String, FileIntegrity>,
}

#[derive(Clone, Debug)]
pub struct InstalledModel {
    pub(super) info: ModelInfo,
    pub(super) detector: PathBuf,
    pub(super) recognizer: PathBuf,
    pub(super) dictionary: PathBuf,
}

#[derive(Clone)]
pub struct ModelManager {
    inner: Arc<ModelManagerInner>,
}

struct ModelManagerInner {
    root: PathBuf,
    client: reqwest::Client,
    events: EventBus,
    statuses: Mutex<HashMap<String, OcrModelStatus>>,
    status_read: Mutex<()>,
    downloads: Mutex<HashMap<String, DownloadTask>>,
    maintenance: Mutex<HashMap<String, CancellationToken>>,
    runtime: Handle,
    closing: CancellationToken,
    tasks: TaskTracker,
    admission: RwLock<()>,
}

#[derive(Clone, Debug)]
pub struct ModelsDiagnostics { pub downloads: usize, pub models: Vec<OcrModelStatus>, pub status_error: Option<String> }

#[derive(Clone)]
struct DownloadTask {
    generation: String,
    cancel: CancellationToken,
    result: watch::Receiver<Option<Result<InstalledModel, String>>>,
}

impl ModelManager {
    pub fn new(root: PathBuf, events: EventBus, runtime: Handle) -> Result<Self, String> {
        all_models()?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| format!("无法初始化模型下载客户端: {error}"))?;
        Ok(Self {
            inner: Arc::new(ModelManagerInner {
                root,
                client,
                events,
                statuses: Mutex::new(HashMap::new()),
                status_read: Mutex::new(()),
                downloads: Mutex::new(HashMap::new()),
                maintenance: Mutex::new(HashMap::new()),
                runtime, closing: CancellationToken::new(), tasks: TaskTracker::new(), admission: RwLock::new(()),
            }),
        })
    }

    pub async fn cleanup_staging(&self) -> Result<(), String> {
        let _admission = self.inner.admission.write().await;
        if !self.inner.tasks.is_empty() { return Err("模型任务运行中，不能清理暂存目录".to_owned()); }
        tokio::fs::create_dir_all(&self.inner.root)
            .await
            .map_err(|error| format!("无法创建 OCR 模型目录: {error}"))?;
        let mut entries = tokio::fs::read_dir(&self.inner.root)
            .await
            .map_err(|error| format!("无法读取 OCR 模型目录: {error}"))?;
        let mut stale_downloads = Vec::new();
        let mut backups = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| format!("无法读取 OCR 模型目录: {error}"))?
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".download-") {
                stale_downloads.push(entry.path());
            } else if name.starts_with(".previous-") {
                backups.push(entry.path());
            }
        }
        for path in stale_downloads {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|error| format!("无法清理中断的模型下载: {error}"))?;
        }
        for backup in backups {
            let Some(name) = backup.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some((model_id, _)) = name
                .strip_prefix(".previous-")
                .and_then(|rest| rest.rsplit_once('-'))
            else {
                continue;
            };
            if get_model(model_id).is_err() {
                tokio::fs::remove_dir_all(backup)
                    .await
                    .map_err(|error| format!("无法清理未知模型备份: {error}"))?;
                continue;
            }
            let current = self.inner.root.join(model_id);
            if tokio::fs::try_exists(&current)
                .await
                .map_err(|error| format!("无法检查 OCR 模型目录: {error}"))?
            {
                tokio::fs::remove_dir_all(backup)
                    .await
                    .map_err(|error| format!("无法清理旧 OCR 模型备份: {error}"))?;
            } else {
                tokio::fs::rename(&backup, &current)
                    .await
                    .map_err(|error| format!("无法恢复中断的 OCR 模型替换: {error}"))?;
            }
        }
        Ok(())
    }

    pub async fn list(&self, caller: &CancellationToken) -> crate::Result<Vec<OcrModelStatus>> {
        self.read(caller, |manager, cancel| async move { manager.list_inner(&cancel).await }).await
    }

    async fn read<T, F, Fut>(&self, caller: &CancellationToken, operation: F) -> crate::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Self, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        let admission = tokio::select! {
            biased;
            _ = caller.cancelled() => return Err(crate::Error::Cancelled),
            _ = self.inner.closing.cancelled() => return Err(crate::Error::Cancelled),
            admission = self.inner.admission.read() => admission,
        };
        if caller.is_cancelled() || self.inner.closing.is_cancelled() { return Err(crate::Error::Cancelled); }
        let cancel = self.inner.closing.child_token();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let manager = self.clone();
        let worker_cancel = cancel.clone();
        // The manager owns filesystem work through its actual completion, even
        // when the page drops its waiter during a Tokio filesystem operation.
        let mut task = self.inner.tasks.spawn_on(async move {
            let result = operation(manager, worker_cancel.clone()).await;
            if worker_cancel.is_cancelled() { Err(crate::Error::Cancelled) }
            else { result.map_err(|error| crate::Error::Platform(error.into())) }
        }, &self.inner.runtime);
        drop(admission);
        tokio::select! {
            biased;
            _ = caller.cancelled() => { cancel.cancel(); task.await },
            result = &mut task => result,
        }.map_err(crate::Error::from)?
    }

    async fn list_inner(&self, cancel: &CancellationToken) -> Result<Vec<OcrModelStatus>, String> {
        let _read = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(DOWNLOAD_CANCELLED.to_owned()),
            read = self.inner.status_read.lock() => read,
        };
        check_cancelled(cancel)?;
        let in_progress = self.inner.statuses.lock().await.clone();
        let models = all_models()?;
        let mut statuses = Vec::with_capacity(models.len());
        for model in models {
            check_cancelled(cancel)?;
            if in_progress
                .get(&model.id)
                .is_some_and(|status| status.state == ModelState::Downloading)
            {
                statuses.push(in_progress[&model.id].clone());
                continue;
            }
            if self.inspect_installed(model, false, Some(cancel)).await?.is_some() {
                let mut status = model_status(model, ModelState::Ready, None);
                status.received_bytes = model.size;
                statuses.push(status);
            } else {
                statuses.push(
                    in_progress
                        .get(&model.id)
                        .filter(|status| status.state == ModelState::Error)
                        .cloned()
                        .unwrap_or_else(|| {
                            model_status(model, ModelState::NotDownloaded, None)
                        }),
                );
            }
        }
        check_cancelled(cancel)?;
        Ok(statuses)
    }

    pub async fn diagnostics(&self) -> ModelsDiagnostics {
        let downloads = self.inner.downloads.lock().await.len();
        let (models, status_error) = match self.list(&self.inner.closing).await {
            Ok(statuses) => (statuses, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        ModelsDiagnostics { downloads, models, status_error }
    }

    pub async fn download(&self, id: &str) -> Result<InstalledModel, String> {
        let admission = self.inner.admission.read().await;
        if self.inner.closing.is_cancelled() { return Err("OCR 模型服务已关闭".to_owned()); }
        let model = get_model(id)?.clone();
        let maintenance = self.inner.maintenance.lock().await;
        if maintenance.contains_key(id) {
            return Err("模型正在维护，请稍后重试".to_owned());
        }
        let generation = uuid::Uuid::new_v4().to_string();
        let (task, launch, sender) = {
            let mut downloads = self.inner.downloads.lock().await;
            if let Some(task) = downloads.get(id) {
                (task.clone(), false, None)
            } else {
                let cancel = self.inner.closing.child_token();
                let (sender, result) = watch::channel(None);
                let task = DownloadTask {
                    generation: generation.clone(),
                    cancel: cancel.clone(),
                    result,
                };
                downloads.insert(id.to_owned(), task.clone());
                (task, true, Some(sender))
            }
        };
        drop(maintenance);
        let mut receiver = task.result.clone();
        if launch {
            let manager = self.clone();
            let model_id = id.to_owned();
            let task_generation = task.generation.clone();
            let cancel = task.cancel.clone();
            let Some(sender) = sender else {
                self.inner.downloads.lock().await.remove(id);
                return Err("无法启动 OCR 模型下载任务".to_owned());
            };
            self.inner.tasks.spawn_on(async move {
                let failure_model = model.clone();
                let result = match AssertUnwindSafe(manager.install(model, cancel, false)).catch_unwind().await {
                    Ok(result) => result,
                    Err(_) => {
                        let error = "OCR 模型下载任务异常退出".to_owned();
                        let mut status = model_status(&failure_model, ModelState::Error, None);
                        status.error = Some(error.clone());
                        manager.publish(status).await;
                        Err(error)
                    }
                };
                let _ = sender.send(Some(result.clone()));
                let mut downloads = manager.inner.downloads.lock().await;
                if downloads
                    .get(&model_id)
                    .is_some_and(|task| task.generation == task_generation)
                {
                    downloads.remove(&model_id);
                }
            }, &self.inner.runtime);
        }
        drop(admission);
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| "模型下载任务异常退出".to_owned())?;
        }
    }

    pub async fn cancel_download(&self, id: &str) -> Result<(), String> {
        get_model(id)?;
        let download = self.inner.downloads.lock().await.get(id).map(|task| task.cancel.clone());
        let maintenance = self.inner.maintenance.lock().await.get(id).cloned();
        if let Some(cancel) = download { cancel.cancel(); }
        if let Some(cancel) = maintenance { cancel.cancel(); }
        Ok(())
    }

    pub async fn remove(&self, id: &str) -> Result<(), String> {
        let admission = self.inner.admission.read().await;
        if self.inner.closing.is_cancelled() { return Err("OCR 模型服务已关闭".to_owned()); }
        let model = get_model(id)?;
        let manager = self.clone();
        let task = self.inner.tasks.spawn_on(async move { manager.maintain(&model.id, |_| async {
            let path = manager.inner.root.join(&model.id);
            if tokio::fs::try_exists(&path)
                .await
                .map_err(|error| format!("无法检查 OCR 模型目录: {error}"))?
            {
                tokio::fs::remove_dir_all(&path)
                    .await
                    .map_err(|error| format!("删除 OCR 模型失败: {error}"))?;
            }
            manager.publish(model_status(model, ModelState::NotDownloaded, None))
                .await;
            Ok(())
        })
        .await }, &self.inner.runtime);
        drop(admission);
        task.await.map_err(|error| format!("模型删除任务异常退出：{error}"))?
    }

    pub async fn redownload(&self, id: &str) -> Result<InstalledModel, String> {
        let admission = self.inner.admission.read().await;
        if self.inner.closing.is_cancelled() { return Err("OCR 模型服务已关闭".to_owned()); }
        let model = get_model(id)?.clone();
        let manager = self.clone();
        let id = id.to_owned();
        let task = self.inner.tasks.spawn_on(async move { let installer = manager.clone(); manager.maintain(&id, move |cancel| async move {
            installer.install(model, cancel, true).await
        }).await }, &self.inner.runtime);
        drop(admission);
        task.await.map_err(|error| format!("模型重新下载任务异常退出：{error}"))?
    }

    pub async fn cancel_all(&self) {
        let downloads = self.inner.downloads.lock().await.values().map(|task| task.cancel.clone()).collect::<Vec<_>>();
        let maintenance = self.inner.maintenance.lock().await.values().cloned().collect::<Vec<_>>();
        for cancel in downloads.into_iter().chain(maintenance) { cancel.cancel(); }
    }
    pub async fn shutdown(&self) {
        self.inner.closing.cancel();
        {
            let _admission = self.inner.admission.write().await;
            self.inner.tasks.close();
        }
        self.inner.tasks.wait().await;
    }

    async fn maintain<T, F, O>(&self, id: &str, operation: O) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
        O: FnOnce(CancellationToken) -> F,
    {
        let cancel = self.inner.closing.child_token();
        {
            let mut maintenance = self.inner.maintenance.lock().await;
            if maintenance.contains_key(id) {
                return Err("模型正在维护，请稍后重试".to_owned());
            }
            maintenance.insert(id.to_owned(), cancel.clone());
        }
        let result = AssertUnwindSafe(async {
            let task = self.inner.downloads.lock().await.get(id).cloned();
            if let Some(task) = task {
                task.cancel.cancel();
                let mut receiver = task.result;
                while receiver.borrow().is_none() {
                    if receiver.changed().await.is_err() { break; }
                }
            }
            operation(cancel).await
        }).catch_unwind().await.unwrap_or_else(|_| Err("OCR 模型维护任务异常退出".to_owned()));
        self.inner.maintenance.lock().await.remove(id);
        result
    }

    pub async fn installed(
        &self, id: &str, cancel: &CancellationToken,
    ) -> crate::Result<Option<InstalledModel>> {
        let model = get_model(id).map_err(|error| crate::Error::Platform(error.into()))?.clone();
        self.read(cancel, move |manager, cancel| async move {
            manager.inspect_installed(&model, true, Some(&cancel)).await
        }).await
    }
    async fn inspect_installed(
        &self,
        model: &ModelInfo,
        verify_hashes: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<Option<InstalledModel>, String> {
        if let Some(cancel) = cancel { check_cancelled(cancel)?; }
        let directory = self.inner.root.join(&model.id);
        let manifest_path = directory.join("manifest.json");
        let file = match tokio::fs::File::open(&manifest_path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("无法读取 OCR 模型清单: {error}")),
        };
        if let Some(cancel) = cancel { check_cancelled(cancel)?; }
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes).await
            .map_err(|error| format!("无法读取 OCR 模型清单: {error}"))?;
        if let Some(cancel) = cancel { check_cancelled(cancel)?; }
        // Treat oversized manifests like malformed JSON: the installation is
        // not usable and may be repaired through the existing download path.
        if bytes.len() as u64 > MAX_MANIFEST_BYTES { return Ok(None); }
        let manifest: ModelManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(_) => return Ok(None),
        };
        if manifest.format != 1
            || manifest.model_id != model.id
            || manifest.revision != model_revision(model)
        {
            return Ok(None);
        }
        for (role, name) in [
            ("det", "det.onnx"),
            ("rec", "rec.onnx"),
            ("dictionary", "dict.txt"),
        ] {
            if let Some(cancel) = cancel { check_cancelled(cancel)?; }
            let Some(expected) = manifest.files.get(role) else {
                return Ok(None);
            };
            if expected.size == 0 || expected.size > MAX_EXTRACTED_FILE_BYTES || !is_sha256(&expected.sha256) {
                return Ok(None);
            }
            let path = directory.join(name);
            let metadata = match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.is_file() && metadata.len() == expected.size => metadata,
                Ok(_) => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(format!("无法读取 OCR 模型文件: {error}")),
            };
            if let Some(cancel) = cancel { check_cancelled(cancel)?; }
            if name == "dict.txt" && metadata.len() > MAX_DICTIONARY_BYTES {
                return Ok(None);
            }
            if verify_hashes && hash_file(&path, cancel).await? != expected.sha256 {
                return Ok(None);
            }
        }
        Ok(Some(InstalledModel {
            info: model.clone(),
            detector: directory.join("det.onnx"),
            recognizer: directory.join("rec.onnx"),
            dictionary: directory.join("dict.txt"),
        }))
    }

    fn install(
        &self,
        model: ModelInfo,
        cancel: CancellationToken,
        force: bool,
    ) -> BoxFuture<'_, Result<InstalledModel, String>> {
        Box::pin(async move {
            let mut status = model_status(
                &model,
                ModelState::Downloading,
                Some(ModelStage::Verifying),
            );
            self.publish(status.clone()).await;
            let result = self
                .install_inner(&model, &cancel, force, &mut status)
                .await;
            match result {
                Ok(installed) => {
                    status.state = ModelState::Ready;
                    status.stage = None;
                    status.error = None;
                    status.received_bytes = model.size;
                    self.publish(status).await;
                    Ok(installed)
                }
                Err(error) => {
                    status.state = ModelState::Error;
                    status.stage = None;
                    status.error = Some(error.clone());
                    self.publish(status).await;
                    Err(error)
                }
            }
        })
    }

    async fn install_inner(
        &self,
        model: &ModelInfo,
        cancel: &CancellationToken,
        force: bool,
        status: &mut OcrModelStatus,
    ) -> Result<InstalledModel, String> {
        let install_deadline = tokio::time::sleep(DOWNLOAD_TIMEOUT);
        let mut staging_path = None;
        let result = {
            let operation = async {
                check_cancelled(cancel)?;
                if let Some(installed) = self.inspect_installed(model, true, Some(cancel)).await? {
                    if !force {
                        status.received_bytes = model.size;
                        return Ok(installed);
                    }
                }
                tokio::fs::create_dir_all(&self.inner.root)
                    .await
                    .map_err(|error| format!("无法创建 OCR 模型目录: {error}"))?;
                let nonce = uuid::Uuid::new_v4().simple().to_string();
                let staging = self
                    .inner
                    .root
                    .join(format!(".download-{}-{nonce}", model.id));
                tokio::fs::create_dir(&staging)
                    .await
                    .map_err(|error| format!("无法创建 OCR 模型暂存目录: {error}"))?;
                staging_path = Some(staging.clone());
                let install_result = self
                    .download_and_extract(model, cancel, &staging, status)
                    .await;
                let mut files = match install_result {
                    Ok(files) => files,
                    Err(error) => {
                        let _ = tokio::fs::remove_dir_all(&staging).await;
                        return Err(error);
                    }
                };
                if !files.contains_key("det") {
                    if model.id == "ppocr-v4" {
                        let _ = tokio::fs::remove_dir_all(&staging).await;
                        return Err("下载包中缺少文字检测模型".to_owned());
                    }
                    status.stage = Some(ModelStage::PreparingDetector);
                    self.publish(status.clone()).await;
                    let detector_model = tokio::select! {
                        _ = cancel.cancelled() => return Err(DOWNLOAD_CANCELLED.to_owned()),
                        result = self.download("ppocr-v4") => result?,
                    };
                    check_cancelled(cancel)?;
                    let target = staging.join("det.onnx");
                    tokio::fs::copy(&detector_model.detector, &target)
                        .await
                        .map_err(|error| format!("无法准备共享文字检测模型: {error}"))?;
                    files.insert("det".to_owned(), file_integrity(&target, cancel).await?);
                }
                if !files.contains_key("rec") || !files.contains_key("dictionary") {
                    return Err("下载包中缺少识别模型或字典".to_owned());
                }
                let dictionary = tokio::fs::read_to_string(staging.join("dict.txt"))
                    .await
                    .map_err(|error| format!("OCR 字典不是有效 UTF-8: {error}"))?;
                if dictionary.trim().is_empty() {
                    return Err("OCR 字典为空".to_owned());
                }
                check_cancelled(cancel)?;
                status.stage = Some(ModelStage::Verifying);
                self.publish(status.clone()).await;
                let manifest = ModelManifest {
                    format: 1,
                    model_id: model.id.clone(),
                    revision: model_revision(model),
                    files,
                };
                let manifest_bytes = serde_json::to_vec(&manifest)
                    .map_err(|error| format!("无法生成 OCR 模型清单: {error}"))?;
                tokio::fs::write(staging.join("manifest.json"), manifest_bytes)
                    .await
                    .map_err(|error| format!("无法写入 OCR 模型清单: {error}"))?;
                check_cancelled(cancel)?;
                let directory = self.inner.root.join(&model.id);
                let backup = self
                    .inner
                    .root
                    .join(format!(".previous-{}-{nonce}", model.id));
                let had_previous = tokio::fs::try_exists(&directory)
                    .await
                    .map_err(|error| format!("无法检查现有 OCR 模型: {error}"))?;
                if had_previous {
                    tokio::fs::rename(&directory, &backup)
                        .await
                        .map_err(|error| format!("无法暂存旧 OCR 模型: {error}"))?;
                }
                if let Err(error) = tokio::fs::rename(&staging, &directory).await {
                    if had_previous {
                        if let Err(restore) = tokio::fs::rename(&backup, &directory).await {
                            return Err(format!("无法安装 OCR 模型: {error}；旧模型恢复失败: {restore}"));
                        }
                    }
                    return Err(format!("无法安装 OCR 模型: {error}"));
                }
                if had_previous {
                    tokio::fs::remove_dir_all(&backup)
                        .await
                        .map_err(|error| format!("OCR 模型已安装，但无法清理旧版本: {error}"))?;
                }
                // Extraction/copy already computed each installed file's hash while writing it.
                self.inspect_installed(model, false, None)
                    .await?
                    .ok_or_else(|| "OCR 模型安装后校验失败".to_owned())
            };
            complete_install(operation, cancel, install_deadline).await
        };
        if let Err(error) = result {
            if let Some(staging) = staging_path {
                if let Err(cleanup) = tokio::fs::remove_dir_all(staging).await {
                    if cleanup.kind() != std::io::ErrorKind::NotFound {
                        return Err(format!("{error}；模型暂存目录清理失败: {cleanup}"));
                    }
                }
            }
            return Err(error);
        }
        result
    }

    async fn download_and_extract(
        &self,
        model: &ModelInfo,
        cancel: &CancellationToken,
        staging: &Path,
        status: &mut OcrModelStatus,
    ) -> Result<HashMap<String, FileIntegrity>, String> {
        status.stage = Some(ModelStage::Downloading);
        status.received_bytes = 0;
        self.publish(status.clone()).await;
        let request = self.inner.client.get(release_url(model)?).send();
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(DOWNLOAD_CANCELLED.to_owned()),
            result = request => result.map_err(|error| format!("模型下载失败: {error}"))?,
        };
        if !response.status().is_success() {
            return Err(format!("模型下载失败，HTTP {}", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ARCHIVE_BYTES || length != model.size)
        {
            return Err("模型下载大小与发布信息不符".to_owned());
        }
        let archive = staging.join("model.zip.part");
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&archive)
            .await
            .map_err(|error| format!("无法创建模型压缩包: {error}"))?;
        let mut stream = response.bytes_stream();
        let mut hasher = Sha256::new();
        let mut last_progress = Instant::now();
        while let Some(next) = tokio::select! {
            _ = cancel.cancelled() => return Err(DOWNLOAD_CANCELLED.to_owned()),
            chunk = stream.next() => chunk,
        } {
            let chunk = next.map_err(|error| format!("模型下载失败: {error}"))?;
            status.received_bytes = status
                .received_bytes
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| "模型下载大小溢出".to_owned())?;
            if status.received_bytes > model.size || status.received_bytes > MAX_ARCHIVE_BYTES {
                return Err("模型下载大小与发布信息不符".to_owned());
            }
            hasher.update(&chunk);
            output
                .write_all(&chunk)
                .await
                .map_err(|error| format!("写入模型压缩包失败: {error}"))?;
            if last_progress.elapsed() >= Duration::from_millis(100) {
                last_progress = Instant::now();
                self.publish(status.clone()).await;
            }
        }
        output
            .flush()
            .await
            .map_err(|error| format!("保存模型压缩包失败: {error}"))?;
        output
            .sync_all()
            .await
            .map_err(|error| format!("保存模型压缩包失败: {error}"))?;
        drop(output);
        if status.received_bytes != model.size {
            return Err("模型下载大小与发布信息不符".to_owned());
        }
        let actual = digest_hex(hasher.finalize().as_slice());
        if actual != model.sha256 {
            return Err("模型下载校验失败，请重试".to_owned());
        }
        check_cancelled(cancel)?;
        status.stage = Some(ModelStage::Extracting);
        self.publish(status.clone()).await;
        let archive_to_read = archive.clone();
        let extraction_path = staging.to_owned();
        let cancel_for_task = cancel.clone();
        let mut extract = tokio::task::spawn_blocking(move || {
            extract_archive(&archive_to_read, &extraction_path, &cancel_for_task)
        });
        let files = tokio::select! {
            result = &mut extract => result
                .map_err(|error| format!("模型解压任务异常退出: {error}"))??,
            _ = cancel.cancelled() => {
                let _ = extract.await;
                return Err(DOWNLOAD_CANCELLED.to_owned());
            },
        };
        tokio::fs::remove_file(archive).await.map_err(|error| format!("模型已解压，但无法清理压缩包: {error}"))?;
        Ok(files)
    }

    async fn publish(&self, status: OcrModelStatus) {
        self.inner
            .statuses
            .lock()
            .await
            .insert(status.id.clone(), status.clone());
        self.inner.events.publish(AppEvent::OcrModelChanged(Arc::new(status)));
    }
}

async fn complete_install<T>(
    operation: impl std::future::Future<Output = Result<T, String>>,
    cancel: &CancellationToken,
    deadline: impl std::future::Future<Output = ()>,
) -> Result<T, String> {
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => result,
        _ = deadline => {
            let timed_out = !cancel.is_cancelled();
            cancel.cancel();
            // Preparation checks cancellation cooperatively. Directory replacement
            // must finish or restore its backup, so its actual result stays authoritative.
            match operation.await {
                Err(error) if timed_out && error == DOWNLOAD_CANCELLED => Err("模型下载超时".to_owned()),
                result => result,
            }
        },
    }
}

fn extract_archive(
    archive_path: &Path,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<HashMap<String, FileIntegrity>, String> {
    let file = File::open(archive_path).map_err(|error| format!("无法打开模型压缩包: {error}"))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("无法读取模型压缩包: {error}"))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("模型压缩包条目超过安全上限".to_owned());
    }
    let mut files = HashMap::new();
    let mut extracted_total = 0u64;
    for index in 0..archive.len() {
        check_cancelled(cancel)?;
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("无法读取模型压缩包条目: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("模型压缩包包含符号链接".to_owned());
        }
        let base = entry.name().rsplit('/').next().unwrap_or_default();
        let Some(role) = model_file_role(base) else {
            continue;
        };
        if files.contains_key(role)
            || entry.size() == 0
            || entry.size() > MAX_EXTRACTED_FILE_BYTES
            || (role == "dict" && entry.size() > MAX_DICTIONARY_BYTES)
        {
            return Err("模型压缩包包含重复或无效的文件".to_owned());
        }
        extracted_total = extracted_total
            .checked_add(entry.size())
            .ok_or_else(|| "模型解压总大小溢出".to_owned())?;
        if extracted_total > MAX_EXTRACTED_TOTAL_BYTES {
            return Err("模型解压总大小超过安全上限".to_owned());
        }
        let file_name = match role {
            "det" => "det.onnx",
            "rec" => "rec.onnx",
            _ => "dict.txt",
        };
        let target = output.join(file_name);
        let mut target_file = File::options()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| format!("无法创建模型文件: {error}"))?;
        let mut hasher = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            check_cancelled(cancel)?;
            let read = entry
                .read(&mut buffer)
                .map_err(|error| format!("解压模型文件失败: {error}"))?;
            if read == 0 {
                break;
            }
            copied = copied
                .checked_add(read as u64)
                .ok_or_else(|| "模型文件大小溢出".to_owned())?;
            if copied > entry.size() || copied > MAX_EXTRACTED_FILE_BYTES {
                return Err("模型解压文件大小超过声明值".to_owned());
            }
            hasher.update(&buffer[..read]);
            target_file
                .write_all(&buffer[..read])
                .map_err(|error| format!("保存模型文件失败: {error}"))?;
        }
        if copied != entry.size() {
            return Err("模型解压文件大小与声明值不符".to_owned());
        }
        target_file
            .sync_all()
            .map_err(|error| format!("保存模型文件失败: {error}"))?;
        let role = match file_name {
            "det.onnx" => "det",
            "rec.onnx" => "rec",
            _ => "dictionary",
        };
        files.insert(
            role.to_owned(),
            FileIntegrity {
                size: copied,
                sha256: digest_hex(hasher.finalize().as_slice()),
            },
        );
    }
    Ok(files)
}

fn model_file_role(name: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    if name.ends_with(".onnx")
        && (name == "det.onnx"
            || name.starts_with("det_")
            || name.contains("_det_")
            || name.ends_with("_det.onnx"))
    {
        return Some("det");
    }
    if name.ends_with(".onnx")
        && (name == "rec.onnx"
            || name.starts_with("rec_")
            || name.contains("_rec_")
            || name.ends_with("_rec.onnx"))
    {
        return Some("rec");
    }
    if name.ends_with(".txt") && !name.contains("readme") && !name.contains("license") {
        return Some("dict");
    }
    None
}

fn check_cancelled(token: &CancellationToken) -> Result<(), String> {
    if token.is_cancelled() {
        Err(DOWNLOAD_CANCELLED.to_owned())
    } else {
        Ok(())
    }
}

async fn hash_file(path: &Path, cancel: Option<&CancellationToken>) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("无法读取模型文件: {error}"))?;
    let mut hash = Sha256::new();
    // Keep the I/O block out of the nested installation/verification futures'
    // inline state; it is reused for the file and released on completion.
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        if let Some(cancel) = cancel { check_cancelled(cancel)?; }
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取模型文件失败: {error}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(digest_hex(hash.finalize().as_slice()))
}

async fn file_integrity(path: &Path, cancel: &CancellationToken) -> Result<FileIntegrity, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("无法读取模型文件: {error}"))?;
    Ok(FileIntegrity {
        size: metadata.len(),
        sha256: hash_file(path, Some(cancel)).await?,
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn digest_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn model_reads_cancel_queued_work_and_drain_accepted_operations() {
        async fn drain_reads(manager: &ModelManager) {
            // TaskTracker::wait requires a closed tracker. This barrier does
            // not cancel operations; reopen before submitting the next case.
            manager.inner.tasks.close();
            manager.inner.tasks.wait().await;
            manager.inner.tasks.reopen();
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            let root = std::env::temp_dir().join(format!("super-paste-rs-model-read-{}", uuid::Uuid::new_v4()));
            let manager = ModelManager::new(root.clone(), EventBus::new(16).expect("model events"), Handle::current())
                .expect("model manager");
            let serial = manager.inner.status_read.lock().await;
            let cancel = CancellationToken::new();
            let mut read = Box::pin(manager.list(&cancel));
            assert!(futures_util::poll!(read.as_mut()).is_pending());
            cancel.cancel();
            assert!(matches!(read.await, Err(crate::Error::Cancelled)), "queued cancellation must not wait for the read slot");

            let cancel = CancellationToken::new();
            let mut read = Box::pin(manager.list(&cancel));
            assert!(futures_util::poll!(read.as_mut()).is_pending());
            drop(read);
            drain_reads(&manager).await;
            assert!(manager.inner.tasks.is_empty(), "a dropped waiter cancels queued work");
            drop(serial);
            assert_eq!(manager.list(&CancellationToken::new()).await.expect("read after cancellation").len(), all_models().expect("catalog").len());
            let cancelled = CancellationToken::new(); cancelled.cancel();
            let model = super::super::catalog::default_model_id().expect("default model");
            assert!(matches!(manager.installed(model, &cancelled).await, Err(crate::Error::Cancelled)));

            // The oneshot represents a filesystem operation that is already
            // running and cannot be dropped until its completion is observed.
            for ending in 0..3 {
                let caller = CancellationToken::new();
                let requested = caller.clone(); let reader = manager.clone();
                let (entered, active) = tokio::sync::oneshot::channel();
                let (complete, completed) = tokio::sync::oneshot::channel();
                let waiter = tokio::spawn(async move {
                    reader.read(&requested, move |_, cancel| async move {
                        entered.send(cancel).map_err(|_| "read entry receiver dropped".to_owned())?;
                        completed.await.map_err(|_| "read completion sender dropped".to_owned())?;
                        Ok(())
                    }).await
                });
                let worker = active.await.expect("read started");
                let shutdown = match ending {
                    0 => { caller.cancel(); None }
                    1 => { waiter.abort(); None }
                    _ => { let owner = manager.clone(); Some(tokio::spawn(async move { owner.shutdown().await })) }
                };
                worker.cancelled().await;
                assert_eq!(manager.inner.tasks.len(), 1, "accepted work stays tracked during cancellation");
                if ending != 1 { assert!(!waiter.is_finished(), "cancellation must drain accepted work"); }
                if let Some(shutdown) = &shutdown { assert!(!shutdown.is_finished(), "shutdown must wait for physical completion"); }
                complete.send(()).expect("finish accepted read");
                if ending == 1 {
                    assert!(waiter.await.expect_err("aborted waiter").is_cancelled());
                } else {
                    assert!(matches!(waiter.await.expect("join read waiter"), Err(crate::Error::Cancelled)));
                }
                if let Some(shutdown) = shutdown { shutdown.await.expect("join shutdown"); }
                if ending != 2 { drain_reads(&manager).await; }
                assert!(manager.inner.tasks.is_empty());
            }
            assert!(matches!(manager.list(&CancellationToken::new()).await, Err(crate::Error::Cancelled)));
            assert!(!root.exists(), "read lifecycle checks do not create model data");
        }).await.expect("bounded model read lifecycle regression");
    }

    #[tokio::test]
    async fn installed_manifest_bounds_apply_to_status_and_hash_validation() {
        let root = std::env::temp_dir().join(format!("super-paste-rs-manifest-{}", uuid::Uuid::new_v4()));
        let manager = ModelManager::new(root.clone(), EventBus::new(16).expect("model event bus"), Handle::current())
            .expect("model manager");
        let model = get_model(super::super::catalog::default_model_id().expect("default model")).expect("catalog model");
        let directory = root.join(&model.id);
        std::fs::create_dir_all(&directory).expect("isolated model directory");
        let mut files = HashMap::new();
        for (role, name, bytes) in [
            ("det", "det.onnx", &b"detector"[..]),
            ("rec", "rec.onnx", &b"recognizer"[..]),
            ("dictionary", "dict.txt", &b"A\nB\n"[..]),
        ] {
            std::fs::write(directory.join(name), bytes).expect("model identity fixture");
            files.insert(role.to_owned(), FileIntegrity { size: bytes.len() as u64, sha256: digest_hex(&Sha256::digest(bytes)) });
        }
        let manifest = serde_json::to_vec(&ModelManifest {
            format: 1, model_id: model.id.clone(), revision: model_revision(model), files,
        }).expect("installed manifest");
        let path = directory.join("manifest.json");
        let cancel = CancellationToken::new();
        for (length, usable) in [(manifest.len(), true), (MAX_MANIFEST_BYTES as usize, true), (MAX_MANIFEST_BYTES as usize + 1, false)] {
            // Extra JSON whitespace keeps every input syntactically valid; the
            // oversized case must be rejected by admission, not JSON parsing.
            let mut padded = manifest.clone();
            padded.resize(length, b' ');
            std::fs::write(&path, padded).expect("write bounded manifest case");
            let statuses = manager.list(&CancellationToken::new()).await.expect("list model statuses");
            let status = statuses.iter().find(|status| status.id == model.id).expect("selected model status");
            assert_eq!(status.state, if usable { ModelState::Ready } else { ModelState::NotDownloaded });
            assert_eq!(manager.installed(&model.id, &cancel).await.expect("validate installed identity").is_some(), usable);
        }
        std::fs::write(&path, &manifest).expect("repair oversized manifest");
        assert!(manager.installed(&model.id, &cancel).await.expect("inspect repaired manifest").is_some());
        // Bounded metadata does not bypass the existing full file hash check.
        std::fs::write(directory.join("det.onnx"), b"corrupt!").expect("replace detector with same-length content");
        assert!(manager.installed(&model.id, &cancel).await.expect("inspect changed detector").is_none());
        std::fs::remove_file(&path).expect("remove repaired manifest");
        std::fs::create_dir(&path).expect("unreadable manifest target");
        assert!(manager.list(&CancellationToken::new()).await.is_err(), "file access failures must remain errors");
        manager.shutdown().await;
        std::fs::remove_dir_all(root).expect("remove isolated manifest fixtures");
    }

    #[tokio::test]
    async fn installation_completion_preserves_late_success_and_failure() {
        tokio::time::timeout(Duration::from_secs(5), async {
            for deadline in [false, true] {
                for failure in [false, true] {
                    let cancel = CancellationToken::new();
                    let task_cancel = cancel.clone();
                    let operation_cancel = cancel.clone();
                    let (entered, accepted) = tokio::sync::oneshot::channel();
                    let (finish, released) = tokio::sync::oneshot::channel();
                    let (expire, expired) = tokio::sync::oneshot::channel();
                    let expected = if failure { Err("模型目录替换失败".to_owned()) } else { Ok(()) };
                    let result = expected.clone();
                    let task = tokio::spawn(async move {
                        complete_install(async move {
                            check_cancelled(&operation_cancel)?;
                            entered.send(()).expect("signal accepted installation");
                            released.await.expect("release installation finalization");
                            result
                        }, &task_cancel, async move {
                            expired.await.expect("wait for installation deadline");
                        }).await
                    });
                    accepted.await.expect("reach installation finalization");
                    if deadline { expire.send(()).expect("expire installation deadline"); }
                    else { cancel.cancel(); }
                    cancel.cancelled().await;
                    assert!(!task.is_finished(), "accepted installation must drain before returning");
                    finish.send(()).expect("finish owned installation");
                    assert_eq!(task.await.expect("join installation"), expected,
                        "late cancellation or timeout must not replace the actual result");
                }
            }
        }).await.expect("bounded installation completion regression");
    }

    #[tokio::test]
    async fn installation_preparation_preserves_cancellation_reason() {
        tokio::time::timeout(Duration::from_secs(5), async {
            // A deadline that races with an existing caller cancellation must
            // not relabel that cancellation as a timeout.
            for (deadline, already_cancelled) in [(false, false), (true, false), (true, true)] {
                let cancel = CancellationToken::new();
                let task_cancel = cancel.clone();
                let operation_cancel = cancel.clone();
                let (entered, preparing) = tokio::sync::oneshot::channel();
                let (expire, expired) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(async move {
                    complete_install(async move {
                        entered.send(()).expect("signal cancellable preparation");
                        operation_cancel.cancelled().await;
                        check_cancelled(&operation_cancel)
                    }, &task_cancel, async move {
                        expired.await.expect("wait for preparation deadline");
                    }).await
                });
                preparing.await.expect("reach cancellable preparation");
                if !deadline || already_cancelled { cancel.cancel(); }
                if deadline { expire.send(()).expect("expire preparation deadline"); }
                let expected = if deadline && !already_cancelled { "模型下载超时" } else { DOWNLOAD_CANCELLED };
                assert_eq!(task.await.expect("join cancelled preparation"), Err(expected.to_owned()));
            }
        }).await.expect("bounded installation preparation regression");
    }

    // Exercise the embedded catalog through the actual startup path. No
    // network request, model download or application data directory is used.
    #[tokio::test]
    async fn bundled_catalog_initializes_model_manager() {
        let root = std::env::temp_dir().join(format!("super-paste-rs-models-{}", uuid::Uuid::new_v4()));
        let manager = ModelManager::new(
            root.clone(),
            EventBus::new(16).expect("create model event bus"),
            Handle::current(),
        ).expect("initialize model manager with the embedded catalog");
        manager.cleanup_staging().await.expect("prepare empty model directory");
        let statuses = manager.list(&CancellationToken::new()).await.expect("list all bundled models");
        assert_eq!(statuses.len(), all_models().expect("read bundled models").len());
        assert!(statuses.iter().all(|status| status.state == ModelState::NotDownloaded));
        let default = super::super::catalog::default_model_id().expect("resolve default model");
        assert!(statuses.iter().any(|status| status.id == default));
        get_model("ppocr-v4").expect("resolve detector fallback for recognition-only archives");
        manager.shutdown().await;
        assert!(std::fs::read_dir(&root).expect("read initialized model directory").next().is_none(),
            "startup must leave model weights uninstalled");
        std::fs::remove_dir(&root).expect("remove isolated empty model directory");
    }
}
