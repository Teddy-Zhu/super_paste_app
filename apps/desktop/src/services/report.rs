use std::{io::{BufWriter, Write}, path::{Path, PathBuf}, sync::Arc};
use serde::Serialize;
use super_paste_core::{Error, Result};
use super_paste_format::{APPLICATION_ID, CredentialId, SCHEMA_VERSION, ColorMode, OverlayPosition, OcrMode, ShortcutState, SyncState};
use super::Services;

#[derive(Serialize)]
struct Report {
    schema_version: u32, generated_at_ms: Option<i64>, application: Application,
    configuration: SafeConfiguration, storage: StorageReport, access: AccessReport,
    shortcut: ShortcutReport, mcp: McpReport, sync: SyncReport, ocr: OcrReport,
    image_memory: ImageMemory, preview_live_bytes: u64, logs: Vec<crate::diagnostics::LogEntry>,
}
#[derive(Serialize)]
struct Export { #[serde(flatten)] report: Report, process_memory: ProcessMemory }
#[derive(Serialize)]
struct Application { id: &'static str, version: &'static str, os: &'static str, arch: &'static str, compositor: String }
#[derive(Serialize)]
struct SafeConfiguration {
    revision: u64, auto_start_requested: bool, silent_start: bool, clipboard_listening: bool,
    color_mode: ColorMode, overlay_position: OverlayPosition, max_history_size: i64, max_keep_time: i64,
    ocr_mode: OcrMode, webdav_password_set: bool, openai_api_key_set: bool, claude_api_key_set: bool, mcp_api_key_set: bool,
}
#[derive(Serialize)]
struct StorageReport { schema_version: i32, counts: Counts, database: FileSize, wal: FileSize }
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum Counts { Ready { records: u64, tags: u64, images: u64, encoded_image_bytes: u64 }, Unavailable { category: &'static str } }
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum FileSize { Ready { bytes: u64 }, Missing, Unavailable { kind: String } }
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum AccessReport {
    Ready { clipboard_listening: bool, suspended: bool, clipboard_error_present: bool, source_available: bool, source_error_present: bool, source_frozen: bool, source_listener: &'static str, source_listener_error_present: bool },
    Unavailable { category: &'static str },
}
#[derive(Serialize)]
struct ShortcutReport { state: ShortcutState, portal: bool, error_present: bool }
#[derive(Serialize)]
struct McpReport { enabled: bool, running: bool, port: u16, authentication_enabled: bool, error_present: bool }
#[derive(Serialize)]
struct SyncReport { enabled: bool, running: bool, state: &'static str, last_sync_time: i64, next_sync_time: i64, restart_required: bool, error_present: bool }
#[derive(Serialize)]
struct OcrReport { revision: u64, queued: usize, running: usize, direct: usize, maintenance: usize, local_model_loaded: bool, suspended: bool, closing: bool }
#[derive(Serialize)]
struct ImageMemory { encoded_bytes: u64, pixel_bytes: u64, scratch_bytes: u64 }
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum ProcessMemory { Ready { resident_bytes: u64, virtual_bytes: u64, pid: u32 }, Unavailable { category: &'static str } }

impl Services {
    /// The existing service shutdown gate admits the whole export atomically;
    /// its task drains before image release and the final SQLite checkpoint.
    pub fn export_diagnostics(self: &Arc<Self>, path: PathBuf, compositor: String) -> Result<tokio::task::JoinHandle<Result<PathBuf>>> {
        let closing = self.shutdown.lock().map_err(|_| Error::Task("退出状态锁损坏".into()))?;
        if closing.is_some() { return Err(Error::Cancelled); }
        let permit = self.export_slots.clone().try_acquire_owned().map_err(|_| Error::ResourceLimit("诊断导出正在执行".into()))?;
        let service = self.clone();
        Ok(self.background.spawn_on(async move {
            let _permit = permit;
            let configuration = service.configuration.snapshot();
            let settings = &configuration.snapshot.value.settings;
            let sync = service.sync.snapshot(); let mcp = service.mcp.snapshot(); let shortcut = service.shortcuts.snapshot();
            let access = match service.access.status() {
                Ok(access) => {
                    use super_paste_platform::source::SourceListener;
                    let source = access.source;
                    AccessReport::Ready { clipboard_listening: access.clipboard_listening, suspended: access.suspended, clipboard_error_present: access.clipboard_error.is_some(),
                        source_available: source.as_ref().is_some_and(|source| source.available),
                        source_error_present: access.source_error.is_some() || source.as_ref().is_some_and(|source| source.error.is_some()),
                        source_frozen: source.as_ref().is_some_and(|source| source.frozen),
                        source_listener: source.as_ref().map_or("unavailable", |source| match source.listener { SourceListener::Starting => "starting", SourceListener::Events => "events", SourceListener::Kwin => "kwin", SourceListener::Polling => "polling" }),
                        source_listener_error_present: source.as_ref().is_some_and(|source| source.listener_error.is_some()) }
                }
                Err(error) => AccessReport::Unavailable { category: category(&error) },
            };
            let refs = &configuration.credentials;
            let safe = SafeConfiguration { revision: configuration.snapshot.revision, auto_start_requested: settings.auto_start,
                silent_start: settings.silent_start, clipboard_listening: settings.clipboard_listening, color_mode: settings.color_mode,
                overlay_position: settings.overlay_position, max_history_size: settings.max_history_size, max_keep_time: settings.max_keep_time,
                ocr_mode: settings.ocr_mode, webdav_password_set: refs.contains_key(&CredentialId::WebDavPassword),
                openai_api_key_set: refs.contains_key(&CredentialId::OpenaiApiKey), claude_api_key_set: refs.contains_key(&CredentialId::ClaudeApiKey), mcp_api_key_set: refs.contains_key(&CredentialId::McpApiKey) };
            let (counts, ocr) = tokio::join!(service.storage.diagnostic_counts(), service.ocr.diagnostics());
            let counts = match counts { Ok(counts) => Counts::Ready { records: counts.records, tags: counts.tags, images: counts.images, encoded_image_bytes: counts.encoded_image_bytes }, Err(error) => Counts::Unavailable { category: category(&error) } };
            let mut wal = service.paths.database.as_os_str().to_os_string(); wal.push("-wal");
            let (database, wal) = tokio::join!(file_size(&service.paths.database), file_size(Path::new(&wal)));
            let memory = service.images.memory();
            let report = Report { schema_version: 1, generated_at_ms: super_paste_core::now_ms().ok(),
                application: Application { id: APPLICATION_ID, version: env!("CARGO_PKG_VERSION"), os: std::env::consts::OS, arch: std::env::consts::ARCH, compositor },
                configuration: safe, storage: StorageReport { schema_version: SCHEMA_VERSION, counts, database, wal }, access,
                shortcut: ShortcutReport { state: shortcut.state, portal: service.shortcuts.uses_portal(), error_present: shortcut.error.is_some() },
                mcp: McpReport { enabled: configuration.snapshot.value.mcp.enabled, running: mcp.running, port: configuration.snapshot.value.mcp.port, authentication_enabled: configuration.snapshot.value.mcp.auth_enabled, error_present: mcp.error.is_some() },
                sync: SyncReport { enabled: configuration.snapshot.value.sync.enabled, running: sync.is_running, state: match sync.state { SyncState::Idle => "idle", SyncState::Syncing => "syncing", SyncState::Success => "success", SyncState::Error => "error" }, last_sync_time: sync.last_sync_time, next_sync_time: sync.next_sync_time, restart_required: sync.restart_required, error_present: sync.error.is_some() },
                ocr: OcrReport { revision: ocr.revision, queued: ocr.queued, running: ocr.running, direct: ocr.direct, maintenance: ocr.maintenance, local_model_loaded: ocr.local_model_loaded, suspended: ocr.suspended, closing: ocr.closing },
                image_memory: ImageMemory { encoded_bytes: memory.encoded, pixel_bytes: memory.pixels, scratch_bytes: memory.scratch },
                preview_live_bytes: service.previews.live_bytes(), logs: service.log.snapshot() };
            service.runtime.spawn_blocking(move || write_report(path, report)).await.map_err(Error::from)?
        }, &self.runtime))
    }
}
async fn file_size(path: &Path) -> FileSize {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => FileSize::Ready { bytes: metadata.len() }, Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileSize::Missing,
        Err(error) => FileSize::Unavailable { kind: format!("{:?}", error.kind()) },
    }
}
fn process_memory() -> ProcessMemory {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let Ok(pid) = sysinfo::get_current_pid() else { return ProcessMemory::Unavailable { category: "current-pid-unavailable" }; };
    let Ok(mut system) = System::new() else { return ProcessMemory::Unavailable { category: "system-initialization-failed" }; };
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, ProcessRefreshKind::nothing().with_memory());
    match system.process(pid) { Some(process) => ProcessMemory::Ready { resident_bytes: process.memory(), virtual_bytes: process.virtual_memory(), pid: pid.as_u32() }, None => ProcessMemory::Unavailable { category: "process-unavailable" } }
}
fn write_report(path: PathBuf, report: Report) -> Result<PathBuf> {
    if !path.is_absolute() { return Err(Error::Invalid("诊断文件需要完整保存路径".into())); }
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).ok_or_else(|| Error::Invalid("诊断文件需要完整保存路径".into()))?;
    let report = Export { report, process_memory: process_memory() };
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    { let mut writer = BufWriter::new(file.as_file_mut()); serde_json::to_writer_pretty(&mut writer, &report)?; writer.write_all(b"\n")?; writer.flush()?; }
    file.as_file().sync_all()?; file.persist(&path).map_err(|error| Error::from(error.error))?;
    Ok(path)
}
// Raw error text is intentionally excluded: OS/network errors can embed paths
// and remote endpoints. The exported schema contains only these stable classes.
fn category(error: &Error) -> &'static str {
    match error {
        Error::Invalid(_) => "invalid", Error::NotFound => "not-found", Error::Cancelled => "cancelled", Error::ResourceLimit(_) => "resource-limit",
        Error::Database(_) => "database", Error::Io(_) => "io", Error::Platform(_) => "platform", Error::Image(_) => "image", Error::Task(_) => "task", Error::Conflict(_) => "conflict",
        Error::CredentialUnavailable => "credential-unavailable", Error::CredentialOperation => "credential-operation", Error::CredentialEncoding => "credential-encoding",
    }
}
