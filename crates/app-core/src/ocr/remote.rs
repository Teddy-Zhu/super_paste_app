use std::{sync::Arc, time::Duration};
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Map, Value};
use super_paste_format::{Preferences, RemoteRequestFormat};
use tokio_util::sync::CancellationToken;
use crate::{Error, Result, images::{EncodedImage, ImageService}};
use super::wire::{Base64Bytes, read_response};

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct RemoteConfig {
    pub url: String, extra: Map<String, Value>, field: String,
    format: RemoteRequestFormat, path: String,
}
impl RemoteConfig {
    pub fn new(settings: &Preferences) -> Result<Arc<Self>> {
        let url = settings.ocr_remote_url.trim().to_owned();
        let parsed = reqwest::Url::parse(&url).map_err(|_| Error::Invalid("远程 OCR 地址格式无效".into()))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(Error::Invalid("远程 OCR 地址必须使用 HTTP 或 HTTPS".into()));
        }
        let extra = match settings.ocr_remote_extra_params.trim() {
            "" => Map::new(),
            source => match serde_json::from_str(source)? {
                Value::Object(values) => values,
                _ => return Err(Error::Invalid("远程 OCR 额外参数必须是 JSON 对象".into())),
            },
        };
        let defaulted = |value: &str, fallback: &str| if value.trim().is_empty() { fallback.to_owned() } else { value.trim().to_owned() };
        Ok(Arc::new(Self { url, extra, field: defaulted(&settings.ocr_remote_image_field_name, "image"),
            format: settings.ocr_remote_request_format, path: defaulted(&settings.ocr_remote_result_path, "data.result") }))
    }
}

pub(super) struct RemoteRecognition { pub text: String, pub details: Value }

pub(super) async fn recognize(client: &reqwest::Client, images: &Arc<ImageService>, config: Arc<RemoteConfig>, image: Arc<EncodedImage>, cancel: &CancellationToken) -> Result<RemoteRecognition> {
    if image.bytes().is_empty() { return Err(Error::Invalid("无效的图片数据".into())); }
    let request = match config.format {
        RemoteRequestFormat::Json => {
            let (payload_config, images) = (config.clone(), images.clone());
            let worker_images = images.clone();
            let body = images.run_cpu(0, cancel, move || {
                let mut writer = worker_images.scratch_writer()?;
                let serialized = serde_json::to_writer(&mut writer, &Payload { config: &payload_config, image: image.bytes() });
                let body = writer.into_bytes()?;
                serialized?;
                Ok(body)
            }).await?;
            client.post(&config.url).header(reqwest::header::CONTENT_TYPE, "application/json").body(body)
        }
        RemoteRequestFormat::Form => {
            let mut form = reqwest::multipart::Form::new();
            for (key, value) in &config.extra { form = form.text(key.clone(), stringify_param(value)); }
            let format = image.format().ok_or_else(|| Error::Invalid("远程 OCR 图片格式无效或不受支持".into()))?;
            let mime = format.to_mime_type();
            let extension = format.extensions_str().first().ok_or_else(|| Error::Invalid("远程 OCR 图片格式无效或不受支持".into()))?;
            let bytes = image.shared_bytes();
            let part = reqwest::multipart::Part::stream_with_length(reqwest::Body::from(bytes), image.bytes().len() as u64)
                .file_name(format!("ocr-image.{extension}")).mime_str(mime).map_err(request_error)?;
            form = form.part(config.field.clone(), part);
            client.post(&config.url).multipart(form)
        }
    }.timeout(TIMEOUT);
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        response = request.send() => response.map_err(request_error)?,
    };
    let status = response.status();
    let bytes = read_response(response, images, cancel, MAX_RESPONSE, "远程 OCR 响应超过大小上限", request_error).await?;
    let parse_config = config.clone();
    images.run_cpu(0, cancel, move || {
        let details: Value = if bytes.is_empty() { Value::Object(Map::new()) }
            else { serde_json::from_slice(&bytes).map_err(|_| Error::Invalid(format!("远程 OCR 响应不是合法 JSON，HTTP 状态: {}", status.as_u16()).into()))? };
        if !status.is_success() {
            let message = details.get("message").and_then(Value::as_str).filter(|s| !s.is_empty())
                .or_else(|| details.get("error").and_then(Value::as_str).filter(|s| !s.is_empty()))
                .map(|message| message.replace(&parse_config.url, "[REDACTED_URL]"))
                .unwrap_or_else(|| format!("远程 OCR 请求失败，HTTP 状态: {}", status.as_u16()));
            return Err(Error::Platform(message.into()));
        }
        let extracted = read_path(&details, &parse_config.path).filter(|v| !v.is_null())
            .ok_or_else(|| Error::Invalid(format!("未找到返回字段: {}", parse_config.path).into()))?;
        let text = normalize_text(extracted);
        if text.trim().is_empty() { return Err(Error::Invalid("远程 OCR 返回内容为空".into())); }
        Ok(RemoteRecognition { text, details })
    }).await
}

