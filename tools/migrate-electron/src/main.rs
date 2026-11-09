#![allow(dead_code)]

#[path = "../../../src-tauri/src/ocr/catalog.rs"]
mod catalog;
#[path = "../../../src-tauri/src/credentials/mod.rs"]
mod credentials;

use aes::Aes256;
use anyhow::{Context, Result, anyhow, bail};
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use chrono::{Local, TimeZone, Timelike};
use data_format::{OcrModelFile, OcrModelManifest};
use pbkdf2::pbkdf2_hmac;
use rusqlite::{Connection, OpenFlags, OptionalExtension, backup::Backup, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use sysinfo::System;
use tempfile::{TempDir, tempdir, tempdir_in};
use uuid::Uuid;

type Aes256CbcDec = cbc::Decryptor<Aes256>;
const LEGACY_CONFIG_KEY: &str = "super_paste_app_11";
const MARKER_ID: &str = "electron-v1";
const DATABASES: [&str; 3] = [
    "clipboard-history.db",
    "clipboard-statistics.db",
    "ai-ocr-usage.db",
];

#[derive(Debug)]
struct Options {
    from: PathBuf,
    to: PathBuf,
    dry_run: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MigrationReport {
    format_version: u32,
    source_fingerprint: String,
    dry_run: bool,
    missing_inputs: Vec<String>,
    history_source_records: u64,
    history_imported_records: u64,
    history_deduplicated_records: u64,
    id_mappings: BTreeMap<String, String>,
    apps_imported: u64,
    tags_imported: u64,
    tag_links_imported: u64,
    missing_app_references: u64,
    copy_statistics_rows: u64,
    copy_statistics_cleared_at: Option<i64>,
    ai_usage_aggregate_rows: u64,
    ai_usage_recent_rows: u64,
    ai_usage_cleared_at: Option<i64>,
    credential_fields_migrated: Vec<String>,
    credential_fields_would_migrate: Vec<String>,
    credential_fields_needing_input: Vec<String>,
    mcp_auth_enabled: bool,
    mcp_key_generated: bool,
    mcp_key_needs_generation: bool,
    mcp_client_connection_must_be_updated: bool,
    models_imported: Vec<String>,
    models_skipped: Vec<String>,
    integrity_checks: BTreeMap<String, String>,
}

struct Staging {
    directory: TempDir,
    target: PathBuf,
}

struct CredentialRollback {
    store: credentials::CredentialStore,
    references: Vec<String>,
    committed: bool,
}

impl CredentialRollback {
    fn new(store: credentials::CredentialStore) -> Self {
        Self {
            store,
            references: Vec::new(),
            committed: false,
        }
    }
}

impl Drop for CredentialRollback {
    fn drop(&mut self) {
        if !self.committed {
            for reference in self.references.iter().rev() {
                let _ = self.store.delete_secret(reference);
            }
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("迁移失败: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let options = parse_args()?;
    if process_is_running()? {
        bail!("检测到旧版或新版 Super Paste 正在运行，请先退出两个应用再迁移");
    }
    let from = options
        .from
        .canonicalize()
        .with_context(|| format!("旧版数据目录不存在: {}", options.from.display()))?;
    if !from.is_dir() {
        bail!("--from 必须是旧版 Electron 数据目录");
    }
    if !DATABASES.iter().any(|name| from.join(name).is_file())
        && !["config.json", "mcp.json"]
            .iter()
            .any(|name| from.join(name).is_file())
        && !from.join(data_format::OCR_MODELS_DIRECTORY).is_dir()
    {
        bail!("--from 中未找到 Electron 配置、数据库或 OCR 模型");
    }
    let target = normalize_target(&options.to)?;
    let fingerprint = source_fingerprint(&from)?;
    if let Some(completed) = completed_report(&target, &fingerprint)? {
        println!(
            "该来源已迁移完成：\n{}",
            serde_json::to_string_pretty(&completed)?
        );
        return Ok(());
    }
    ensure_target_is_empty(&target)?;

    let target_parent = target.parent().context("--to 必须有父目录")?;
    fs::create_dir_all(target_parent)?;
    let staging = Staging {
        directory: tempdir_in(target_parent).context("无法创建目标旁暂存目录")?,
        target,
    };
    let snapshots_directory = tempdir().context("无法创建 SQLite 快照临时目录")?;
    let snapshots = snapshot_databases(&from, snapshots_directory.path())?;

    let mut report = MigrationReport {
        format_version: data_format::FORMAT_VERSION,
        source_fingerprint: fingerprint.clone(),
        dry_run: options.dry_run,
        ..MigrationReport::default()
    };
    for name in DATABASES {
        if !from.join(name).is_file() {
            report.missing_inputs.push(name.to_owned());
        }
    }
    for name in ["config.json", "mcp.json"] {
        if !from.join(name).is_file() {
            report.missing_inputs.push(name.to_owned());
        }
    }

    let db_path = staging.directory.path().join(data_format::DATABASE_FILE);
    let mut target_db = Connection::open(&db_path).context("无法创建新格式数据库")?;
    target_db.pragma_update(None, "foreign_keys", "ON")?;
    data_format::initialize_schema(&target_db).context("无法初始化新格式数据库 schema")?;

    let defaults = data_format::default_preferences();
    let old_config = read_config(&from.join("config.json"))?;
    let mut preferences = merge_preferences(defaults, old_config.as_ref())?;
    let credentials = credentials::CredentialStore::new();
    let mut credential_rollback = CredentialRollback::new(credentials);
    migrate_secret(
        &credentials,
        &mut credential_rollback.references,
        &mut report,
        &mut preferences,
        "settings.ocrAIConfigs.openai.apiKey",
        !options.dry_run,
    );
    migrate_secret(
        &credentials,
        &mut credential_rollback.references,
        &mut report,
        &mut preferences,
        "settings.ocrAIConfigs.claude.apiKey",
        !options.dry_run,
    );
    migrate_secret(
        &credentials,
        &mut credential_rollback.references,
        &mut report,
        &mut preferences,
        "sync.params.webdav.password",
        !options.dry_run,
    );
    insert_preferences(&target_db, &preferences)?;
    report.mcp_auth_enabled = read_mcp_auth(&from.join("mcp.json"))?;
    let mcp_key = if options.dry_run {
        None
    } else {
        let generated = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        match credentials.reference_for_new_secret(&generated) {
            Ok(reference) => {
                credential_rollback.references.push(reference.clone());
                report.mcp_key_generated = true;
                Some(reference)
            }
            Err(_) => {
                report.mcp_key_needs_generation = true;
                None
            }
        }
    };
    if options.dry_run {
        report.mcp_key_needs_generation = true;
    }
    let mut mcp = json!({
        "enabled": false,
        "port": data_format::DEFAULT_MCP_PORT,
        "authEnabled": report.mcp_auth_enabled,
    });
    if let Some(reference) = mcp_key {
        mcp["apiKeyRef"] = Value::String(reference);
    }
    target_db.execute(
        "INSERT INTO app_meta(key,value_json) VALUES('mcp.settings',?1)",
        [serde_json::to_string(&mcp)?],
    )?;
    report.mcp_client_connection_must_be_updated = true;

    if let Some(path) = snapshots.get(DATABASES[0]) {
        import_history(&mut target_db, path, &mut report)?;
    }
    if let Some(path) = snapshots.get(DATABASES[1]) {
        import_copy_statistics(&mut target_db, path, &mut report)?;
    }
    if let Some(path) = snapshots.get(DATABASES[2]) {
        import_ai_usage(&mut target_db, path, &mut report)?;
    }
    import_models(
        &from.join(data_format::OCR_MODELS_DIRECTORY),
        staging.directory.path(),
        &mut report,
    )?;
    verify_database(&target_db, &report)?;

    let current_fingerprint = source_fingerprint(&from)?;
    if current_fingerprint != fingerprint {
        bail!("迁移过程中旧版数据发生变化，请保持两版退出后重新执行");
    }
    report
        .integrity_checks
        .insert("sourceReadOnly".to_owned(), "通过".to_owned());
    report
        .integrity_checks
        .insert("sourceStable".to_owned(), "通过".to_owned());
    report
        .integrity_checks
        .insert("targetForeignKeys".to_owned(), "通过".to_owned());
    target_db.execute(
        "INSERT INTO migration_markers(id,completed_at,source_fingerprint,report_json) VALUES(?1,?2,?3,?4)",
        params![MARKER_ID, now_ms(), fingerprint, serde_json::to_string(&report)?],
    )?;
    target_db.flush_prepared_statement_cache();
    drop(target_db);
    write_report(
        &staging.directory.path().join("migration-report.json"),
        &report,
    )?;

    if options.dry_run {
        println!(
            "Dry-run 完成，未写入目标目录或系统凭据库：\n{}",
            serde_json::to_string_pretty(&report)?
        );
        return Ok(());
    }
    staging.commit()?;
    credential_rollback.committed = true;
    println!("迁移完成：\n{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn parse_args() -> Result<Options> {
    let mut from = None;
    let mut to = None;
    let mut dry_run = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" => from = Some(PathBuf::from(args.next().context("--from 缺少目录")?)),
            "--to" => to = Some(PathBuf::from(args.next().context("--to 缺少目录")?)),
            "--dry-run" => dry_run = true,
            "--help" | "-h" => {
                println!(
                    "用法: migrate-electron --from <旧版Electron数据目录> --to <新应用数据目录> [--dry-run]\n\n要求两版应用退出。迁移源只读，目标必须不存在或为空。--dry-run执行转换校验但不写最终目标或系统凭据库。"
                );
                std::process::exit(0);
            }
            unknown => bail!("未知参数: {unknown}"),
        }
    }
    Ok(Options {
        from: from.context("必须提供 --from")?,
        to: to.context("必须提供 --to")?,
        dry_run,
    })
}

fn process_is_running() -> Result<bool> {
    let mut system = System::new_all().map_err(|error| anyhow!("无法枚举运行中的进程: {error}"))?;
    system.refresh_all();
    for process in system.processes().values() {
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        let executable = process
            .exe()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if is_application_name(&name) || is_application_name(&executable) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_application_name(name: &str) -> bool {
    let name = name.trim_end_matches(".exe");
    name == "super_paste_app"
        || name == "super paste app"
        || name == "super_paste"
        || name == "super paste"
}

fn normalize_target(target: &Path) -> Result<PathBuf> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = if parent.exists() {
        parent.canonicalize()?
    } else {
        fs::create_dir_all(parent)?;
        parent.canonicalize()?
    };
    let name = target.file_name().context("--to 必须指向数据目录")?;
    let target = parent.join(name);
    if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!("目标路径不能是符号链接");
    }
    Ok(target)
}

fn ensure_target_is_empty(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(target)?.file_type().is_symlink() {
        bail!("目标路径不能是符号链接");
    }
    if !target.is_dir() {
        bail!("目标路径已存在且不是目录");
    }
    if fs::read_dir(target)?.next().is_some() {
        bail!("目标目录不为空，拒绝合并或覆盖新应用数据");
    }
    Ok(())
}

fn completed_report(target: &Path, fingerprint: &str) -> Result<Option<Value>> {
    let path = target.join(data_format::DATABASE_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("无法读取现存目标数据库")?;
    if !table_exists(&connection, "migration_markers")? {
        return Ok(None);
    }
    let stored = connection
        .query_row(
            "SELECT source_fingerprint,report_json FROM migration_markers WHERE id=?1",
            [MARKER_ID],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    match stored {
        Some((source, report)) if source == fingerprint => Ok(Some(serde_json::from_str(&report)?)),
        Some(_) => bail!("此新数据目录已由不同来源迁移，请使用空目录；禁止增量覆盖"),
        None => Ok(None),
    }
}

impl Staging {
    fn commit(&self) -> Result<()> {
        if self.target.exists() {
            let backup = self
                .target
                .with_file_name(format!(".empty-target-{}", Uuid::new_v4()));
            fs::rename(&self.target, &backup).context("无法暂存空目标目录")?;
            if let Err(error) = fs::rename(self.directory.path(), &self.target) {
                let _ = fs::rename(&backup, &self.target);
                return Err(error).context("无法原子提交迁移数据");
            }
            let _ = fs::remove_dir(backup);
        } else {
            fs::rename(self.directory.path(), &self.target).context("无法原子提交迁移数据")?;
        }
        Ok(())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn write_report(path: &Path, report: &MigrationReport) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(report)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn source_fingerprint(source: &Path) -> Result<String> {
    let mut files = Vec::new();
    for name in DATABASES.into_iter().chain(["config.json", "mcp.json"]) {
        let path = source.join(name);
        if path.exists() {
            add_regular_file(source, &path, &mut files)?;
        }
        if name.ends_with(".db") {
            for suffix in ["-wal", "-shm"] {
                let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
                if sidecar.exists() {
                    add_regular_file(source, &sidecar, &mut files)?;
                }
            }
        }
    }
    let models = source.join(data_format::OCR_MODELS_DIRECTORY);
    if models.exists() {
        collect_tree(source, &models, &mut files)?;
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, path) in files {
        digest.update((relative.len() as u64).to_be_bytes());
        digest.update(relative.as_bytes());
        let mut file = fs::File::open(&path)
            .with_context(|| format!("无法读取迁移输入 {}", path.display()))?;
        let length = file
            .metadata()
            .with_context(|| format!("无法读取迁移输入大小 {}", path.display()))?
            .len();
        digest.update(length.to_be_bytes());
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .with_context(|| format!("无法读取迁移输入 {}", path.display()))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
    }
    Ok(hex_lower(&digest.finalize()))
}

fn collect_tree(root: &Path, directory: &Path, files: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("模型数据目录不允许包含符号链接: {}", path.display());
        }
        if metadata.is_dir() {
            collect_tree(root, &path, files)?;
        } else if metadata.is_file() {
            add_regular_file(root, &path, files)?;
        }
    }
    Ok(())
}

fn add_regular_file(root: &Path, path: &Path, files: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("迁移输入必须是普通文件: {}", path.display());
    }
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("迁移输入超出源目录: {}", path.display()))?
        .to_string_lossy()
        .replace('\\', "/");
    files.push((relative, path.to_owned()));
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

fn snapshot_databases(source: &Path, snapshot_dir: &Path) -> Result<HashMap<String, PathBuf>> {
    let mut snapshots = HashMap::new();
    for name in DATABASES {
        let source_path = source.join(name);
        if !source_path.exists() {
            continue;
        }
        let input_directory = snapshot_dir.join(format!(".{name}-input"));
        fs::create_dir(&input_directory)?;
        let input_path = input_directory.join(name);
        copy_database_input(&source_path, &input_path)?;

        let source_wal = sqlite_sidecar_path(&source_path, "-wal");
        match fs::symlink_metadata(&source_wal) {
            Ok(metadata) if metadata.file_type().is_file() => {
                copy_database_input(&source_wal, &sqlite_sidecar_path(&input_path, "-wal"))?;
            }
            Ok(_) => bail!("SQLite WAL 输入必须是普通文件: {}", source_wal.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法读取 SQLite WAL {}", source_wal.display()));
            }
        }

        // SQLite may rebuild this derived index beside the temporary copy.
        let destination_path = snapshot_dir.join(format!("{name}.snapshot"));
        let source_connection = Connection::open_with_flags(
            &input_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("无法以只读方式打开临时数据库副本 {name}"))?;
        source_connection.busy_timeout(Duration::from_secs(5))?;
        let mut destination_connection = Connection::open(&destination_path)?;
        {
            let backup = Backup::new(&source_connection, &mut destination_connection)
                .with_context(|| format!("无法创建 {name} 的 SQLite 一致性快照"))?;
            backup
                .run_to_completion(32, Duration::from_millis(100), None)
                .with_context(|| format!("无法备份 {name}，源数据库可能仍在使用"))?;
        }
        let integrity: String =
            destination_connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            bail!("旧数据库 {name} 一致性检查失败: {integrity}");
        }
        snapshots.insert(name.to_owned(), destination_path);
    }
    Ok(snapshots)
}

fn copy_database_input(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("无法读取 SQLite 迁移输入 {}", source.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("SQLite 迁移输入必须是普通文件: {}", source.display());
    }
    fs::copy(source, destination)
        .with_context(|| format!("无法复制 SQLite 迁移输入 {}", source.display()))?;
    Ok(())
}

fn sqlite_sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn read_config(path: &Path) -> Result<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).context("无法读取旧配置文件")?;
    let value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(_) => {
            let plaintext =
                decrypt_legacy_config(&bytes).context("无法解密旧配置；源数据保持不变")?;
            serde_json::from_slice(&plaintext).context("旧配置解密后不是有效 JSON")?
        }
    };
    if !value.is_object() {
        bail!("旧配置根节点格式不受支持");
    }
    Ok(Some(value))
}

fn decrypt_legacy_config(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() <= 17 || bytes[16] != b':' {
        bail!("旧配置文件不是有效的 conf AES-256-CBC 加密数据");
    }
    let iv = &bytes[..16];
    let salt = String::from_utf8_lossy(iv);
    let mut key = [0_u8; 32];
    pbkdf2_hmac::<Sha512>(
        LEGACY_CONFIG_KEY.as_bytes(),
        salt.as_bytes(),
        10_000,
        &mut key,
    );
    Aes256CbcDec::new_from_slices(&key, iv)
        .context("旧配置密钥或初始化向量无效")?
        .decrypt_padded_vec_mut::<Pkcs7>(&bytes[17..])
        .map_err(|_| anyhow!("旧配置解密校验失败"))
}

fn merge_preferences(mut defaults: Value, old: Option<&Value>) -> Result<Value> {
    let Some(old) = old else { return Ok(defaults) };
    for root in ["settings", "sync"] {
        if let Some(value) = old.get(root) {
            merge_known(
                defaults
                    .get_mut(root)
                    .ok_or_else(|| anyhow!("新默认配置缺少 {root}"))?,
                value,
                root,
            )?;
        }
    }
    Ok(defaults)
}

fn merge_known(target: &mut Value, source: &Value, path: &str) -> Result<()> {
    let target_object = target
        .as_object_mut()
        .ok_or_else(|| anyhow!("新配置结构无效: {path}"))?;
    let source_object = source
        .as_object()
        .ok_or_else(|| anyhow!("旧配置字段不是对象: {path}"))?;
    let keys = target_object.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let Some(source_value) = source_object.get(&key) else {
            continue;
        };
        let target_value = target_object
            .get_mut(&key)
            .expect("key came from this object");
        let field_path = format!("{path}.{key}");
        if target_value.is_object() {
            merge_known(target_value, source_value, &field_path)?;
        } else if same_json_kind(target_value, source_value) || source_value.is_null() {
            *target_value = source_value.clone();
        } else {
            bail!("旧配置字段类型不匹配: {field_path}");
        }
    }
    Ok(())
}

