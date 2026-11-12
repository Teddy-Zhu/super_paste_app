use super::document::{MAX_DOCUMENT_BYTES, RemoteEnvelope, decode_envelope};
use futures_util::StreamExt;
use reqwest::{
    Client, Method, Response, StatusCode, Url,
    header::{CONTENT_TYPE, ETAG, HeaderMap, HeaderValue, IF_MATCH, IF_NONE_MATCH},
};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const DOCUMENT_PATH: [&str; 3] = ["super_paste", "v1", "config.json"];
const MAX_TIMEOUT_MS: u64 = 300_000;

pub(super) struct RemoteDocument {
    pub(super) envelope: RemoteEnvelope,
    pub(super) etag: Option<String>,
}

#[derive(Clone)]
pub(super) struct WebDavConfig {
    server_url: String,
    username: String,
    password: String,
    remote_path: String,
    timeout_ms: u64,
}

pub(super) struct WebDavTarget {
    client: Client,
    username: String,
    password: String,
    collection: Url,
    collections: Vec<Url>,
    pub(super) document: Url,
    pub(super) target_key: String,
    created_collections: Vec<Url>,
}

pub(super) fn parse_webdav_config(value: &Value) -> Result<WebDavConfig, String> {
    let server_url = value
        .get("serverUrl")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if server_url.is_empty() {
        return Err("WebDAV 服务器地址不能为空".to_owned());
    }
    let username = optional_string(value, "username")?.unwrap_or_default();
    let password = optional_string(value, "password")?.unwrap_or_default();
    let remote_path = optional_string(value, "remotePath")?.unwrap_or_default();
    let timeout_ms = match value.get("timeout") {
        Some(timeout) => timeout
            .as_u64()
            .ok_or_else(|| "WebDAV 超时设置无效".to_owned())?,
        None => 30_000,
    };
    if timeout_ms == 0 || timeout_ms > MAX_TIMEOUT_MS {
        return Err(format!("WebDAV 超时必须在 1 到 {MAX_TIMEOUT_MS} 毫秒之间"));
    }
    Ok(WebDavConfig {
        server_url,
        username,
        password,
        remote_path,
        timeout_ms,
    })
}

fn optional_string(value: &Value, field: &str) -> Result<Option<String>, String> {
    value
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("WebDAV 字段无效: {field}"))
        })
        .transpose()
}

