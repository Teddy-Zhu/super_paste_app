use super::catalog::{
    ModelInfo, ModelStage, ModelState, OcrModelStatus, all_models, get_model, model_revision,
    model_status, release_url,
};
use crate::events::EventSink;
use futures_util::{StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use zip::ZipArchive;

const MAX_ARCHIVE_BYTES: u64 = 160_000_000;
const MAX_EXTRACTED_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_TOTAL_BYTES: u64 = 768 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 128;
const MAX_DICTIONARY_BYTES: u64 = 16 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

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
pub(super) struct InstalledModel {
    pub info: ModelInfo,
    pub detector: PathBuf,
    pub recognizer: PathBuf,
    pub dictionary: PathBuf,
}

#[derive(Clone)]
pub(super) struct ModelManager {
    inner: Arc<ModelManagerInner>,
}

struct ModelManagerInner {
    root: PathBuf,
    client: reqwest::Client,
    events: EventSink,
    statuses: Mutex<HashMap<String, OcrModelStatus>>,
    downloads: Mutex<HashMap<String, DownloadTask>>,
    maintenance: Mutex<HashSet<String>>,
}

#[derive(Clone)]
struct DownloadTask {
    generation: String,
    cancel: CancellationToken,
    result: watch::Receiver<Option<Result<InstalledModel, String>>>,
}

impl ModelManager {
    pub(super) fn new(root: PathBuf, events: EventSink) -> Result<Self, String> {
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
                downloads: Mutex::new(HashMap::new()),
                maintenance: Mutex::new(HashSet::new()),
            }),
        })
    }

    pub(super) async fn cleanup_staging(&self) -> Result<(), String> {
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

    pub(super) async fn list(&self) -> Result<Vec<OcrModelStatus>, String> {
        let in_progress = self.inner.statuses.lock().await.clone();
        let mut statuses = Vec::with_capacity(all_models().len());
        for model in all_models() {
            if in_progress
                .get(&model.id)
                .is_some_and(|status| status.state == ModelState::Downloading)
            {
                statuses.push(in_progress[&model.id].clone());
                continue;
            }
            if self.installed(model, false).await?.is_some() {
                let mut status = model_status(&model.id, ModelState::Ready, None);
                status.received_bytes = model.size;
                statuses.push(status);
            } else {
                statuses.push(
                    in_progress
                        .get(&model.id)
                        .filter(|status| status.state == ModelState::Error)
                        .cloned()
                        .unwrap_or_else(|| {
                            model_status(&model.id, ModelState::NotDownloaded, None)
                        }),
                );
            }
        }
        Ok(statuses)
    }

    pub(super) async fn diagnostics(&self) -> Value {
        let downloads = self.inner.downloads.lock().await.len();
        let (statuses, status_unavailable) = match self.list().await {
            Ok(statuses) => (statuses, false),
            Err(_) => (Vec::new(), true),
        };
        let models = statuses
            .into_iter()
            .map(|status| {
                json!({
                    "id":status.id,
                    "state":status.state,
                    "stage":status.stage,
                    "receivedBytes":status.received_bytes,
                    "totalBytes":status.total_bytes,
                })
            })
            .collect::<Vec<_>>();
        json!({"downloads":downloads,"models":models,"statusUnavailable":status_unavailable})
    }

    pub(super) async fn download(&self, id: &str) -> Result<InstalledModel, String> {
        let model = get_model(id)?.clone();
        let maintenance = self.inner.maintenance.lock().await;
        if maintenance.contains(id) {
            return Err("模型正在维护，请稍后重试".to_owned());
        }
        let generation = uuid::Uuid::new_v4().to_string();
        let (task, launch, sender) = {
            let mut downloads = self.inner.downloads.lock().await;
            if let Some(task) = downloads.get(id) {
                (task.clone(), false, None)
            } else {
                let cancel = CancellationToken::new();
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
            tokio::spawn(async move {
                let result = manager.install(model, cancel, false).await;
                let _ = sender.send(Some(result.clone()));
                let mut downloads = manager.inner.downloads.lock().await;
                if downloads
                    .get(&model_id)
                    .is_some_and(|task| task.generation == task_generation)
                {
                    downloads.remove(&model_id);
                }
            });
        }
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

    pub(super) async fn cancel_download(&self, id: &str) -> Result<(), String> {
        get_model(id)?;
        if let Some(task) = self.inner.downloads.lock().await.get(id) {
            task.cancel.cancel();
        }
        Ok(())
    }

    pub(super) async fn remove(&self, id: &str) -> Result<(), String> {
        let model = get_model(id)?;
        self.maintain(id, async {
            let path = self.inner.root.join(&model.id);
            if tokio::fs::try_exists(&path)
                .await
                .map_err(|error| format!("无法检查 OCR 模型目录: {error}"))?
            {
                tokio::fs::remove_dir_all(&path)
                    .await
                    .map_err(|error| format!("删除 OCR 模型失败: {error}"))?;
            }
            self.publish(model_status(id, ModelState::NotDownloaded, None))
                .await;
            Ok(())
        })
        .await
    }

    pub(super) async fn redownload(&self, id: &str) -> Result<InstalledModel, String> {
        let model = get_model(id)?.clone();
        self.maintain(id, async {
            self.install(model, CancellationToken::new(), true).await
        })
        .await
    }

    pub(super) async fn cancel_all(&self) {
        let downloads = self.inner.downloads.lock().await;
        for task in downloads.values() {
            task.cancel.cancel();
        }
    }

    async fn maintain<T, F>(&self, id: &str, operation: F) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
    {
        {
            let mut maintenance = self.inner.maintenance.lock().await;
            if !maintenance.insert(id.to_owned()) {
                return Err("模型正在维护，请稍后重试".to_owned());
            }
        }
        let task = self.inner.downloads.lock().await.get(id).cloned();
        if let Some(task) = task {
            task.cancel.cancel();
            let mut receiver = task.result;
            while receiver.borrow().is_none() {
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        }
        let result = operation.await;
        self.inner.maintenance.lock().await.remove(id);
        result
    }

    pub(super) async fn installed(
        &self,
        model: &ModelInfo,
        verify_hashes: bool,
    ) -> Result<Option<InstalledModel>, String> {
        let directory = self.inner.root.join(&model.id);
        let manifest_path = directory.join("manifest.json");
        let bytes = match tokio::fs::read(&manifest_path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("无法读取 OCR 模型清单: {error}")),
        };
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
            let Some(expected) = manifest.files.get(role) else {
                return Ok(None);
            };
            if expected.size == 0 || !is_sha256(&expected.sha256) {
                return Ok(None);
            }
            let path = directory.join(name);
            let metadata = match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.is_file() && metadata.len() == expected.size => metadata,
                Ok(_) => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(format!("无法读取 OCR 模型文件: {error}")),
            };
            if name == "dict.txt" && metadata.len() > MAX_DICTIONARY_BYTES {
                return Ok(None);
            }
            if verify_hashes && hash_file(&path).await? != expected.sha256 {
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
                &model.id,
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
        tokio::pin!(install_deadline);
        let mut staging_path = None;
        let result = {
            let operation = async {
                if let Some(installed) = self.installed(model, true).await? {
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
                        _ = cancel.cancelled() => return Err("模型下载已取消".to_owned()),
                        result = self.download("ppocr-v4") => result?,
                    };
                    check_cancelled(cancel)?;
                    let target = staging.join("det.onnx");
                    tokio::fs::copy(&detector_model.detector, &target)
                        .await
                        .map_err(|error| format!("无法准备共享文字检测模型: {error}"))?;
                    files.insert("det".to_owned(), file_integrity(&target).await?);
                }
                if !files.contains_key("rec") || !files.contains_key("dictionary") {
                    let _ = tokio::fs::remove_dir_all(&staging).await;
                    return Err("下载包中缺少识别模型或字典".to_owned());
                }
                let dictionary = tokio::fs::read_to_string(staging.join("dict.txt"))
                    .await
                    .map_err(|error| format!("OCR 字典不是有效 UTF-8: {error}"))?;
                if dictionary.trim().is_empty() {
                    let _ = tokio::fs::remove_dir_all(&staging).await;
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
                        let _ = tokio::fs::rename(&backup, &directory).await;
                    }
                    let _ = tokio::fs::remove_dir_all(&staging).await;
                    return Err(format!("无法安装 OCR 模型: {error}"));
                }
                if had_previous {
                    tokio::fs::remove_dir_all(&backup)
                        .await
                        .map_err(|error| format!("OCR 模型已安装，但无法清理旧版本: {error}"))?;
                }
                self.installed(model, true)
                    .await?
                    .ok_or_else(|| "OCR 模型安装后校验失败".to_owned())
            };
            tokio::pin!(operation);
            tokio::select! {
                result = &mut operation => result,
                _ = cancel.cancelled() => {
                    let _ = operation.await;
                    Err("模型下载已取消".to_owned())
                },
                _ = &mut install_deadline => {
                    cancel.cancel();
                    let _ = operation.await;
                    Err("模型下载超时".to_owned())
                },
            }
        };
        if result.is_err() {
            if let Some(staging) = staging_path {
                let _ = tokio::fs::remove_dir_all(staging).await;
            }
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
        let request = self.inner.client.get(release_url(model)).send();
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err("模型下载已取消".to_owned()),
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
            _ = cancel.cancelled() => return Err("模型下载已取消".to_owned()),
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
        if let Some(expected) = model.sha256.as_deref() {
            let actual = digest_hex(hasher.finalize().as_slice());
            if actual != expected {
                return Err("模型下载校验失败，请重试".to_owned());
            }
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
                return Err("模型下载已取消".to_owned());
            },
        };
        let _ = tokio::fs::remove_file(archive).await;
        Ok(files)
    }

    async fn publish(&self, status: OcrModelStatus) {
        self.inner
            .statuses
            .lock()
            .await
            .insert(status.id.clone(), status.clone());
        let payload = serde_json::to_value(status).unwrap_or(serde_json::Value::Null);
        (self.inner.events)("ocr:model-status", payload);
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
        Err("模型下载已取消".to_owned())
    } else {
        Ok(())
    }
}

async fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("无法读取模型文件: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
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

async fn file_integrity(path: &Path) -> Result<FileIntegrity, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("无法读取模型文件: {error}"))?;
    Ok(FileIntegrity {
        size: metadata.len(),
        sha256: hash_file(path).await?,
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