fn same_json_kind(left: &Value, right: &Value) -> bool {
    (left.is_boolean() && right.is_boolean())
        || (left.is_number() && right.is_number())
        || (left.is_string() && right.is_string())
        || (left.is_array() && right.is_array())
        || (left.is_object() && right.is_object())
}

fn value_at_mut<'a>(value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get_mut(segment)?;
    }
    Some(current)
}

fn migrate_secret(
    credentials: &credentials::CredentialStore,
    created: &mut Vec<String>,
    report: &mut MigrationReport,
    preferences: &mut Value,
    field: &str,
    persist: bool,
) {
    let Some(secret) = value_at_mut(preferences, field) else {
        return;
    };
    let Some(secret_text) = secret.as_str().map(str::to_owned) else {
        report
            .credential_fields_needing_input
            .push(field.to_owned());
        *secret = Value::String(String::new());
        return;
    };
    if secret_text.is_empty() {
        *secret = Value::String(String::new());
        return;
    }
    if !persist {
        *secret = Value::String(String::new());
        report
            .credential_fields_would_migrate
            .push(field.to_owned());
        return;
    }
    match credentials.reference_for_new_secret(&secret_text) {
        Ok(reference) => {
            *secret = Value::String(reference.clone());
            created.push(reference);
            report.credential_fields_migrated.push(field.to_owned());
        }
        Err(_) => {
            *secret = Value::String(String::new());
            report
                .credential_fields_needing_input
                .push(field.to_owned());
        }
    }
}

