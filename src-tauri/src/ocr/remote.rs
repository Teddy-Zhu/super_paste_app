use serde_json::{Map, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RemoteConfig {
    pub url: String,
    pub extra_params: Map<String, Value>,
    pub image_field_name: String,
    pub request_format: RemoteRequestFormat,
    pub result_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RemoteRequestFormat {
    Json,
    Form,
}

#[derive(Clone, Debug)]
pub(super) struct RemoteRecognition {
    pub text: String,
    pub details: Value,
}

impl RemoteConfig {
    pub(super) fn from_settings(settings: &Value) -> Result<Self, String> {
        let nested = settings.get("ocrRemoteConfig").unwrap_or(&Value::Null);
        let string = |nested_key: &str, flat_key: &str, default: &str| {
            nested
                .get(nested_key)
                .or_else(|| settings.get(flat_key))
                .and_then(Value::as_str)
                .unwrap_or(default)
                .trim()
                .to_owned()
        };
        let url = string("url", "ocrRemoteUrl", "");
        let extra_params = match string("extraParams", "ocrRemoteExtraParams", "{}").as_str() {
            "" => Map::new(),
            source => match serde_json::from_str::<Value>(source) {
                Ok(Value::Object(values)) => values,
                Ok(_) => return Err("远程 OCR 额外参数必须是 JSON 对象".to_owned()),
                Err(error) => return Err(format!("远程 OCR 额外参数 JSON 解析失败: {error}")),
            },
        };
        let image_field_name = string("imageFieldName", "ocrRemoteImageFieldName", "image");
        let image_field_name = if image_field_name.is_empty() {
            "image".to_owned()
        } else {
            image_field_name
        };
        let request_format =
            match string("requestFormat", "ocrRemoteRequestFormat", "json").as_str() {
                "json" => RemoteRequestFormat::Json,
                "form" => RemoteRequestFormat::Form,
                _ => return Err("远程 OCR 请求格式无效".to_owned()),
            };
        let result_path = string("resultPath", "ocrRemoteResultPath", "data.result");
        let result_path = if result_path.is_empty() {
            "data.result".to_owned()
        } else {
            result_path
        };
        if url.is_empty() {
            return Err("远程 OCR 地址不能为空".to_owned());
        }
        let parsed = reqwest::Url::parse(&url).map_err(|_| "远程 OCR 地址格式无效".to_owned())?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err("远程 OCR 地址必须使用 HTTP 或 HTTPS".to_owned());
        }
        Ok(Self {
            url,
            extra_params,
            image_field_name,
            request_format,
            result_path,
        })
    }
}

pub(super) async fn recognize(
    client: &reqwest::Client,
    config: &RemoteConfig,
    image: Vec<u8>,
    cancel: &CancellationToken,
) -> Result<RemoteRecognition, String> {
    if image.is_empty() {
        return Err("无效的图片数据".to_owned());
    }
    let mime = image_mime(&image);
    let request = match config.request_format {
        RemoteRequestFormat::Json => {
            let mut body = config.extra_params.clone();
            body.insert(
                config.image_field_name.clone(),
                Value::String(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &image,
                )),
            );
            client
                .post(&config.url)
                .timeout(REQUEST_TIMEOUT)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&Value::Object(body))
        }
        RemoteRequestFormat::Form => {
            let mut form = reqwest::multipart::Form::new();
            for (key, value) in &config.extra_params {
                form = form.text(key.clone(), stringify_param(value));
            }
            let file_name = format!("ocr-image.{}", mime_extension(mime));
            let part = reqwest::multipart::Part::bytes(image)
                .file_name(file_name)
                .mime_str(mime)
                .map_err(|error| format!("远程 OCR 图片格式无效: {error}"))?;
            form = form.part(config.image_field_name.clone(), part);
            client
                .post(&config.url)
                .timeout(REQUEST_TIMEOUT)
                .multipart(form)
        }
    };
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err("OCR 任务已取消".to_owned()),
        result = request.send() => result.map_err(format_request_error)?,
    };
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("远程 OCR 响应超过大小上限".to_owned());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Err("OCR 任务已取消".to_owned()),
            next = futures_util::StreamExt::next(&mut stream) => next,
        };
        let Some(next) = next else { break };
        let chunk = next.map_err(format_request_error)?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("远程 OCR 响应超过大小上限".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = if bytes.is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|_| format!("远程 OCR 响应不是合法 JSON，HTTP 状态: {}", status.as_u16()))?
    };
    if !status.is_success() {
        let message = data
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .or_else(|| {
                data.get("error")
                    .and_then(Value::as_str)
                    .filter(|message| !message.is_empty())
            })
            .map(|message| redact_url(message, &config.url))
            .unwrap_or_else(|| format!("远程 OCR 请求失败，HTTP 状态: {}", status.as_u16()));
        return Err(message);
    }
    let extracted = read_result_by_path(&data, &config.result_path)
        .filter(|value| !value.is_null())
        .ok_or_else(|| format!("未找到返回字段: {}", config.result_path))?;
    let text = normalize_text(extracted);
    if text.trim().is_empty() {
        return Err("远程 OCR 返回内容为空".to_owned());
    }
    Ok(RemoteRecognition {
        text,
        details: data,
    })
}

fn read_result_by_path<'a>(data: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(data);
    }
    let normalized = path.replace('[', ".").replace(']', "");
    let mut current = data;
    for key in normalized
        .split('.')
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        current = match current {
            Value::Array(values) => values.get(key.parse::<usize>().ok()?)?,
            Value::Object(values) => values.get(key)?,
            _ => return None,
        };
    }
    Some(current)
}

fn normalize_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(_) | Value::Bool(_) => value.to_string(),
        Value::Array(values) => values
            .iter()
            .map(normalize_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => value.to_string(),
        Value::Null => String::new(),
    }
}

fn stringify_param(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(_) | Value::Bool(_) => value.to_string(),
        Value::Null => String::new(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn image_mime(image: &[u8]) -> &'static str {
    if image.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        "image/png"
    } else if image.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if image.starts_with(b"GIF87a") || image.starts_with(b"GIF89a") {
        "image/gif"
    } else if image.starts_with(b"RIFF") && image.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if image.starts_with(b"BM") {
        "image/bmp"
    } else {
        "image/png"
    }
}

fn mime_extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "png",
    }
}

fn format_request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "远程 OCR 请求超时".to_owned()
    } else {
        format!("远程 OCR 请求失败: {}", error.without_url())
    }
}

fn redact_url(message: &str, url: &str) -> String {
    message.replace(url, "[REDACTED_URL]")
}
