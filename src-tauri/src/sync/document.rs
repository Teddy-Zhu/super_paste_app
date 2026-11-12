use crate::{config::validate_remote_projection_value, events::now_ms};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const FORMAT_VERSION: u32 = 1;
const CONFIG_SCHEMA_VERSION: u32 = 1;
pub(super) const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RemoteEnvelope {
    pub(super) format_version: u32,
    pub(super) config_schema_version: u32,
    pub(super) revision: String,
    pub(super) parent_revision: Option<String>,
    pub(super) updated_at: i64,
    pub(super) payload_sha256: String,
    pub(super) payload: Value,
}

impl RemoteEnvelope {
    pub(super) fn new(payload: Value, parent_revision: Option<String>) -> Result<Self, String> {
        let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
        Ok(Self {
            format_version: FORMAT_VERSION,
            config_schema_version: CONFIG_SCHEMA_VERSION,
            revision: Uuid::new_v4().to_string(),
            parent_revision,
            updated_at: now_ms(),
            payload_sha256: data_format::sha256_hex(&bytes),
            payload,
        })
    }

    fn validate(self) -> Result<Self, String> {
        if self.format_version != FORMAT_VERSION {
            return Err(format!("不支持的 WebDAV 文档格式: {}", self.format_version));
        }
        if self.config_schema_version != CONFIG_SCHEMA_VERSION {
            return Err(format!(
                "不支持的远端配置 schema: {}",
                self.config_schema_version
            ));
        }
        Uuid::parse_str(&self.revision).map_err(|_| "远端 revision 无效".to_owned())?;
        if let Some(parent) = &self.parent_revision {
            Uuid::parse_str(parent).map_err(|_| "远端 parentRevision 无效".to_owned())?;
        }
        if self.updated_at <= 0
            || self.payload_sha256.len() != 64
            || !self
                .payload_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("远端配置元数据无效".to_owned());
        }
        let bytes = serde_json::to_vec(&self.payload).map_err(|error| error.to_string())?;
        if data_format::sha256_hex(&bytes) != self.payload_sha256.to_ascii_lowercase() {
            return Err("远端配置摘要校验失败".to_owned());
        }
        validate_remote_projection_value(&self.payload)?;
        Ok(self)
    }
}

pub(super) fn decode_envelope(bytes: &[u8]) -> Result<RemoteEnvelope, String> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err("远端配置超过文档大小限制".to_owned());
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("远端配置 JSON 无效: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "远端配置文档必须是对象".to_owned())?;
    let expected = [
        "formatVersion",
        "configSchemaVersion",
        "revision",
        "parentRevision",
        "updatedAt",
        "payloadSha256",
        "payload",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err("远端配置 envelope 字段无效".to_owned());
    }
    serde_json::from_value::<RemoteEnvelope>(value)
        .map_err(|error| format!("远端配置 envelope 无效: {error}"))?
        .validate()
}