fn insert_preferences(connection: &Connection, preferences: &Value) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    let now = now_ms();
    for root in ["settings", "sync"] {
        let value = preferences.get(root).context("偏好设置缺少根字段")?;
        transaction.execute(
            "INSERT INTO app_settings(key,value_json,updated_at) VALUES(?1,?2,?3)",
            params![root, serde_json::to_string(value)?, now],
        )?;
        if let Some(fields) = value.as_object() {
            for field in fields.keys() {
                transaction.execute(
                    "INSERT OR IGNORE INTO config_field_versions(key,version) VALUES(?1,0)",
                    [format!("{root}.{field}")],
                )?;
            }
        }
    }
    transaction.commit()?;
    Ok(())
}

fn read_mcp_auth(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    let value: Value = serde_json::from_slice(&fs::read(path).context("无法读取旧 MCP 配置")?)
        .context("旧 MCP 配置格式无效")?;
    let config = value.get("config").unwrap_or(&value);
    match config.get("authEnabled") {
        Some(Value::Bool(enabled)) => Ok(*enabled),
        Some(_) => bail!("旧 MCP 鉴权开关不是布尔值"),
        None => Ok(true),
    }
}

fn import_history(
    target: &mut Connection,
    snapshot: &Path,
    report: &mut MigrationReport,
) -> Result<()> {
    let source = Connection::open_with_flags(snapshot, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if !table_exists(&source, "clipboard_history")? {
        report
            .integrity_checks
            .insert("historyDatabase".to_owned(), "无历史表".to_owned());
        return Ok(());
    }
    let transaction = target.transaction()?;
    let mut app_ids = HashMap::new();
    if table_exists(&source, "apps")? {
        let mut statement = source.prepare(
            "SELECT id,application,path,icon,created_at,updated_at FROM apps ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("application")?,
                row.get::<_, String>("path")?,
                row.get::<_, Option<String>>("icon")?,
                timestamp_column(row, "created_at", now_ms())?,
                timestamp_column(row, "updated_at", now_ms())?,
            ))
        })?;
        for row in rows {
            let (old_id, application, path, icon, created_at, updated_at) = row?;
            let existing = transaction
                .query_row(
                    "SELECT id FROM apps WHERE application=?1 AND path=?2",
                    params![application, path],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let id = if let Some(id) = existing {
                id
            } else {
                transaction.execute(
                    "INSERT INTO apps(id,application,path,icon,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",
                    params![old_id,application,path,icon,created_at,updated_at],
                )?;
                report.apps_imported += 1;
                old_id.clone()
            };
            app_ids.insert(old_id.clone(), id.clone());
            if id != old_id {
                report
                    .id_mappings
                    .insert(format!("app:{old_id}"), format!("app:{id}"));
            }
        }
    }

    if table_exists(&source, "tag_groups")? {
        let mut statement = source.prepare(
            "SELECT id,name,color,description,created_at,updated_at FROM tag_groups ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("name")?,
                row.get::<_, String>("color")?,
                row.get::<_, Option<String>>("description")?,
                timestamp_column(row, "created_at", now_ms())?,
                timestamp_column(row, "updated_at", now_ms())?,
            ))
        })?;
        for row in rows {
            let (id, name, color, description, created_at, updated_at) = row?;
            transaction.execute(
                "INSERT INTO tag_groups(id,name,color,description,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",
                params![id,name,color,description,created_at,updated_at],
            )?;
            report.tags_imported += 1;
        }
    }

    let mut record_ids = HashMap::<String, String>::new();
    let mut digest_ids = HashMap::<String, String>::new();
    let mut history_statement = source.prepare("SELECT content_id,content_text,content_html,content_image,content_rtf,content_files,content_ocr,primary_type,available_formats,timestamp,last_used,source_app,created_at,updated_at FROM clipboard_history ORDER BY content_id")?;
    let history_rows = history_statement.query_map([], |row| {
        Ok(LegacyHistory {
            id: row.get("content_id")?,
            text: row.get("content_text")?,
            html: row.get("content_html")?,
            image: row.get("content_image")?,
            rtf: row.get("content_rtf")?,
            files: row.get("content_files")?,
            ocr: row.get("content_ocr")?,
            primary_type: row.get("primary_type")?,
            formats: row.get("available_formats")?,
            timestamp: row.get("timestamp")?,
            last_used: row.get("last_used")?,
            source_app: row.get("source_app")?,
            created_at: timestamp_column(row, "created_at", row.get::<_, i64>("timestamp")?)?,
            updated_at: timestamp_column(row, "updated_at", row.get::<_, i64>("last_used")?)?,
        })
    })?;
    for record in history_rows {
        let record = record.context("读取旧历史记录失败")?;
        report.history_source_records += 1;
        let files: Option<Value> = record
            .files
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .with_context(|| format!("历史记录 {} 的文件列表 JSON 无效", record.id))?;
        if files.as_ref().is_some_and(|value| !value.is_array()) {
            bail!("历史记录 {} 的文件列表不是数组", record.id);
        }
        let formats: Value = serde_json::from_str(&record.formats)
            .with_context(|| format!("历史记录 {} 的格式列表 JSON 无效", record.id))?;
        if !formats.is_array() {
            bail!("历史记录 {} 的格式列表不是数组", record.id);
        }
        let mut content = Map::new();
        if let Some(value) = record.text.clone() {
            content.insert("text".to_owned(), Value::String(value));
        }
        if let Some(value) = record.html.clone() {
            content.insert("html".to_owned(), Value::String(value));
        }
        if let Some(value) = files.clone() {
            content.insert("files".to_owned(), value);
        }
        if let Some(value) = record.rtf.clone() {
            content.insert("rtf".to_owned(), Value::String(value));
        }
        if let Some(value) = record.ocr.clone() {
            content.insert("ocr".to_owned(), Value::String(value));
        }
        let content = Value::Object(content);
        let digest = data_format::content_digest_with_image(&content, record.image.as_deref())
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("无法规范化历史记录 {} 的内容摘要", record.id))?;
        let source_app = record
            .source_app
            .as_ref()
            .and_then(|id| app_ids.get(id))
            .cloned();
        if record.source_app.is_some() && source_app.is_none() {
            report.missing_app_references += 1;
        }
        if let Some(existing_id) = digest_ids.get(&digest).cloned() {
            transaction.execute(
                "UPDATE clipboard_history SET primary_type=CASE WHEN ?1<timestamp THEN ?7 ELSE primary_type END,available_formats_json=CASE WHEN ?1<timestamp THEN ?8 ELSE available_formats_json END,timestamp=MIN(timestamp,?1),source_app=CASE WHEN ?2>=last_used THEN COALESCE(?4,source_app) ELSE source_app END,last_used=MAX(last_used,?2),content_ocr=CASE WHEN ?5>=updated_at THEN COALESCE(?3,content_ocr) ELSE content_ocr END,created_at=MIN(created_at,?9),updated_at=MAX(updated_at,?5) WHERE content_id=?6",
                params![record.timestamp,record.last_used,record.ocr,source_app,record.updated_at,existing_id,record.primary_type,serde_json::to_string(&formats)?,record.created_at],
            )?;
            record_ids.insert(record.id.clone(), existing_id.clone());
            report.history_deduplicated_records += 1;
            if record.id != existing_id {
                report.id_mappings.insert(record.id, existing_id);
            }
            continue;
        }
        let insert_result = transaction.execute(
            "INSERT INTO clipboard_history(content_id,digest,content_text,content_html,content_image,content_rtf,content_files_json,content_ocr,primary_type,available_formats_json,timestamp,last_used,source_app,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                record.id,
                digest,
                record.text,
                record.html,
                record.image,
                record.rtf,
                files.as_ref().map(serde_json::to_string).transpose()?,
                record.ocr,
                record.primary_type,
                serde_json::to_string(&formats)?,
                record.timestamp,
                record.last_used,
                source_app,
                record.created_at,
                record.updated_at,
            ],
        );
        insert_result.with_context(|| format!("无法导入历史记录 {}", record.id))?;
        digest_ids.insert(digest, record.id.clone());
        record_ids.insert(record.id.clone(), record.id);
        report.history_imported_records += 1;
    }
    drop(history_statement);

    if table_exists(&source, "clipboard_history_tags")? {
        let mut statement = source.prepare("SELECT content_id,tag_id,created_at FROM clipboard_history_tags ORDER BY content_id,tag_id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>("content_id")?,
                row.get::<_, String>("tag_id")?,
                timestamp_column(row, "created_at", now_ms())?,
            ))
        })?;
        for row in rows {
            let (content_id, tag_id, created_at) = row?;
            let target_id = record_ids
                .get(&content_id)
                .with_context(|| format!("标签关联引用不存在的历史记录 {content_id}"))?;
            let tag_exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM tag_groups WHERE id=?1)",
                [&tag_id],
                |row| row.get(0),
            )?;
            if !tag_exists {
                bail!("标签关联引用不存在的标签 {tag_id}");
            }
            let inserted = transaction.execute(
                "INSERT OR IGNORE INTO clipboard_history_tags(content_id,tag_id,created_at) VALUES(?1,?2,?3)",
                params![target_id,tag_id,created_at],
            )?;
            if inserted > 0 {
                report.tag_links_imported += 1;
            } else {
                transaction.execute(
                    "UPDATE clipboard_history_tags SET created_at=MIN(created_at,?1) WHERE content_id=?2 AND tag_id=?3",
                    params![created_at,target_id,tag_id],
                )?;
            }
        }
    }
    transaction.commit()?;
    Ok(())
}

