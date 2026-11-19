use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::ValidationError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode { #[default] System, Light, Dark }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition { Top, #[default] Bottom, Left, Right }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OcrMode { #[default] Local, Remote, Openai, Claude }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteRequestFormat { #[default] Json, Form }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenaiApi { #[default] Chat, Responses }

/// Secrets have a separate credential-store API and cannot be serialized here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiOcrConfig { pub base_url: String, pub model: String, pub api: OpenaiApi }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiOcrConfigs { pub openai: AiOcrConfig, pub claude: AiOcrConfig }
impl Default for AiOcrConfigs {
    fn default() -> Self {
        Self {
            openai: AiOcrConfig { base_url: "https://api.openai.com/v1".into(), model: String::new(), api: OpenaiApi::Chat },
            claude: AiOcrConfig { base_url: "https://api.anthropic.com".into(), model: String::new(), api: OpenaiApi::Chat },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub color_mode: ColorMode,
    pub auto_start: bool,
    pub silent_start: bool,
    pub overlay_position: OverlayPosition,
    pub global_shortcut: String,
    pub max_history_size: i64,
    pub max_keep_time: i64,
    pub clipboard_listening: bool,
    #[serde(rename = "enableLocalOCR")]
    pub enable_local_ocr: bool,
    pub ocr_mode: OcrMode,
    pub ocr_local_model: String,
    pub ocr_remote_url: String,
    pub ocr_remote_extra_params: String,
    pub ocr_remote_image_field_name: String,
    pub ocr_remote_request_format: RemoteRequestFormat,
    pub ocr_remote_result_path: String,
    #[serde(rename = "ocrAIConfigs")]
    pub ocr_ai_configs: AiOcrConfigs,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            color_mode: ColorMode::System, auto_start: false, silent_start: false,
            overlay_position: OverlayPosition::Bottom, global_shortcut: String::new(),
            max_history_size: 100, max_keep_time: -1, clipboard_listening: true,
            enable_local_ocr: false, ocr_mode: OcrMode::Local, ocr_local_model: "ppocr-v6-tiny".into(),
            ocr_remote_url: String::new(), ocr_remote_extra_params: "{}".into(),
            ocr_remote_image_field_name: "image".into(), ocr_remote_request_format: RemoteRequestFormat::Json,
            ocr_remote_result_path: "data.result".into(), ocr_ai_configs: AiOcrConfigs::default(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Preserve backend acceptance; the settings controls impose their narrower UI ranges.
        if self.max_history_size < -1 || self.max_keep_time < -1 {
            return Err(ValidationError("历史记录限制无效".into()));
        }
        if self.ocr_mode == OcrMode::Remote {
            let extra: serde_json::Value = serde_json::from_str(&self.ocr_remote_extra_params)
                .map_err(|_| ValidationError("远程 OCR 额外参数必须是 JSON 对象".into()))?;
            if !extra.is_object() { return Err(ValidationError("远程 OCR 额外参数必须是 JSON 对象".into())); }
            let address = self.ocr_remote_url.trim();
            if self.enable_local_ocr && address.is_empty() { return Err(ValidationError("远程 OCR 地址不能为空".into())); }
            if !address.is_empty() { validate_http_url(address, "远程 OCR 地址")?; }
        }
        for config in [&self.ocr_ai_configs.openai, &self.ocr_ai_configs.claude] {
            if !config.base_url.trim().is_empty() {
                let url = validate_http_url(config.base_url.trim(), "AI OCR Base URL")?;
                if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
                    return Err(ValidationError("AI OCR Base URL 不能包含用户名、密码、查询参数或片段".into()));
                }
            }
        }
        if self.ocr_ai_configs.claude.api != OpenaiApi::Chat { return Err(ValidationError("Claude API 类型无效".into())); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncDirection { #[default] Bidirectional, Upload, Download }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavConfig {
    pub server_url: String, pub username: String, pub remote_path: String,
    pub timeout: u64, pub use_https: bool,
}
impl Default for WebDavConfig {
    fn default() -> Self { Self { server_url: String::new(), username: String::new(), remote_path: String::new(), timeout: 30_000, use_https: false } }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncPreferences {
    pub enabled: bool, pub provider: String, pub interval: u64,
    pub direction: SyncDirection, pub webdav: WebDavConfig,
}
impl Default for SyncPreferences {
    fn default() -> Self { Self { enabled: false, provider: String::new(), interval: 1_800_000, direction: SyncDirection::Bidirectional, webdav: WebDavConfig::default() } }
}
impl SyncPreferences {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !matches!(self.provider.as_str(), "" | "webdav") { return Err(ValidationError("同步提供者无效".into())); }
        if self.interval == 0 || self.webdav.timeout == 0 { return Err(ValidationError("同步时间参数无效".into())); }
        if !self.webdav.server_url.trim().is_empty() {
            let url = validate_http_url(self.webdav.server_url.trim(), "WebDAV 服务地址")?;
            if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
                return Err(ValidationError("WebDAV 地址必须是无凭据的 HTTP 或 HTTPS URL".into()));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpSettings { pub enabled: bool, pub port: u16, pub auth_enabled: bool }
impl Default for McpSettings {
    fn default() -> Self { Self { enabled: false, port: 19528, auth_enabled: true } }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration { pub settings: Preferences, pub sync: SyncPreferences, pub mcp: McpSettings }
impl Configuration {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.settings.validate()?;
        self.sync.validate()?;
        if self.mcp.port < 1024 { return Err(ValidationError("MCP 端口须为 1024–65535".into())); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CredentialId { WebDavPassword, OpenaiApiKey, ClaudeApiKey, McpApiKey }
impl CredentialId {
    pub const ALL: &'static [Self] = &[Self::WebDavPassword, Self::OpenaiApiKey, Self::ClaudeApiKey, Self::McpApiKey];
    pub const fn field(self) -> ConfigField {
        match self { Self::WebDavPassword => ConfigField::WebDavPassword, Self::OpenaiApiKey => ConfigField::OpenaiApiKey, Self::ClaudeApiKey => ConfigField::ClaudeApiKey, Self::McpApiKey => ConfigField::McpApiKey }
    }
    pub const fn account(self) -> &'static str {
        match self { Self::WebDavPassword => "webdav.password", Self::OpenaiApiKey => "ocr.openai.api-key", Self::ClaudeApiKey => "ocr.claude.api-key", Self::McpApiKey => "mcp.api-key" }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialAvailability { Available, Configured, Unset, Unavailable, Error }

#[derive(Clone, Debug)]
pub struct ConfigSnapshot { pub value: Arc<Configuration>, pub revision: u64 }

macro_rules! configuration_fields {
    ($( $variant:ident($ty:ty) => $name:literal, $root:ident $(.$field:ident)+; )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum ConfigField { $( $variant, )* WebDavPassword, OpenaiApiKey, ClaudeApiKey, McpApiKey }
        impl ConfigField {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)* Self::WebDavPassword, Self::OpenaiApiKey, Self::ClaudeApiKey, Self::McpApiKey];
            pub fn change(self, configuration: &Configuration) -> Option<ConfigChange> { match self {
                $(Self::$variant => Some(ConfigChange::$variant(configuration.$root$(.$field)+.clone())),)*
                _ => None,
            } }
            pub fn differs(self, left: &Configuration, right: &Configuration) -> bool { match self {
                $(Self::$variant => left.$root$(.$field)+ != right.$root$(.$field)+,)*
                _ => false,
            } }
            pub const fn as_str(self) -> &'static str { match self {
                $(Self::$variant => $name,)*
                Self::WebDavPassword => "credential.webdav.password", Self::OpenaiApiKey => "credential.ocr.openai.api-key",
                Self::ClaudeApiKey => "credential.ocr.claude.api-key", Self::McpApiKey => "credential.mcp.api-key",
            } }
        }
        #[derive(Clone, Debug)]
        pub enum ConfigChange { $( $variant($ty), )* }
        impl ConfigChange {
            pub const fn field(&self) -> ConfigField { match self { $(Self::$variant(_) => ConfigField::$variant,)* } }
            pub fn apply(&self, configuration: &mut Configuration) {
                match self { $(Self::$variant(value) => configuration.$root$(.$field)+ = value.clone(),)* }
            }
            pub fn differs(&self, configuration: &Configuration) -> bool {
                match self { $(Self::$variant(value) => configuration.$root$(.$field)+ != *value,)* }
            }
        }
    };
}
configuration_fields! {
    ColorMode(ColorMode) => "settings.colorMode", settings.color_mode;
    AutoStart(bool) => "settings.autoStart", settings.auto_start;
    SilentStart(bool) => "settings.silentStart", settings.silent_start;
    OverlayPosition(OverlayPosition) => "settings.overlayPosition", settings.overlay_position;
    GlobalShortcut(String) => "settings.globalShortcut", settings.global_shortcut;
    MaxHistorySize(i64) => "settings.maxHistorySize", settings.max_history_size;
    MaxKeepTime(i64) => "settings.maxKeepTime", settings.max_keep_time;
    ClipboardListening(bool) => "settings.clipboardListening", settings.clipboard_listening;
    EnableLocalOcr(bool) => "settings.enableLocalOCR", settings.enable_local_ocr;
    OcrMode(OcrMode) => "settings.ocrMode", settings.ocr_mode;
    OcrLocalModel(String) => "settings.ocrLocalModel", settings.ocr_local_model;
    OcrRemoteUrl(String) => "settings.ocrRemoteUrl", settings.ocr_remote_url;
    OcrRemoteExtraParams(String) => "settings.ocrRemoteExtraParams", settings.ocr_remote_extra_params;
    OcrRemoteImageFieldName(String) => "settings.ocrRemoteImageFieldName", settings.ocr_remote_image_field_name;
    OcrRemoteRequestFormat(RemoteRequestFormat) => "settings.ocrRemoteRequestFormat", settings.ocr_remote_request_format;
    OcrRemoteResultPath(String) => "settings.ocrRemoteResultPath", settings.ocr_remote_result_path;
    OpenaiConfig(AiOcrConfig) => "settings.ocrAIConfigs.openai", settings.ocr_ai_configs.openai;
    ClaudeConfig(AiOcrConfig) => "settings.ocrAIConfigs.claude", settings.ocr_ai_configs.claude;
    SyncEnabled(bool) => "sync.enabled", sync.enabled;
    SyncProvider(String) => "sync.provider", sync.provider;
    SyncInterval(u64) => "sync.interval", sync.interval;
    SyncDirection(SyncDirection) => "sync.direction", sync.direction;
    WebDav(WebDavConfig) => "sync.params", sync.webdav;
    McpEnabled(bool) => "mcp.enabled", mcp.enabled;
    McpPort(u16) => "mcp.port", mcp.port;
    McpAuthEnabled(bool) => "mcp.authEnabled", mcp.auth_enabled;
}

fn validate_http_url(value: &str, label: &str) -> Result<url::Url, ValidationError> {
    let url = url::Url::parse(value).map_err(|_| ValidationError(format!("{label}无效")))?;
    if !matches!(url.scheme(), "http" | "https") { return Err(ValidationError(format!("{label}必须使用 HTTP 或 HTTPS"))); }
    Ok(url)
}

