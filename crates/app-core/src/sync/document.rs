use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use super_paste_format::{Digest, SyncProjection};
use uuid::Uuid;
use crate::{Error, Result};

pub(super) const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
pub(super) struct Payload { pub value: SyncProjection, pub raw: Box<RawValue>, pub hash: Digest }
impl Payload {
    pub fn new(value: SyncProjection) -> Result<Self> {
        value.validate()?;
        let raw = serde_json::value::to_raw_value(&value)?;
        if raw.get().len() > MAX_DOCUMENT_BYTES { return Err(Error::ResourceLimit("同步配置超过文档大小上限".into())); }
        let hash = digest(raw.get().as_bytes());
        Ok(Self { value, raw, hash })
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Metadata { pub revision: Uuid, pub parent_revision: Option<Uuid>, pub updated_at: i64 }
impl Metadata {
    pub fn new(parent_revision: Option<Uuid>) -> Result<Self> { Ok(Self { revision: Uuid::new_v4(), parent_revision, updated_at: crate::now_ms()? }) }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Envelope<'a> {
    format_version: u32, config_schema_version: u32, revision: Uuid,
    #[serde(deserialize_with = "required_parent")] parent_revision: Option<Uuid>,
    updated_at: i64, payload_sha256: String,
    #[serde(borrow)] payload: &'a RawValue,
}
fn required_parent<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Option<Uuid>, D::Error> { Option::deserialize(deserializer) }
pub(super) struct Document { pub metadata: Metadata, pub payload: Payload, pub etag: Option<String> }
impl Document {
    pub fn envelope(&self) -> Envelope<'_> { Envelope { format_version: 1, config_schema_version: 1, revision: self.metadata.revision,
        parent_revision: self.metadata.parent_revision, updated_at: self.metadata.updated_at, payload_sha256: hex(self.payload.hash), payload: &self.payload.raw } }
    pub fn encode(metadata: Metadata, payload: &Payload) -> Result<Bytes> {
        // The canonical payload is serialized once. Envelope serialization
        // copies its raw JSON bytes without visiting the settings a second time.
        let bytes = serde_json::to_vec(&Envelope { format_version: 1, config_schema_version: 1, revision: metadata.revision,
            parent_revision: metadata.parent_revision, updated_at: metadata.updated_at, payload_sha256: hex(payload.hash), payload: &payload.raw })?;
        if bytes.len() > MAX_DOCUMENT_BYTES { return Err(Error::ResourceLimit("同步文档超过大小上限".into())); }
        Ok(Bytes::from(bytes))
    }
    pub fn decode(bytes: &[u8], etag: Option<String>) -> Result<Self> {
        if bytes.len() > MAX_DOCUMENT_BYTES { return Err(Error::ResourceLimit("远端配置超过文档大小上限".into())); }
        let wire: Envelope<'_> = serde_json::from_slice(bytes).map_err(|_| invalid("远端配置 envelope 无效"))?;
        if wire.format_version != 1 || wire.config_schema_version != 1 { return Err(invalid("不支持的远端配置文档版本")); }
        if wire.updated_at <= 0 { return Err(invalid("远端配置更新时间无效")); }
        let expected = parse_hex(&wire.payload_sha256)?;
        let value: SyncProjection = serde_json::from_str(wire.payload.get()).map_err(|_| invalid("远端配置字段不在同步白名单或类型无效"))?;
        // v1 hashes canonical JSON rather than transport whitespace. The
        // typed wire serializer preserves the existing alphabetical key order.
        let payload = Payload::new(value)?;
        if payload.hash != expected { return Err(invalid("远端配置摘要校验失败")); }
        Ok(Self { metadata: Metadata { revision: wire.revision, parent_revision: wire.parent_revision, updated_at: wire.updated_at }, payload, etag })
    }
}
pub(super) fn digest(bytes: &[u8]) -> Digest { Digest(Sha256::digest(bytes).into()) }
pub(super) fn hex(value: Digest) -> String { value.0.iter().map(|byte| format!("{byte:02x}")).collect() }
pub(super) fn parse_hex(value: &str) -> Result<Digest> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) { return Err(invalid("远端配置摘要格式无效")); }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() { *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| invalid("远端配置摘要格式无效"))?; }
    Ok(Digest(digest))
}
fn invalid(message: &'static str) -> Error { Error::Invalid(message.into()) }