struct LegacyHistory {
    id: String,
    text: Option<String>,
    html: Option<String>,
    image: Option<Vec<u8>>,
    rtf: Option<String>,
    files: Option<String>,
    ocr: Option<String>,
    primary_type: String,
    formats: String,
    timestamp: i64,
    last_used: i64,
    source_app: Option<String>,
    created_at: i64,
    updated_at: i64,
}

fn timestamp_column(row: &rusqlite::Row<'_>, name: &str, fallback: i64) -> rusqlite::Result<i64> {
    match row.get_ref(name)? {
        rusqlite::types::ValueRef::Integer(value) => Ok(value),
        rusqlite::types::ValueRef::Real(value) if value.is_finite() => Ok(value as i64),
        rusqlite::types::ValueRef::Text(value) => {
            let value = std::str::from_utf8(value).ok();
            if let Some(timestamp) = value.and_then(|value| value.parse().ok()) {
                return Ok(timestamp);
            }
            if let Some(timestamp) = value
                .and_then(|value| {
                    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()
                })
                .map(|value| value.and_utc().timestamp_millis())
            {
                return Ok(timestamp);
            }
            Ok(fallback)
        }
        _ => Ok(fallback),
    }
}

fn import_copy_statistics(
    target: &mut Connection,
    snapshot: &Path,
    report: &mut MigrationReport,
) -> Result<()> {
    let source = Connection::open_with_flags(snapshot, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if !table_exists(&source, "copy_statistics")? {
        report
            .integrity_checks
            .insert("copyStatistics".to_owned(), "无统计表".to_owned());
        return Ok(());
    }
    let transaction = target.transaction()?;
    let mut statement=source.prepare("SELECT hour,type,source,copies,bytes,files,firstCopiedAt,lastCopiedAt FROM copy_statistics ORDER BY hour,type,source")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>("hour")?,
            row.get::<_, String>("type")?,
            row.get::<_, String>("source")?,
            row.get::<_, i64>("copies")?,
            row.get::<_, i64>("bytes")?,
            row.get::<_, i64>("files")?,
            row.get::<_, i64>("firstCopiedAt")?,
            row.get::<_, i64>("lastCopiedAt")?,
        ))
    })?;
    for row in rows {
        let (hour, kind, source, copies, bytes, files, first, last) = row?;
        if hour < 0 || copies < 0 || bytes < 0 || files < 0 || first < 0 || last < 0 {
            bail!("复制统计包含无效计数或时间");
        }
        transaction.execute("INSERT INTO copy_statistics(hour,type,source,copies,bytes,files,first_copied_at,last_copied_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![hour,kind,source,copies,bytes,files,first,last])?;
        report.copy_statistics_rows += 1;
    }
    if table_exists(&source, "copy_statistics_meta")? {
        report.copy_statistics_cleared_at = source
            .query_row(
                "SELECT value FROM copy_statistics_meta WHERE key='clearedAt'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(cleared_at) = report.copy_statistics_cleared_at {
            transaction.execute(
                "INSERT INTO copy_statistics_meta(key,value) VALUES('clearedAt',?1)",
                [cleared_at],
            )?;
        }
    }
    transaction.commit()?;
    Ok(())
}

