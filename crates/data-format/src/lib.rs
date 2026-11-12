use image::ImageFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const FORMAT_ID: &str = "super-paste";
pub const FORMAT_VERSION: u32 = 1;
pub const DATABASE_FILE: &str = "app.db";
pub const OCR_MODELS_DIRECTORY: &str = "ocr-models";
pub const OCR_MODEL_MANIFEST_FILE: &str = "manifest.json";
pub const OCR_MODEL_MANIFEST_VERSION: u32 = 1;
pub const DEFAULT_MCP_PORT: u16 = 19528;

pub const INITIAL_SCHEMA: &str = include_str!("../migrations/0001_initial.sql");

pub fn default_preferences() -> Value {
    serde_json::json!({
        "settings": {
            "colorMode": "system",
            "autoStart": false,
            "silentStart": false,
            "overlayPosition": "bottom",
            "globalShortcut": "",
            "maxHistorySize": 100,
            "maxKeepTime": -1,
            "clipboardListening": true,
            "enableLocalOCR": false,
            "ocrMode": "local",
            "ocrLocalModel": "ppocr-v6-tiny",
            "ocrRemoteUrl": "",
            "ocrRemoteExtraParams": "{}",
            "ocrRemoteImageFieldName": "image",
            "ocrRemoteRequestFormat": "json",
            "ocrRemoteResultPath": "data.result",
            "ocrAIConfigs": {
                "openai": { "baseUrl": "https://api.openai.com/v1", "apiKey": "", "model": "", "api": "chat" },
                "claude": { "baseUrl": "https://api.anthropic.com", "apiKey": "", "model": "", "api": "chat" }
            }
        },
        "sync": {
            "enabled": false,
            "provider": "",
            "interval": 1800000,
            "direction": "bidirectional",
            "params": {
                "webdav": {
                    "serverUrl": "",
                    "username": "",
                    "password": "",
                    "remotePath": "",
                    "timeout": 30000,
                    "useHttps": false
                }
            }
        }
    })
}

pub fn initialize_schema(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    connection.execute_batch(INITIAL_SCHEMA)?;
    connection.pragma_update(None, "user_version", FORMAT_VERSION)?;
    Ok(())
}

/// Produces a stable digest of all user-visible clipboard formats.
/// Image encodings with equivalent decoded pixels intentionally deduplicate.
pub fn content_digest(content: &Value) -> Result<String, String> {
    let image = content
        .get("image")
        .filter(|value| !value.is_null())
        .map(image_bytes)
        .transpose()?;
    content_digest_with_image(content, image.as_deref())
}

pub fn content_digest_with_image(content: &Value, image: Option<&[u8]>) -> Result<String, String> {
    let content = content
        .as_object()
        .ok_or_else(|| "剪贴板内容必须是对象".to_owned())?;
    for key in ["text", "html", "rtf", "ocr"] {
        if content
            .get(key)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(format!("剪贴板 {key} 内容格式无效"));
        }
    }
    if content
        .get("files")
        .is_some_and(|value| !value.is_null() && !value.is_array())
    {
        return Err("剪贴板文件列表格式无效".to_owned());
    }
    let mut normalized = Map::new();
    for key in ["text", "html", "rtf", "files"] {
        if let Some(value) = content.get(key).filter(|value| !value.is_null()) {
            normalized.insert(key.to_owned(), value.clone());
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(b"super-paste-content-v1\0");
    hasher.update(serde_json::to_vec(&normalized).map_err(|error| error.to_string())?);
    if let Some(bytes) = image {
        add_image_digest(&mut hasher, bytes);
    } else if let Some(value) = content.get("image").filter(|value| !value.is_null()) {
        add_image_digest(&mut hasher, &image_bytes(value)?);
    }
    Ok(to_hex(&hasher.finalize()))
}

fn add_image_digest(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(b"\0image\0");
    match image::load_from_memory(bytes) {
        Ok(decoded) => {
            hasher.update(b"rgba8\0");
            hasher.update(decoded.width().to_be_bytes());
            hasher.update(decoded.height().to_be_bytes());
            hasher.update(decoded.into_rgba8().as_raw());
        }
        Err(_) => {
            hasher.update(b"raw\0");
            hasher.update((bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }
    }
}

pub fn image_bytes(value: &Value) -> Result<Vec<u8>, String> {
    if let Some(array) = value.as_array() {
        return array
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .filter(|byte| *byte <= u8::MAX as u64)
                    .map(|byte| byte as u8)
                    .ok_or_else(|| "剪贴板图片包含无效字节".to_owned())
            })
            .collect();
    }
    if let Some(encoded) = value.as_str() {
        let (metadata, payload) = encoded
            .split_once(',')
            .ok_or_else(|| "剪贴板图片编码无效".to_owned())?;
        if !metadata.starts_with("data:image/") || !metadata.ends_with(";base64") {
            return Err("剪贴板图片必须是图片 Base64 数据".to_owned());
        }
        return decode_base64(payload);
    }
    Err("剪贴板图片必须是字节数组或图片 Base64 数据".to_owned())
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            byte if byte.is_ascii_whitespace() => continue,
            _ => return Err("剪贴板图片 Base64 数据无效".to_owned()),
        };
        accumulator = (accumulator << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
        }
    }
    Ok(output)
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrModelFile {
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrModelManifest {
    pub format: u32,
    #[serde(rename = "modelId")]
    pub model_id: String,
    pub revision: String,
    pub files: Map<String, Value>,
}

impl OcrModelManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.format != OCR_MODEL_MANIFEST_VERSION {
            return Err(format!("不支持的 OCR 模型清单版本: {}", self.format));
        }
        if self.model_id.trim().is_empty() || self.revision.trim().is_empty() {
            return Err("OCR 模型清单缺少模型标识或修订版本".to_owned());
        }
        for name in ["det", "rec", "dictionary"] {
            let value = self
                .files
                .get(name)
                .ok_or_else(|| format!("OCR 模型清单缺少 {name} 文件"))?;
            let file: OcrModelFile = serde_json::from_value(value.clone())
                .map_err(|error| format!("OCR 模型清单中的 {name} 文件无效: {error}"))?;
            if file.size == 0
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(format!("OCR 模型 {name} 文件信息无效"));
            }
        }
        Ok(())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

pub fn supported_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    image::guess_format(bytes).ok()
}