struct Payload<'a> { config: &'a RemoteConfig, image: &'a [u8] }
impl Serialize for Payload<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let length = self.config.extra.len() + usize::from(!self.config.extra.contains_key(&self.config.field));
        let mut map = serializer.serialize_map(Some(length))?;
        for (key, value) in &self.config.extra {
            if key != &self.config.field { map.serialize_entry(key, value)?; }
        }
        map.serialize_entry(&self.config.field, &Base64Bytes(self.image))?;
        map.end()
    }
}
fn read_path<'a>(data: &'a Value, path: &str) -> Option<&'a Value> {
    let normalized = path.replace('[', ".").replace(']', "");
    let mut value = data;
    for key in normalized.split('.').map(str::trim).filter(|key| !key.is_empty()) {
        value = match value { Value::Array(values) => values.get(key.parse::<usize>().ok()?), Value::Object(values) => values.get(key), _ => None }?;
    }
    Some(value)
}
fn normalize_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(), Value::Null => String::new(),
        Value::Array(values) => values.iter().map(normalize_text).collect::<Vec<_>>().join("\n"),
        _ => value.to_string(),
    }
}
fn stringify_param(value: &Value) -> String {
    match value { Value::String(text) => text.clone(), Value::Null => String::new(), _ => value.to_string() }
}
fn request_error(error: reqwest::Error) -> Error {
    Error::Platform(if error.is_timeout() { "远程 OCR 请求超时".into() } else { format!("远程 OCR 请求失败: {}", error.without_url()).into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Request};
    use crate::{images::ImageBudgets, storage::Storage};
    use image::{DynamicImage, ImageFormat};
    use std::{io::Cursor, path::PathBuf, sync::Mutex};

    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) { std::fs::remove_dir_all(&self.0).expect("remove isolated remote OCR directory"); }
    }

    #[tokio::test]
    async fn multipart_preserves_original_formats_bytes_parameters_and_result_path() {
        let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-remote-ocr-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&directory.0).expect("isolated directory");
        let storage = Storage::open(directory.0.join("images.sqlite3")).await.expect("database");
        let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets::default()).expect("images");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let router = Router::new().fallback(move |request: Request| {
            let received = received.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = axum::body::to_bytes(body, 64 * 1024).await.expect("tiny multipart image request");
                received.lock().expect("request log").push((parts.headers, body));
                Json(serde_json::json!({ "payload": { "lines": [{ "text": "识别结果 😀" }] } }))
            }
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("loopback HTTP listener");
        let url = format!("http://{}/ocr", listener.local_addr().expect("listener address"));
        let _server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move { axum::serve(listener, router).await.expect("HTTP server"); }));
        let config = RemoteConfig::new(&Preferences { ocr_remote_url: url, ocr_remote_request_format: RemoteRequestFormat::Form,
            ocr_remote_image_field_name: "scan".into(), ocr_remote_extra_params: r#"{"language":"zh","count":2}"#.into(),
            ocr_remote_result_path: "payload.lines[0].text".into(), ..Default::default() }).expect("remote OCR configuration");
        let client = reqwest::Client::new();
        let cancel = CancellationToken::new();
        let formats = [
            (ImageFormat::Png, "png", "image/png"), (ImageFormat::Jpeg, "jpg", "image/jpeg"),
            (ImageFormat::Gif, "gif", "image/gif"), (ImageFormat::WebP, "webp", "image/webp"),
            (ImageFormat::Bmp, "bmp", "image/bmp"), (ImageFormat::Tiff, "tiff", "image/tiff"),
            (ImageFormat::Ico, "ico", "image/x-icon"),
        ];
        for (index, (format, extension, mime)) in formats.into_iter().enumerate() {
            let fixture = if format == ImageFormat::Jpeg { DynamicImage::new_rgb8(1, 1) } else { DynamicImage::new_rgba8(1, 1) };
            let mut data = Cursor::new(Vec::new());
            fixture.write_to(&mut data, format).expect("valid original image");
            let data = data.into_inner();
            let original = images.import_encoded(data.clone(), &cancel).await.expect("original image");
            assert_eq!(original.format(), Some(format));
            let result = tokio::time::timeout(Duration::from_secs(3), recognize(&client, &images, config.clone(), original, &cancel)).await.expect("remote OCR completes").expect("remote OCR result");
            assert_eq!(result.text, "识别结果 😀");
            assert_eq!(result.details["payload"]["lines"][0]["text"], result.text);
            let requests = requests.lock().expect("request log");
            assert_eq!(requests.len(), index + 1);
            let (headers, body) = &requests[index];
            assert!(headers.get("content-type").expect("multipart content type").to_str().expect("ASCII content type").starts_with("multipart/form-data; boundary="));
            assert_eq!(headers.get("content-length").expect("known original byte length").to_str().expect("ASCII length").parse::<usize>().expect("length"), body.len());
            let text = String::from_utf8_lossy(body);
            assert!(text.contains(&format!("name=\"scan\"; filename=\"ocr-image.{extension}\"")));
            assert!(text.to_ascii_lowercase().contains(&format!("content-type: {mime}\r\n")));
            assert!(text.contains("name=\"language\"\r\n\r\nzh\r\n"));
            assert!(text.contains("name=\"count\"\r\n\r\n2\r\n"));
            assert!(body.windows(data.len()).any(|bytes| bytes == data), "original encoded bytes must not be transcoded or Base64-wrapped");
        }
        let unknown = images.import_encoded(vec![1, 2, 3, 4], &cancel).await.expect("unclassified encoded payload");
        let result = recognize(&client, &images, config, unknown, &cancel).await;
        assert!(matches!(result, Err(Error::Invalid(message)) if message.as_ref() == "远程 OCR 图片格式无效或不受支持"));
        assert_eq!(requests.lock().expect("request log").len(), formats.len(), "unknown bytes must not be uploaded as PNG");
        assert_eq!(images.memory().scratch, 0);
        images.shutdown().await.expect("shutdown images");
        storage.close_for_test().await.expect("close database");
    }
}