fn import_ai_usage(
    target: &mut Connection,
    snapshot: &Path,
    report: &mut MigrationReport,
) -> Result<()> {
    let source = Connection::open_with_flags(snapshot, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_meta = table_exists(&source, "ai_ocr_usage_meta")?;
    let has_recent = table_exists(&source, "ai_ocr_usage")?;
    let has_hourly = table_exists(&source, "ai_ocr_usage_hourly")?;
    let has_models = table_exists(&source, "ai_ocr_models")?;
    if !has_meta && !has_recent && !has_hourly && !has_models {
        report
            .integrity_checks
            .insert("aiUsage".to_owned(), "无用量表".to_owned());
        return Ok(());
    }
    let transaction = target.transaction()?;
    let mut source_models = HashMap::<i64, (String, String, String)>::new();
    if has_models {
        let mut statement =
            source.prepare("SELECT id,provider,baseUrl,model FROM ai_ocr_models ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>("id")?,
                row.get::<_, String>("provider")?,
                row.get::<_, String>("baseUrl")?,
                row.get::<_, String>("model")?,
            ))
        })?;
        for row in rows {
            let (id, provider, base_url, model) = row?;
            transaction.execute(
                "INSERT INTO ai_ocr_models(id,provider,base_url,model) VALUES(?1,?2,?3,?4)",
                params![id, provider, base_url, model],
            )?;
            source_models.insert(id, (provider, base_url, model));
        }
    }
    let hourly_version = if has_meta {
        source
            .query_row(
                "SELECT value FROM ai_ocr_usage_meta WHERE key='hourlyVersion'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
    } else {
        None
    };
    if has_hourly && (hourly_version.is_some() || !has_recent) {
        let mut statement=source.prepare("SELECT hour,modelId,requests,failures,unknownUsage,inputTokens,outputTokens,cachedInputTokens,cacheWriteTokens,totalTokens,durationMs,firstUsedAt,lastUsedAt FROM ai_ocr_usage_hourly ORDER BY hour,modelId")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>("hour")?,
                row.get::<_, i64>("modelId")?,
                row.get::<_, i64>("requests")?,
                row.get::<_, i64>("failures")?,
                row.get::<_, i64>("unknownUsage")?,
                row.get::<_, i64>("inputTokens")?,
                row.get::<_, i64>("outputTokens")?,
                row.get::<_, i64>("cachedInputTokens")?,
                row.get::<_, i64>("cacheWriteTokens")?,
                row.get::<_, i64>("totalTokens")?,
                row.get::<_, i64>("durationMs")?,
                row.get::<_, i64>("firstUsedAt")?,
                row.get::<_, i64>("lastUsedAt")?,
            ))
        })?;
        for row in rows {
            let (
                hour,
                model_id,
                requests,
                failures,
                unknown,
                input,
                output,
                cached,
                cache_write,
                total,
                duration,
                first,
                last,
            ) = row?;
            if !source_models.contains_key(&model_id) {
                bail!("AI 用量小时聚合引用不存在的模型 ID {model_id}");
            }
            if [
                hour,
                requests,
                failures,
                unknown,
                input,
                output,
                cached,
                cache_write,
                total,
                duration,
                first,
                last,
            ]
            .iter()
            .any(|value| *value < 0)
            {
                bail!("AI 用量小时聚合包含负数");
            }
            transaction.execute("INSERT INTO ai_ocr_usage_hourly(hour,model_id,requests,failures,unknown_usage,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens,duration_ms,first_used_at,last_used_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![hour,model_id,requests,failures,unknown,input,output,cached,cache_write,total,duration,first,last])?;
            report.ai_usage_aggregate_rows += 1;
        }
    } else if has_recent {
        let raw_count: i64 =
            source.query_row("SELECT COUNT(*) FROM ai_ocr_usage", [], |row| row.get(0))?;
        if hourly_version.is_some() && raw_count > 0 {
            bail!("AI 用量小时聚合表缺失，但明细已按保留策略裁剪，无法无损恢复");
        }
        if hourly_version.is_none() {
            import_legacy_ai_usage_aggregates(
                &transaction,
                &source,
                &mut report.ai_usage_aggregate_rows,
            )?;
        }
    }
    if has_recent {
        let mut statement=source.prepare("SELECT id,provider,baseUrl,model,purpose,startedAt,finishedAt,durationMs,success,inputTokens,outputTokens,cachedInputTokens,cacheWriteTokens,totalTokens FROM ai_ocr_usage ORDER BY startedAt DESC,id DESC LIMIT 50")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("provider")?,
                row.get::<_, String>("baseUrl")?,
                row.get::<_, String>("model")?,
                row.get::<_, String>("purpose")?,
                row.get::<_, i64>("startedAt")?,
                row.get::<_, i64>("finishedAt")?,
                row.get::<_, i64>("durationMs")?,
                row.get::<_, bool>("success")?,
                row.get::<_, Option<i64>>("inputTokens")?,
                row.get::<_, Option<i64>>("outputTokens")?,
                row.get::<_, i64>("cachedInputTokens")?,
                row.get::<_, i64>("cacheWriteTokens")?,
                row.get::<_, Option<i64>>("totalTokens")?,
            ))
        })?;
        for row in rows {
            let (
                id,
                provider,
                base_url,
                model,
                purpose,
                started,
                finished,
                duration,
                success,
                input,
                output,
                cached,
                cache_write,
                total,
            ) = row?;
            transaction.execute("INSERT INTO ai_ocr_usage_recent(id,provider,base_url,model,purpose,started_at,finished_at,duration_ms,success,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![id,provider,base_url,model,purpose,started,finished,duration,success,input,output,cached,cache_write,total])?;
            report.ai_usage_recent_rows += 1;
        }
    }
    if has_meta {
        report.ai_usage_cleared_at = source
            .query_row(
                "SELECT value FROM ai_ocr_usage_meta WHERE key='clearedAt'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(cleared_at) = report.ai_usage_cleared_at {
            transaction.execute(
                "INSERT INTO ai_ocr_usage_meta(key,value) VALUES('clearedAt',?1)",
                [cleared_at],
            )?;
        }
    }
    report.ai_usage_aggregate_rows =
        transaction.query_row("SELECT COUNT(*) FROM ai_ocr_usage_hourly", [], |row| {
            row.get::<_, i64>(0)
        })? as u64;
    transaction.commit()?;
    Ok(())
}