impl WebDavTarget {
    pub(super) fn new(config: WebDavConfig) -> Result<Self, String> {
        let mut base =
            Url::parse(&config.server_url).map_err(|_| "WebDAV 服务器地址无效".to_owned())?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err("WebDAV 地址必须是无凭据的 HTTP 或 HTTPS URL".to_owned());
        }
        let mut collections = Vec::new();
        let mut segments = Vec::new();
        for segment in config.remote_path.split('/') {
            if segment.is_empty() {
                continue;
            }
            if segment == "." || segment == ".." || segment.contains('\\') {
                return Err("WebDAV 远程路径包含无效目录段".to_owned());
            }
            segments.push(segment.to_owned());
        }
        segments.extend(DOCUMENT_PATH[..2].iter().map(|part| (*part).to_owned()));
        for segment in &segments {
            append_url_segment(&mut base, segment)?;
            collections.push(base.clone());
        }
        let collection = base;
        let mut document = collection.clone();
        append_url_segment(&mut document, DOCUMENT_PATH[2])?;
        let target_key =
            data_format::sha256_hex(format!("{}\0{}", document, config.username).as_bytes());
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .connect_timeout(Duration::from_millis(config.timeout_ms.min(15_000)))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("创建 WebDAV 客户端失败: {error}"))?;
        Ok(Self {
            client,
            username: config.username,
            password: config.password,
            collection,
            collections,
            document,
            target_key,
            created_collections: Vec::new(),
        })
    }

    pub(super) async fn send(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        body: Option<Vec<u8>>,
        cancel: &CancellationToken,
    ) -> Result<Response, String> {
        self.send_with_timeout(method, url, headers, body, cancel, None)
            .await
    }

    async fn send_with_timeout(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        body: Option<Vec<u8>>,
        cancel: &CancellationToken,
        timeout: Option<Duration>,
    ) -> Result<Response, String> {
        if cancel.is_cancelled() {
            return Err("同步已取消".to_owned());
        }
        let mut request = self.client.request(method, url.clone()).headers(headers);
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        if !self.username.is_empty() || !self.password.is_empty() {
            request = request.basic_auth(&self.username, Some(&self.password));
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        tokio::select! {
            _ = cancel.cancelled() => Err("同步已取消".to_owned()),
            result = request.send() => result.map_err(|error| {
                if error.is_timeout() {
                    "WebDAV 请求超时".to_owned()
                } else {
                    format!("WebDAV 请求失败: {error}")
                }
            }),
        }
    }

    pub(super) async fn ensure_collection(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let method = Method::from_bytes(b"MKCOL").map_err(|error| error.to_string())?;
        for collection in self.collection_urls() {
            let response = self
                .send(method.clone(), &collection, HeaderMap::new(), None, cancel)
                .await?;
            match response.status() {
                StatusCode::CREATED => self.created_collections.push(collection),
                StatusCode::METHOD_NOT_ALLOWED | StatusCode::OK | StatusCode::NO_CONTENT => {}
                status => return Err(format!("WebDAV 创建目录失败，HTTP {status}")),
            }
        }
        Ok(())
    }

    fn collection_urls(&self) -> Vec<Url> {
        self.collections.clone()
    }

    pub(super) async fn read_document(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Option<RemoteDocument>, String> {
        let response = self
            .send(Method::GET, &self.document, HeaderMap::new(), None, cancel)
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(format!("WebDAV 下载失败，HTTP {}", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DOCUMENT_BYTES as u64)
        {
            return Err("远端配置超过文档大小限制".to_owned());
        }
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| is_strong_etag(value))
            .map(str::to_owned);
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err("同步已取消".to_owned()),
                next = stream.next() => next,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|error| format!("读取远端配置失败: {error}"))?;
            if bytes.len().saturating_add(chunk.len()) > MAX_DOCUMENT_BYTES {
                return Err("远端配置超过文档大小限制".to_owned());
            }
            bytes.extend_from_slice(&chunk);
        }
        let envelope = decode_envelope(&bytes)?;
        Ok(Some(RemoteDocument { envelope, etag }))
    }

    pub(super) async fn test_conditionals(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        self.ensure_collection(cancel).await?;
        let mut probe = self.collection.clone();
        append_url_segment(&mut probe, &format!(".probe-{}.json", Uuid::new_v4()))?;
        let result = self.verify_conditionals_at(&probe, cancel).await;
        let cleanup_cancel = CancellationToken::new();
        let cleanup = self.delete_url(&probe, &cleanup_cancel).await;
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(format!("条件写入测试通过，但临时对象清理失败: {error}")),
            (Err(error), Err(cleanup_error)) => {
                Err(format!("{error}; 临时对象清理失败: {cleanup_error}"))
            }
        }
    }

    async fn verify_conditionals_at(
        &self,
        probe: &Url,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let body = br#"{"probe":"super_paste"}"#.to_vec();
        let mut first_headers = HeaderMap::new();
        first_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        first_headers.insert(IF_NONE_MATCH, HeaderValue::from_static("*"));
        let first = self
            .send(
                Method::PUT,
                &probe,
                first_headers,
                Some(body.clone()),
                cancel,
            )
            .await?;
        if !first.status().is_success() {
            return Err(format!("WebDAV 条件创建失败，HTTP {}", first.status()));
        }
        let etag = first
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| is_strong_etag(value))
            .ok_or_else(|| "WebDAV 服务未返回强 ETag，无法安全执行双向同步".to_owned())?
            .to_owned();
        let create_again = self
            .send(
                Method::PUT,
                probe,
                HeaderMap::from_iter([
                    (CONTENT_TYPE, HeaderValue::from_static("application/json")),
                    (IF_NONE_MATCH, HeaderValue::from_static("*")),
                ]),
                Some(body.clone()),
                cancel,
            )
            .await?;
        if create_again.status() != StatusCode::PRECONDITION_FAILED {
            return Err("WebDAV 服务未遵守 If-None-Match 条件写入".to_owned());
        }
        let wrong_match = HeaderValue::from_static("\"super-paste-invalid-etag\"");
        let wrong_match_response = self
            .send(
                Method::PUT,
                probe,
                HeaderMap::from_iter([
                    (CONTENT_TYPE, HeaderValue::from_static("application/json")),
                    (IF_MATCH, wrong_match),
                ]),
                Some(body.clone()),
                cancel,
            )
            .await?;
        if wrong_match_response.status() != StatusCode::PRECONDITION_FAILED {
            return Err("WebDAV 服务未遵守 If-Match 条件写入".to_owned());
        }
        let updated_body = br#"{"probe":"super_paste","updated":true}"#.to_vec();
        let update = self
            .send(
                Method::PUT,
                probe,
                HeaderMap::from_iter([
                    (CONTENT_TYPE, HeaderValue::from_static("application/json")),
                    (
                        IF_MATCH,
                        HeaderValue::from_str(&etag).map_err(|_| "WebDAV ETag 无效")?,
                    ),
                ]),
                Some(updated_body),
                cancel,
            )
            .await?;
        if !update.status().is_success() {
            return Err(format!("WebDAV 条件更新失败，HTTP {}", update.status()));
        }
        let updated_etag = update
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| is_strong_etag(value))
            .ok_or_else(|| "WebDAV 服务条件更新后未返回强 ETag".to_owned())?;
        if updated_etag == etag {
            return Err("WebDAV 服务更新内容后仍返回相同强 ETag".to_owned());
        }
        Ok(())
    }

    async fn delete_url(&self, url: &Url, cancel: &CancellationToken) -> Result<(), String> {
        let response = self
            .send_with_timeout(
                Method::DELETE,
                url,
                HeaderMap::new(),
                None,
                cancel,
                Some(Duration::from_secs(3)),
            )
            .await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(format!(
                "WebDAV 删除临时对象失败，HTTP {}",
                response.status()
            ))
        }
    }

    pub(super) async fn cleanup_created(
        &mut self,
        _cancel: &CancellationToken,
    ) -> Result<(), String> {
        let mut errors = Vec::new();
        let cleanup_cancel = CancellationToken::new();
        let cleanup = tokio::time::timeout(Duration::from_secs(3), async {
            for url in self.created_collections.iter().rev() {
                if let Err(error) = self.delete_url(url, &cleanup_cancel).await {
                    errors.push(error);
                }
            }
        })
        .await;
        if cleanup.is_err() {
            errors.push("清理新建的 WebDAV 目录超时".to_owned());
        }
        self.created_collections.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

fn append_url_segment(url: &mut Url, segment: &str) -> Result<(), String> {
    url.path_segments_mut()
        .map_err(|_| "WebDAV URL 不支持路径段".to_owned())?
        .pop_if_empty()
        .push(segment);
    Ok(())
}

pub(super) fn is_strong_etag(value: &str) -> bool {
    value.len() >= 2
        && !value.starts_with("W/")
        && value.starts_with('"')
        && value.ends_with('"')
        && !value[1..value.len() - 1].contains('"')
}
