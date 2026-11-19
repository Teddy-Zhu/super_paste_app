// WebDAV v1 paths and conditional-write rules follow Super Paste's Rust service.
use std::{sync::Arc, time::Duration};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::{Client, Method, Response, StatusCode, Url, header::{CONTENT_TYPE, ETAG, HeaderMap, HeaderValue, IF_MATCH, IF_NONE_MATCH}};
use super_paste_format::{Digest, WebDavConfig};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use crate::{Error, Result, credentials::Secret};
use super::document::{self, Document, MAX_DOCUMENT_BYTES};

pub(super) struct Target {
    client: Client, username: String, password: Arc<Secret>, timeout: Duration,
    collections: Vec<Url>, pub document: Url, pub key: Digest,
}
pub(super) struct ConditionalTest { pub supported: Option<bool>, pub result: Result<()> }
impl Target {
    pub fn new(client: Client, config: &WebDavConfig, password: Arc<Secret>) -> Result<Self> {
        let mut base = Url::parse(config.server_url.trim()).map_err(|_| invalid("WebDAV 服务器地址无效"))?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() || !base.username().is_empty() || base.password().is_some() || base.query().is_some() || base.fragment().is_some() {
            return Err(invalid("WebDAV 地址必须是无凭据的 HTTP 或 HTTPS URL"));
        }
        if config.timeout == 0 || config.timeout > 300_000 { return Err(invalid("WebDAV 超时必须在 1 到 300000 毫秒之间")); }
        if config.username.trim().is_empty() || password.expose().is_empty() { return Err(invalid("WebDAV 用户名和密码不能为空")); }
        let mut collections = Vec::new();
        for segment in config.remote_path.trim().split('/').filter(|segment| !segment.is_empty()).chain(["super_paste", "v1"]) {
            if segment == "." || segment == ".." || segment.contains('\\') { return Err(invalid("WebDAV 远程路径包含无效目录段")); }
            append(&mut base, segment)?;
            let mut collection = base.clone();
            collection.path_segments_mut().map_err(|_| invalid("WebDAV URL 不支持路径段"))?.push("");
            collections.push(collection);
        }
        append(&mut base, "config.json")?;
        let username = config.username.trim().to_owned();
        let key = document::digest(format!("{base}\0{username}").as_bytes());
        Ok(Self { client, username, password, timeout: Duration::from_millis(config.timeout), collections, document: base, key })
    }
    pub async fn send(&self, method: Method, url: &Url, headers: HeaderMap, body: Option<Bytes>, cancel: &CancellationToken) -> Result<Response> {
        self.send_with_timeout(method, url, headers, body, cancel, self.timeout).await
    }
    async fn send_with_timeout(&self, method: Method, url: &Url, headers: HeaderMap, body: Option<Bytes>, cancel: &CancellationToken, timeout: Duration) -> Result<Response> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let mut request = self.client.request(method, url.clone()).timeout(timeout).headers(headers).basic_auth(&self.username, Some(self.password.expose()));
        if let Some(body) = body { request = request.body(body); }
        tokio::select! { biased; _ = cancel.cancelled() => Err(Error::Cancelled), result = request.send() => result.map_err(network_error) }
    }
    pub async fn ensure_collection(&self, cancel: &CancellationToken) -> Result<()> {
        let method = Method::from_bytes(b"MKCOL").map_err(|_| invalid("WebDAV 方法无效"))?;
        for collection in &self.collections {
            let response = self.send(method.clone(), collection, HeaderMap::new(), None, cancel).await?;
            if !matches!(response.status(), StatusCode::CREATED | StatusCode::METHOD_NOT_ALLOWED | StatusCode::OK | StatusCode::NO_CONTENT) { return Err(http_error("创建目录", response.status())); }
        }
        Ok(())
    }
    pub async fn read(&self, cancel: &CancellationToken) -> Result<Option<Document>> {
        let response = self.send(Method::GET, &self.document, HeaderMap::new(), None, cancel).await?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None); }
        if !response.status().is_success() { return Err(http_error("下载", response.status())); }
        if response.content_length().is_some_and(|length| length > MAX_DOCUMENT_BYTES as u64) { return Err(Error::ResourceLimit("远端配置超过文档大小上限".into())); }
        let etag = response.headers().get(ETAG).and_then(|value| value.to_str().ok()).filter(|value| strong_etag(value)).map(str::to_owned);
        let mut stream = response.bytes_stream(); let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! { biased; _ = cancel.cancelled() => return Err(Error::Cancelled), chunk = stream.next() => chunk };
            let Some(chunk) = chunk else { break; }; let chunk = chunk.map_err(network_error)?;
            if bytes.len().saturating_add(chunk.len()) > MAX_DOCUMENT_BYTES { return Err(Error::ResourceLimit("远端配置超过文档大小上限".into())); }
            bytes.extend_from_slice(&chunk);
        }
        Document::decode(&bytes, etag).map(Some)
    }
    pub async fn test_conditionals(&self, cancel: &CancellationToken) -> ConditionalTest {
        if let Err(error) = self.ensure_collection(cancel).await { return ConditionalTest { supported: None, result: Err(error) }; }
        let Some(mut probe) = self.collections.last().cloned() else { return ConditionalTest { supported: None, result: Err(invalid("WebDAV 同步目录未初始化")) }; };
        if let Err(error) = append(&mut probe, &format!(".probe-{}.json", Uuid::new_v4())) { return ConditionalTest { supported: None, result: Err(error) }; }
        let mut cleanup_required = false;
        let result = self.probe(&probe, cancel, &mut cleanup_required).await;
        let supported = match &result { Ok(()) => Some(true), Err(Error::Invalid(_)) => Some(false), _ => None };
        // Only the UUID probe is temporary. Deleting a just-created collection
        // is unsafe: another client may have populated it in the meantime.
        let cleanup = if cleanup_required { self.send_with_timeout(Method::DELETE, &probe, HeaderMap::new(), None, &CancellationToken::new(), Duration::from_secs(3)).await
            .and_then(|response| if response.status().is_success() || response.status() == StatusCode::NOT_FOUND { Ok(()) } else { Err(http_error("清理测试对象", response.status())) }) } else { Ok(()) };
        let result = match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()), (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(Error::Platform(format!("条件写入测试通过，但临时对象清理失败：{error}").into())),
            (Err(error), Err(cleanup)) => Err(Error::Platform(format!("{error}；临时对象清理失败：{cleanup}").into())),
        };
        ConditionalTest { supported, result }
    }
    async fn probe(&self, url: &Url, cancel: &CancellationToken, cleanup_required: &mut bool) -> Result<()> {
        let body = Bytes::from_static(br#"{"probe":"super_paste"}"#);
        let create_headers = HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("application/json")), (IF_NONE_MATCH, HeaderValue::from_static("*"))]);
        *cleanup_required = true;
        let first = self.send(Method::PUT, url, create_headers.clone(), Some(body.clone()), cancel).await?;
        if !first.status().is_success() { *cleanup_required = false; return Err(http_error("条件创建", first.status())); }
        let etag = response_etag(&first).ok_or_else(|| invalid("WebDAV 服务未返回强 ETag，无法安全执行双向同步"))?.to_owned();
        let repeated = self.send(Method::PUT, url, create_headers, Some(body.clone()), cancel).await?;
        if repeated.status() != StatusCode::PRECONDITION_FAILED { return Err(invalid("WebDAV 服务未遵守 If-None-Match 条件写入")); }
        let wrong = self.send(Method::PUT, url, HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("application/json")), (IF_MATCH, HeaderValue::from_static("\"super-paste-invalid-etag\""))]), Some(body), cancel).await?;
        if wrong.status() != StatusCode::PRECONDITION_FAILED { return Err(invalid("WebDAV 服务未遵守 If-Match 条件写入")); }
        let updated = self.send(Method::PUT, url, HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("application/json")), (IF_MATCH, HeaderValue::from_str(&etag).map_err(|_| invalid("WebDAV ETag 无效"))?)]),
            Some(Bytes::from_static(br#"{"probe":"super_paste","updated":true}"#)), cancel).await?;
        if !updated.status().is_success() { return Err(http_error("条件更新", updated.status())); }
        let updated_etag = response_etag(&updated).ok_or_else(|| invalid("WebDAV 条件更新后未返回强 ETag"))?;
        if updated_etag == etag { return Err(invalid("WebDAV 服务更新内容后仍返回相同强 ETag")); }
        Ok(())
    }
}
fn append(url: &mut Url, segment: &str) -> Result<()> { url.path_segments_mut().map_err(|_| invalid("WebDAV URL 不支持路径段"))?.pop_if_empty().push(segment); Ok(()) }
fn response_etag(response: &Response) -> Option<&str> { response.headers().get(ETAG).and_then(|value| value.to_str().ok()).filter(|value| strong_etag(value)) }
pub(super) fn strong_etag(value: &str) -> bool { value.len() >= 2 && value.starts_with('"') && value.ends_with('"') && value.as_bytes()[1..value.len() - 1].iter().all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte)) }
fn invalid(message: &'static str) -> Error { Error::Invalid(message.into()) }
fn http_error(operation: &str, status: StatusCode) -> Error { Error::Platform(format!("WebDAV {operation}失败，HTTP {status}").into()) }
fn network_error(error: reqwest::Error) -> Error {
    if error.is_timeout() { Error::Platform("WebDAV 请求超时".into()) } else { Error::Platform(format!("WebDAV 请求失败：{}", error.without_url()).into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, extract::Request, response::Response};
    use std::sync::Mutex;

    async fn serve(router: Router) -> (String, tokio_util::task::AbortOnDropHandle<()>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("loopback listener");
        let url = format!("http://{}/dav/", listener.local_addr().expect("listener address"));
        let server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            axum::serve(listener, router).await.expect("loopback HTTP server");
        }));
        (url, server)
    }

    fn target(url: String) -> Target {
        Target::new(Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("HTTP client"),
            &WebDavConfig { server_url: url, username: " user ".into(), remote_path: "clipboard backups".into(), ..Default::default() },
            Arc::new(Secret::new("pass".into()))).expect("WebDAV target")
    }

    #[tokio::test]
    async fn conditionals_use_scoped_paths_authentication_and_cleanup() {
        for obeys in [true, false] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let received = requests.clone();
            let router = Router::new().fallback(move |request: Request| {
                let received = received.clone();
                async move {
                    let (parts, body) = request.into_parts();
                    let body = axum::body::to_bytes(body, 1024).await.expect("small probe body");
                    let index = {
                        let mut requests = received.lock().expect("request log");
                        let index = requests.len();
                        requests.push((parts.method, parts.uri.path().to_owned(), parts.headers, body));
                        index
                    };
                    let (status, etag) = match index {
                        0..=2 => (StatusCode::CREATED, None),
                        3 => (StatusCode::CREATED, Some("\"one\"")),
                        4 => (if obeys { StatusCode::PRECONDITION_FAILED } else { StatusCode::OK }, None),
                        5 if !obeys => (StatusCode::NO_CONTENT, None),
                        5 => (StatusCode::PRECONDITION_FAILED, None),
                        6 => (StatusCode::OK, Some("\"two\"")),
                        7 => (StatusCode::NO_CONTENT, None),
                        _ => panic!("unexpected WebDAV request"),
                    };
                    let mut response = Response::builder().status(status);
                    if let Some(etag) = etag { response = response.header(ETAG, etag); }
                    response.body(Body::empty()).expect("probe response")
                }
            });
            let (url, _server) = serve(router).await;
            let target = target(url);
            let test = tokio::time::timeout(Duration::from_secs(3), target.test_conditionals(&CancellationToken::new())).await.expect("conditional probe completes");
            assert_eq!(test.supported, Some(obeys));
            if obeys { test.result.expect("server supports conditional writes"); }
            else { assert!(matches!(test.result, Err(Error::Invalid(_)))); }
            let requests = requests.lock().expect("request log");
            assert_eq!(requests.len(), if obeys { 8 } else { 6 });
            for (index, path) in ["/dav/clipboard%20backups/", "/dav/clipboard%20backups/super_paste/", "/dav/clipboard%20backups/super_paste/v1/"].into_iter().enumerate() {
                assert_eq!(requests[index].0.as_str(), "MKCOL");
                assert_eq!(requests[index].1, path);
            }
            let probe = &requests[3].1;
            assert!(probe.starts_with("/dav/clipboard%20backups/super_paste/v1/.probe-"));
            assert!(probe.ends_with(".json"));
            for (method, path, headers, _) in requests.iter() {
                assert_eq!(headers.get("authorization").expect("Basic auth"), "Basic dXNlcjpwYXNz");
                if method.as_str() != "MKCOL" { assert_eq!(path, probe, "only the UUID probe may be modified or deleted"); }
            }
            assert_eq!(requests[3].0, Method::PUT);
            assert_eq!(requests[3].2.get(IF_NONE_MATCH).expect("conditional create"), "*");
            assert_eq!(requests[4].2.get(IF_NONE_MATCH).expect("repeat conditional create"), "*");
            assert_eq!(requests.last().expect("cleanup").0, Method::DELETE);
            if obeys {
                assert_eq!(requests[5].2.get(IF_MATCH).expect("wrong ETag"), "\"super-paste-invalid-etag\"");
                assert_eq!(requests[6].2.get(IF_MATCH).expect("matching ETag"), "\"one\"");
                assert_ne!(requests[3].3, requests[6].3, "successful update changes the probe body");
            }
        }
    }

    #[tokio::test]
    async fn cancellation_stops_requests_with_delayed_headers_or_unfinished_bodies() {
        for stream_body in [false, true] {
            let entered = Arc::new(tokio::sync::Notify::new());
            let signal = entered.clone();
            let router = Router::new().fallback(move || {
                let signal = signal.clone();
                async move {
                    if !stream_body {
                        signal.notify_one();
                        return std::future::pending::<Response>().await;
                    }
                    let body = futures_util::stream::once(async move {
                        signal.notify_one();
                        Ok::<_, std::convert::Infallible>(Bytes::from_static(b"{"))
                    }).chain(futures_util::stream::pending());
                    Response::new(Body::from_stream(body))
                }
            });
            let (url, _server) = serve(router).await;
            let target = target(url);
            let cancel = CancellationToken::new();
            let _cancel_on_drop = cancel.clone().drop_guard();
            let caller = cancel.clone();
            let mut request = tokio::spawn(async move { target.read(&caller).await });
            tokio::time::timeout(Duration::from_secs(3), entered.notified()).await.expect("HTTP request reached the waiting stage");
            cancel.cancel();
            let result = tokio::time::timeout(Duration::from_secs(3), &mut request).await.expect("cancel without waiting for HTTP timeout").expect("request task");
            assert!(matches!(result, Err(Error::Cancelled)));
        }
    }
}