fn import_legacy_ai_usage_aggregates(
    target: &rusqlite::Transaction<'_>,
    source: &Connection,
    aggregate_rows: &mut u64,
) -> Result<()> {
    let mut model_ids = HashMap::<(String, String, String), i64>::new();
    let mut statement = source.prepare("SELECT provider,baseUrl,model,startedAt,finishedAt,durationMs,success,inputTokens,outputTokens,cachedInputTokens,cacheWriteTokens,totalTokens FROM ai_ocr_usage ORDER BY startedAt,id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>("provider")?,
            row.get::<_, String>("baseUrl")?,
            row.get::<_, String>("model")?,
            row.get::<_, i64>("startedAt")?,
            row.get::<_, i64>("finishedAt")?,
            row.get::<_, i64>("durationMs")?,
            row.get::<_, bool>("success")?,
            row.get::<_, Option<i64>>("inputTokens")?,
            row.get::<_, Option<i64>>("outputTokens")?,
            row.get::<_, i64>("cachedInputTokens")?,
            row.get::<_, i64>("cacheWriteTokens")?,
            row.get::<_, Option<i64>>("totalTokens")?,
        ))
    })?;
    for row in rows {
        let (
            provider,
            base_url,
            model,
            started,
            finished,
            duration,
            success,
            input,
            output,
            cached,
            cache_write,
            total,
        ) = row?;
        let key = (provider.clone(), base_url.clone(), model.clone());
        let model_id = if let Some(id) = model_ids.get(&key) {
            *id
        } else {
            target.execute(
                "INSERT OR IGNORE INTO ai_ocr_models(provider,base_url,model) VALUES(?1,?2,?3)",
                params![provider, base_url, model],
            )?;
            let id = target.query_row(
                "SELECT id FROM ai_ocr_models WHERE provider=?1 AND base_url=?2 AND model=?3",
                params![key.0, key.1, key.2],
                |row| row.get::<_, i64>(0),
            )?;
            model_ids.insert(key, id);
            id
        };
        if [started, finished, duration, cached, cache_write]
            .iter()
            .any(|value| *value < 0)
            || input.is_some_and(|value| value < 0)
            || output.is_some_and(|value| value < 0)
            || total.is_some_and(|value| value < 0)
        {
            bail!("AI 用量明细包含负数");
        }
        let hour = legacy_local_hour(started)?;
        target.execute("INSERT INTO ai_ocr_usage_hourly(hour,model_id,requests,failures,unknown_usage,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens,duration_ms,first_used_at,last_used_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(hour,model_id) DO UPDATE SET requests=requests+1,failures=failures+excluded.failures,unknown_usage=unknown_usage+excluded.unknown_usage,input_tokens=input_tokens+excluded.input_tokens,output_tokens=output_tokens+excluded.output_tokens,cached_input_tokens=cached_input_tokens+excluded.cached_input_tokens,cache_write_tokens=cache_write_tokens+excluded.cache_write_tokens,total_tokens=total_tokens+excluded.total_tokens,duration_ms=duration_ms+excluded.duration_ms,first_used_at=MIN(first_used_at,excluded.first_used_at),last_used_at=MAX(last_used_at,excluded.last_used_at)",params![hour,model_id,i64::from(!success),i64::from(total.is_none()),input.unwrap_or(0),output.unwrap_or(0),cached,cache_write,total.unwrap_or(0),duration,started,finished])?;
    }
    *aggregate_rows = target.query_row("SELECT COUNT(*) FROM ai_ocr_usage_hourly", [], |row| {
        row.get::<_, i64>(0)
    })? as u64;
    Ok(())
}

