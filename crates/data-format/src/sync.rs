use serde::{Deserialize, Serialize};
use crate::{AiOcrConfig, AiOcrConfigs, ColorMode, ConfigChange, ConfigField, Configuration, OcrMode, OpenaiApi, RemoteRequestFormat, SyncDirection, ValidationError};

/// The existing WebDAV v1 whitelist. Endpoint credentials, machine-local
/// integration settings, history and tags cannot enter this document.
/// Declaration order is canonical JSON key order for the v1 payload digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncProjection { pub settings: RemoteSettings, pub sync: RemoteSchedule }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteSettings {
    pub color_mode: ColorMode,
    #[serde(rename = "enableLocalOCR")] pub enable_local_ocr: bool,
    pub max_history_size: i64,
    pub max_keep_time: i64,
    #[serde(rename = "ocrAIConfigs", with = "AiConfigsWire")] pub ocr_ai_configs: AiOcrConfigs,
    pub ocr_local_model: String,
    pub ocr_mode: OcrMode,
    pub ocr_remote_image_field_name: String,
    pub ocr_remote_request_format: RemoteRequestFormat,
    pub ocr_remote_result_path: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteSchedule { pub direction: SyncDirection, pub enabled: bool, pub interval: u64, pub provider: String }

#[derive(Serialize, Deserialize)]
#[serde(remote = "AiOcrConfigs", deny_unknown_fields)]
struct AiConfigsWire {
    #[serde(with = "AiProfileWire")] claude: AiOcrConfig,
    #[serde(with = "AiProfileWire")] openai: AiOcrConfig,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "AiOcrConfig", rename_all = "camelCase", deny_unknown_fields)]
struct AiProfileWire { api: OpenaiApi, base_url: String, model: String }

impl SyncProjection {
    pub const FIELDS: [ConfigField; 15] = [ConfigField::ColorMode, ConfigField::EnableLocalOcr, ConfigField::MaxHistorySize, ConfigField::MaxKeepTime,
        ConfigField::ClaudeConfig, ConfigField::OpenaiConfig, ConfigField::OcrLocalModel, ConfigField::OcrMode, ConfigField::OcrRemoteImageFieldName,
        ConfigField::OcrRemoteRequestFormat, ConfigField::OcrRemoteResultPath, ConfigField::SyncDirection, ConfigField::SyncEnabled, ConfigField::SyncInterval, ConfigField::SyncProvider];
    pub fn capture(configuration: &Configuration) -> Self {
        let settings = &configuration.settings; let sync = &configuration.sync;
        Self { settings: RemoteSettings { color_mode: settings.color_mode, enable_local_ocr: settings.enable_local_ocr, max_history_size: settings.max_history_size,
            max_keep_time: settings.max_keep_time, ocr_ai_configs: settings.ocr_ai_configs.clone(), ocr_local_model: settings.ocr_local_model.clone(),
            ocr_mode: settings.ocr_mode, ocr_remote_image_field_name: settings.ocr_remote_image_field_name.clone(), ocr_remote_request_format: settings.ocr_remote_request_format,
            ocr_remote_result_path: settings.ocr_remote_result_path.clone() },
            sync: RemoteSchedule { direction: sync.direction, enabled: sync.enabled, interval: sync.interval, provider: sync.provider.clone() } }
    }
    pub fn changes(&self) -> Vec<ConfigChange> {
        let value = &self.settings;
        vec![ConfigChange::ColorMode(value.color_mode), ConfigChange::EnableLocalOcr(value.enable_local_ocr), ConfigChange::MaxHistorySize(value.max_history_size),
            ConfigChange::MaxKeepTime(value.max_keep_time), ConfigChange::ClaudeConfig(value.ocr_ai_configs.claude.clone()), ConfigChange::OpenaiConfig(value.ocr_ai_configs.openai.clone()),
            ConfigChange::OcrLocalModel(value.ocr_local_model.clone()), ConfigChange::OcrMode(value.ocr_mode), ConfigChange::OcrRemoteImageFieldName(value.ocr_remote_image_field_name.clone()),
            ConfigChange::OcrRemoteRequestFormat(value.ocr_remote_request_format), ConfigChange::OcrRemoteResultPath(value.ocr_remote_result_path.clone()),
            ConfigChange::SyncDirection(self.sync.direction), ConfigChange::SyncEnabled(self.sync.enabled), ConfigChange::SyncInterval(self.sync.interval), ConfigChange::SyncProvider(self.sync.provider.clone())]
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut candidate = Configuration::default();
        for change in self.changes() { change.apply(&mut candidate); }
        // The whitelist deliberately omits the local remote-OCR endpoint.
        // Active-service prerequisites are checked against the real local
        // configuration when its guarded mutation is committed.
        candidate.settings.enable_local_ocr = false;
        candidate.validate()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncState { #[default] Idle, Syncing, Success, Error }
#[derive(Clone, Debug, Default)]
pub struct SyncStatus {
    pub is_running: bool, pub last_sync_time: i64, pub next_sync_time: i64,
    pub state: SyncState, pub error: Option<String>, pub restart_required: bool, pub restart_revision: u64,
}
#[derive(Clone, Debug)]
pub struct SyncOutcome {
    pub message: String, pub uploaded: bool, pub downloaded: bool,
    pub conflict: Option<String>, pub repeated_conflict: bool, pub restart_required: bool,
}