fn legacy_local_hour(timestamp: i64) -> Result<i64> {
    let date = Local
        .timestamp_millis_opt(timestamp)
        .single()
        .context("AI 用量明细的时间超出支持范围")?;
    Ok(date
        .with_minute(0)
        .context("AI 用量明细的时间无效")?
        .with_second(0)
        .context("AI 用量明细的时间无效")?
        .with_nanosecond(0)
        .context("AI 用量明细的时间无效")?
        .timestamp_millis())
}

fn import_models(source: &Path, target_data: &Path, report: &mut MigrationReport) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    if !source.is_dir() {
        bail!("旧 OCR 模型路径不是目录");
    }
    let destination_root = target_data.join(data_format::OCR_MODELS_DIRECTORY);
    fs::create_dir_all(&destination_root)?;
    for entry in fs::read_dir(source)? {
        let directory = entry?.path();
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink() {
            bail!("OCR 模型目录包含符号链接");
        }
        if !metadata.is_dir() {
            continue;
        }
        let fallback = directory
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let manifest_path = directory.join(data_format::OCR_MODEL_MANIFEST_FILE);
        if !manifest_path.is_file() {
            report
                .models_skipped
                .push(format!("{fallback}: 缺少模型清单"));
            continue;
        }
        let attempt = import_one_model(&directory, &destination_root);
        match attempt {
            Ok(model_id) => report.models_imported.push(model_id),
            Err(error) => report.models_skipped.push(format!("{fallback}: {error}")),
        }
    }
    Ok(())
}

fn import_one_model(source: &Path, destination_root: &Path) -> Result<String> {
    let raw: Value = serde_json::from_slice(&fs::read(
        source.join(data_format::OCR_MODEL_MANIFEST_FILE),
    )?)
    .context("模型清单 JSON 无效")?;
    let model_id = raw
        .get("modelId")
        .and_then(Value::as_str)
        .context("模型清单缺少 modelId")?;
    let model = catalog::get_model(model_id).map_err(|_| anyhow!("不支持的模型标识"))?;
    let revision = raw
        .get("revision")
        .and_then(Value::as_str)
        .context("模型清单缺少 revision")?;
    if revision != catalog::model_revision(model) {
        bail!("模型修订版本不匹配");
    }
    let files = raw
        .get("files")
        .and_then(Value::as_object)
        .context("模型清单缺少文件索引")?
        .clone();
    let manifest = OcrModelManifest {
        format: raw
            .get("format")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        model_id: model_id.to_owned(),
        revision: revision.to_owned(),
        files,
    };
    manifest.validate().map_err(|error| anyhow!("{error}"))?;
    let file_names = [
        ("det", "det.onnx"),
        ("rec", "rec.onnx"),
        ("dictionary", "dict.txt"),
    ];
    let mut payloads = Vec::new();
    for (role, name) in file_names {
        let integrity: OcrModelFile = serde_json::from_value(
            manifest
                .files
                .get(role)
                .cloned()
                .context("模型文件条目缺失")?,
        )?;
        let path = source.join(name);
        let metadata =
            fs::symlink_metadata(&path).with_context(|| format!("缺少模型文件 {name}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("模型文件不是普通文件: {name}");
        }
        if metadata.len() != integrity.size {
            bail!("模型文件大小不匹配: {name}");
        }
        let bytes = fs::read(&path)?;
        if data_format::sha256_hex(&bytes) != integrity.sha256 {
            bail!("模型文件摘要校验失败: {name}");
        }
        if role == "dictionary" && bytes.iter().all(u8::is_ascii_whitespace) {
            bail!("OCR 字典为空");
        }
        payloads.push((name, bytes));
    }
    let model_staging = tempdir_in(destination_root)?;
    let target = model_staging.path();
    for (name, bytes) in payloads {
        let path = target.join(name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target.join(data_format::OCR_MODEL_MANIFEST_FILE))?;
    file.write_all(&manifest_bytes)?;
    file.sync_all()?;
    fs::rename(target, destination_root.join(model_id))
        .with_context(|| format!("无法提交模型目录 {model_id}"))?;
    Ok(model_id.to_owned())
}

fn verify_database(connection: &Connection, report: &MigrationReport) -> Result<()> {
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        bail!("目标数据库完整性检查失败: {integrity}");
    }
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if rows.next()?.is_some() {
        bail!("目标数据库存在外键引用错误");
    }
    let history_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
            row.get(0)
        })?;
    if history_count as u64 != report.history_imported_records {
        bail!("历史记录数量校验失败");
    }
    let apps_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM apps", [], |row| row.get(0))?;
    if apps_count as u64 != report.apps_imported {
        bail!("应用来源数量校验失败");
    }
    let tags_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM tag_groups", [], |row| row.get(0))?;
    if tags_count as u64 != report.tags_imported {
        bail!("标签数量校验失败");
    }
    let tag_links_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM clipboard_history_tags", [], |row| {
            row.get(0)
        })?;
    if tag_links_count as u64 != report.tag_links_imported {
        bail!("标签关联数量校验失败");
    }
    let stats_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM copy_statistics", [], |row| row.get(0))?;
    if stats_count as u64 != report.copy_statistics_rows {
        bail!("复制统计数量校验失败");
    }
    let ai_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM ai_ocr_usage_hourly", [], |row| {
            row.get(0)
        })?;
    if ai_count as u64 != report.ai_usage_aggregate_rows {
        bail!("AI 用量聚合数量校验失败");
    }
    let recent_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM ai_ocr_usage_recent", [], |row| {
            row.get(0)
        })?;
    if recent_count as u64 != report.ai_usage_recent_rows {
        bail!("AI 用量明细数量校验失败");
    }
    verify_content_digests(connection)?;
    if report.history_deduplicated_records + report.history_imported_records
        != report.history_source_records
    {
        bail!("历史记录去重数量校验失败");
    }
    Ok(())
}

fn verify_content_digests(connection: &Connection) -> Result<()> {
    let mut statement=connection.prepare("SELECT content_id,digest,content_text,content_html,content_image,content_rtf,content_files_json,content_ocr FROM clipboard_history")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    for row in rows {
        let (id, stored, text, html, image, rtf, files, ocr) = row?;
        let mut content = Map::new();
        if let Some(value) = text {
            content.insert("text".to_owned(), Value::String(value));
        }
        if let Some(value) = html {
            content.insert("html".to_owned(), Value::String(value));
        }
        if let Some(value) = rtf {
            content.insert("rtf".to_owned(), Value::String(value));
        }
        if let Some(value) = files {
            content.insert(
                "files".to_owned(),
                serde_json::from_str(&value).context("目标文件列表 JSON 无效")?,
            );
        }
        if let Some(value) = ocr {
            content.insert("ocr".to_owned(), Value::String(value));
        }
        let digest =
            data_format::content_digest_with_image(&Value::Object(content), image.as_deref())
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("无法复算目标记录 {id} 摘要"))?;
        if digest != stored {
            bail!("目标记录 {id} 内容摘要不匹配");
        }
    }
    Ok(())
}
