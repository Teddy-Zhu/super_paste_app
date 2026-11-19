// Adapted from Super Paste (MIT); see THIRD-PARTY-NOTICES.md.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{
        HeaderValue, Method, Request, Response, StatusCode,
        header::{
            ALLOW, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN,
            WWW_AUTHENTICATE,
        },
    },
    middleware::{self, Next},
    routing::any_service,
};
use futures_util::{StreamExt, stream};
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use rmcp::transport::streamable_http_server::{
    session::{SessionId, SessionManager, local::LocalSessionManager},
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::{
    sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
    time::{Instant, sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::mcp::{
    server::McpServerContext,
    types::{
        MCP_HOST, MCP_MAX_BODY_BYTES, MCP_MAX_REQUESTS, MCP_MAX_SESSIONS, MCP_PATH,
        MCP_SESSION_IDLE_SECS, RuntimeSettings, SessionCancellation,
    },
};

const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SESSION_HEADER: &str = "mcp-session-id";

#[derive(Clone)]
struct SessionActivity {
    last_activity: Instant,
    active_requests: usize,
    generation: u64,
    expiry: CancellationToken,
    cancellation: CancellationToken,
}

struct HttpState {
    settings: RuntimeSettings,
    sessions: Mutex<HashMap<String, SessionActivity>>,
    manager: Arc<LocalSessionManager>,
    requests: Arc<Semaphore>,
    initialize_lock: AsyncMutex<()>,
    shutdown: CancellationToken,
    tasks: tokio_util::task::TaskTracker,
}

pub(crate) struct HttpRuntime {
    pub listener: Arc<tokio::net::TcpListener>,
    pub router: Router,
    pub cancellation: CancellationToken,
    state: Arc<HttpState>,
    _guard: tokio_util::sync::DropGuard,
}

impl HttpRuntime {
    pub async fn bind_listener(port: u16) -> Result<Arc<tokio::net::TcpListener>, String> {
        tokio::net::TcpListener::bind((MCP_HOST, port)).await.map(Arc::new).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AddrInUse { format!("端口 {port} 已被占用，请更换 MCP 端口") }
            else { format!("MCP 服务启动失败：{error}") }
        })
    }
    pub fn with_listener(listener: Arc<tokio::net::TcpListener>, settings: RuntimeSettings, server: Arc<McpServerContext>, parent: CancellationToken) -> Self {
        let port = settings.port;
        let cancellation = parent.child_token();
        let manager = Arc::new(LocalSessionManager::default());
        let allowed_hosts = [format!("{MCP_HOST}:{port}"), format!("localhost:{port}")];
        let allowed_origins = [
            format!("http://{MCP_HOST}:{port}"),
            format!("http://localhost:{port}"),
        ];
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(true)
            .with_json_response(true)
            .with_max_request_body_bytes(MCP_MAX_BODY_BYTES)
            .with_allowed_hosts(allowed_hosts)
            .with_allowed_origins(allowed_origins)
            .enforce_origin_validation()
            .with_cancellation_token(cancellation.clone());
        let state = Arc::new(HttpState {
            settings,
            sessions: Mutex::new(HashMap::new()),
            manager,
            requests: Arc::new(Semaphore::new(MCP_MAX_REQUESTS)),
            initialize_lock: AsyncMutex::new(()),
            shutdown: cancellation.clone(),
            tasks: tokio_util::task::TaskTracker::new(),
        });
        let state_ref = Arc::downgrade(&state);
        server.set_session_closer(Arc::new(move |session_id| {
            let state_ref = state_ref.clone();
            Box::pin(async move {
                let state = state_ref
                    .upgrade()
                    .ok_or_else(|| "MCP HTTP 服务已停止".to_owned())?;
                state.close_session(&session_id).await
            })
        }));
        let context = server.clone();
        let service = StreamableHttpService::new(
            move || Ok(context.new_session()),
            state.manager.clone(),
            config,
        );
        let router = Router::new()
            .route_service(MCP_PATH, any_service(service))
            .fallback(not_found)
            .layer(middleware::from_fn_with_state(state.clone(), handle_request));
        Self { listener, router, state, _guard: cancellation.clone().drop_guard(), cancellation }
    }

    pub async fn serve(self) -> Result<(), String> {
        let Self { listener, router, cancellation, state, _guard } = self;
        let mut connections = JoinSet::new();
        let capacity = Arc::new(Semaphore::new(MCP_MAX_REQUESTS + MCP_MAX_SESSIONS));
        let mut errors = Vec::new();
        loop {
            while let Some(result) = connections.try_join_next() { if let Err(error) = result { tracing::warn!(%error, "MCP 连接任务异常退出"); } }
            let permit = tokio::select! { biased;
                _ = cancellation.cancelled() => break,
                permit = capacity.clone().acquire_owned() => match permit { Ok(permit) => permit, Err(_) => { errors.push("MCP 连接资源已关闭".into()); break; } },
            };
            let accepted = tokio::select! { biased; _ = cancellation.cancelled() => break, accepted = listener.accept() => accepted };
            let (stream, _) = match accepted { Ok(accepted) => accepted, Err(error) => { errors.push(format!("MCP 监听失败：{error}")); break; } };
            let router = router.clone(); let cancellation = cancellation.clone();
            connections.spawn(async move {
                let _permit = permit;
                let service = service_fn(move |request: Request<Incoming>| {
                    let router = router.clone(); async move { router.oneshot(request.map(Body::new)).await }
                });
                let mut builder = http1::Builder::new(); builder.timer(TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT);
                let mut connection = std::pin::pin!(builder.serve_connection(TokioIo::new(stream), service));
                let result = tokio::select! { biased;
                    _ = cancellation.cancelled() => {
                        connection.as_mut().graceful_shutdown();
                        match timeout(Duration::from_secs(5), &mut connection).await {
                            Ok(result) => result,
                            Err(_) => { tracing::debug!("MCP 连接在关闭期限内未收尾，已关闭连接"); return; },
                        }
                    },
                    result = &mut connection => result,
                };
                if let Err(error) = result { tracing::debug!(%error, "MCP HTTP 连接已结束"); }
            });
        }
        cancellation.cancel();
        let close = state.close_all();
        let drain = async { while let Some(result) = connections.join_next().await { if let Err(error) = result { tracing::warn!(%error, "MCP 连接任务回收失败"); } } };
        let (closed, ()) = tokio::join!(close, drain);
        if let Err(error) = closed { errors.push(error); }
        // An initialization already admitted at shutdown may complete after the
        // first drain. No HTTP request remains before this final session pass.
        if let Err(error) = state.close_all().await { errors.push(error); }
        state.tasks.close(); state.tasks.wait().await;
        if errors.is_empty() { Ok(()) } else { Err(errors.join("；")) }
    }
}

async fn not_found(request: Request<Body>) -> Response<Body> {
    error_response(request, StatusCode::NOT_FOUND, "接口不存在", -32000)
}

/// Inspect only the method while validating JSON syntax. Unknown values are
/// skipped, so a large image string is not materialized before the SDK parses it.
fn initialize_request(bytes: &[u8]) -> Result<bool, serde_json::Error> {
    struct Method;
    impl<'de> serde::de::Visitor<'de> for Method {
        type Value = bool;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str("JSON value") }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<bool, A::Error> {
            let mut initialize = false;
            while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                if key == "method" {
                    let raw = map.next_value::<&'de serde_json::value::RawValue>()?;
                    initialize = raw.get().len() <= 64 && serde_json::from_str::<std::borrow::Cow<'_, str>>(raw.get()).is_ok_and(|method| method == "initialize");
                } else { map.next_value::<serde::de::IgnoredAny>()?; }
            }
            Ok(initialize)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<bool, A::Error> {
            while sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {} Ok(false)
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<bool, E> { Ok(false) }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<bool, E> { Ok(false) }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<bool, E> { Ok(false) }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<bool, E> { Ok(false) }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<bool, E> { Ok(false) }
        fn visit_unit<E: serde::de::Error>(self) -> Result<bool, E> { Ok(false) }
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let result = serde::Deserializer::deserialize_any(&mut deserializer, Method)?;
    deserializer.end()?;
    Ok(result)
}

async fn handle_request(
    State(state): State<Arc<HttpState>>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    if state.shutdown.is_cancelled() { return error_response(request, StatusCode::SERVICE_UNAVAILABLE, "MCP 服务已停止", -32000); }
    if request
        .uri()
        .path_and_query()
        .is_some_and(|path| path.as_str() != MCP_PATH)
    {
        return error_response(request, StatusCode::NOT_FOUND, "接口不存在", -32000);
    }
    if !valid_host(request.headers().get(HOST), state.settings.port) {
        return error_response(request, StatusCode::FORBIDDEN, "不允许的 Host", -32000);
    }
    if !valid_origin(request.headers().get(ORIGIN), state.settings.port) {
        return error_response(request, StatusCode::FORBIDDEN, "不允许的 Origin", -32000);
    }
    if state.settings.auth_enabled
        && !valid_bearer(
            request.headers().get(AUTHORIZATION),
            state.settings.authorization_digest.as_ref(),
        )
    {
        let mut response = error_response(
            request,
            StatusCode::UNAUTHORIZED,
            "API Key 缺失或无效",
            -32000,
        );
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"Super Paste MCP\""),
        );
        return response;
    }
    if !matches!(
        *request.method(),
        Method::POST | Method::GET | Method::DELETE
    ) {
        let mut response = error_response(
            request,
            StatusCode::METHOD_NOT_ALLOWED,
            "请使用 MCP Streamable HTTP POST、GET 或 DELETE 请求",
            -32000,
        );
        response
            .headers_mut()
            .insert(ALLOW, HeaderValue::from_static("POST, GET, DELETE"));
        return response;
    }
    if request.method() == Method::POST
        && !is_json_content_type(request.headers().get(CONTENT_TYPE))
    {
        return error_response(
            request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type 须为 application/json",
            -32000,
        );
    }
    let permit = match state.requests.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error_response(
                request,
                StatusCode::TOO_MANY_REQUESTS,
                "MCP 并发请求过多，请稍后重试",
                -32000,
            );
        }
    };

    let is_post = request.method() == Method::POST;
    let (parts, body) = request.into_parts();
    let mut is_initialize = false;
    let request_body = if is_post {
        match read_post_body(body, parts.headers.get(CONTENT_LENGTH), &state.shutdown).await {
            Ok(bytes) => {
                match initialize_request(&bytes) {
                    Ok(value) => is_initialize = value,
                    Err(_) => {
                        return error_response_from_parts(
                            parts,
                            StatusCode::BAD_REQUEST,
                            "JSON 解析失败",
                            -32700,
                        );
                    }
                }
                Body::from(bytes)
            }
            Err(BodyReadError::TooLarge) => {
                return error_response_from_parts(
                    parts,
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "请求体超过 15 MiB 限制",
                    -32000,
                );
            }
            Err(BodyReadError::Timeout) => {
                return error_response_from_parts(
                    parts,
                    StatusCode::REQUEST_TIMEOUT,
                    "MCP 请求接收超时",
                    -32000,
                );
            }
            Err(BodyReadError::Read) => {
                return error_response_from_parts(
                    parts,
                    StatusCode::BAD_REQUEST,
                    "MCP 请求体无效",
                    -32000,
                );
            }
            Err(BodyReadError::Cancelled) => {
                return error_response_from_parts(
                    parts,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "MCP 服务已停止",
                    -32000,
                );
            }
        }
    } else {
        body
    };
    let mut request = Request::from_parts(parts, request_body);
    let is_delete = request.method() == Method::DELETE;
    let session_id = request
        .headers()
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let initialize_guard = if session_id.is_none() && is_initialize {
        Some(state.initialize_lock.lock().await)
    } else {
        None
    };
    let active_session = if let Some(session_id) = session_id.as_deref() {
        match state.begin_session(session_id) {
            Ok(guard) => Some(guard),
            Err(expired) => {
                if expired {
                    let _ = state.close_session(session_id).await;
                }
                return error_response(
                    request,
                    StatusCode::NOT_FOUND,
                    "MCP 会话已失效，请重新初始化",
                    -32000,
                );
            }
        }
    } else if is_initialize {
        let count = state.manager.sessions.read().await.len();
        if count >= MCP_MAX_SESSIONS {
            return error_response(
                request,
                StatusCode::TOO_MANY_REQUESTS,
                "MCP 会话过多，请关闭闲置连接",
                -32000,
            );
        }
        None
    } else {
        return error_response(
            request,
            StatusCode::BAD_REQUEST,
            "缺少有效的 Mcp-Session-Id，请先初始化 MCP 会话",
            -32000,
        );
    };

    if let Some(session) = &active_session {
        request.extensions_mut().insert(SessionCancellation(session.cancellation.clone()));
    }
    let response = next.run(request).await;
    let response_session_id = response
        .headers()
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if let Some(session_id) = response_session_id.filter(|_| active_session.is_none()) {
        let guard = state.register_session(session_id);
        drop(initialize_guard);
        return attach_guards(response, permit, Some(guard));
    }

    // A rejected DELETE leaves the SDK session alive. Preserve our activity
    // guard and expiry ownership until the transport confirms termination.
    if is_delete && response.status().is_success() {
        if let Some(session_id) = session_id.as_deref() {
            state.remove_session(session_id);
        }
    }
    drop(initialize_guard);
    attach_guards(response, permit, active_session)
}

impl HttpState {
    fn begin_session(self: &Arc<Self>, session_id: &str) -> Result<SessionRequestGuard, bool> {
        let now = Instant::now();
        let mut sessions = self.sessions.lock().expect("MCP session map poisoned");
        let Some(activity) = sessions.get_mut(session_id) else {
            return Err(false);
        };
        if activity.active_requests == 0
            && now.duration_since(activity.last_activity)
                >= Duration::from_secs(MCP_SESSION_IDLE_SECS)
        {
            activity.expiry.cancel();
            activity.cancellation.cancel();
            sessions.remove(session_id);
            return Err(true);
        }
        activity.expiry.cancel();
        activity.generation = activity.generation.wrapping_add(1);
        activity.active_requests += 1;
        activity.last_activity = now;
        Ok(SessionRequestGuard {
            state: self.clone(),
            session_id: session_id.to_owned(),
            cancellation: activity.cancellation.clone(),
        })
    }

    fn register_session(self: &Arc<Self>, session_id: String) -> SessionRequestGuard {
        let cancellation = self.shutdown.child_token();
        let activity = SessionActivity {
            last_activity: Instant::now(),
            active_requests: 1,
            generation: 1,
            expiry: CancellationToken::new(),
            cancellation: cancellation.clone(),
        };
        self.sessions
            .lock()
            .expect("MCP session map poisoned")
            .insert(session_id.clone(), activity);
        SessionRequestGuard {
            state: self.clone(),
            session_id,
            cancellation,
        }
    }

    fn end_session_request(self: &Arc<Self>, session_id: &str) {
        let generation = {
            let now = Instant::now();
            let mut sessions = self.sessions.lock().expect("MCP session map poisoned");
            let Some(activity) = sessions.get_mut(session_id) else {
                return;
            };
            activity.active_requests = activity.active_requests.saturating_sub(1);
            activity.last_activity = now;
            if activity.active_requests != 0 {
                return;
            }
            activity.generation = activity.generation.wrapping_add(1);
            activity.expiry.cancel();
            activity.expiry = CancellationToken::new();
            activity.generation
        };
        self.schedule_expiration(session_id.to_owned(), generation);
    }

    fn schedule_expiration(self: &Arc<Self>, session_id: String, generation: u64) {
        if self.shutdown.is_cancelled() { return; }
        let Some(expiry) = self
            .sessions
            .lock()
            .expect("MCP session map poisoned")
            .get(&session_id)
            .filter(|activity| activity.generation == generation)
            .map(|activity| activity.expiry.clone())
        else {
            return;
        };
        let state = self.clone();
        self.tasks.spawn(async move {
            tokio::select! { biased;
                _ = state.shutdown.cancelled() => return,
                _ = expiry.cancelled() => return,
                _ = sleep(Duration::from_secs(MCP_SESSION_IDLE_SECS)) => {}
            }
            let should_close = {
                let mut sessions = state.sessions.lock().expect("MCP session map poisoned");
                let Some(activity) = sessions.get(&session_id) else {
                    return;
                };
                if activity.generation != generation || activity.active_requests != 0 {
                    false
                } else {
                    activity.cancellation.cancel();
                    sessions.remove(&session_id);
                    true
                }
            };
            if should_close {
                if let Err(error) = state.close_session(&session_id).await { tracing::warn!(%error, "MCP 空闲会话清理失败"); }
            }
        });
    }

    fn remove_session(&self, session_id: &str) {
        if let Some(activity) = self
            .sessions
            .lock()
            .expect("MCP session map poisoned")
            .remove(session_id)
        {
            activity.expiry.cancel();
            activity.cancellation.cancel();
        }
    }

    async fn close_all(&self) -> Result<(), String> {
        {
            let mut sessions = self.sessions.lock().expect("MCP session map poisoned");
            for (_, activity) in sessions.drain() { activity.expiry.cancel(); activity.cancellation.cancel(); }
        }
        let ids = self.manager.sessions.read().await.keys().cloned().collect::<Vec<_>>();
        let results = futures_util::future::join_all(ids.iter().map(|id| self.manager.close_session(id))).await;
        let errors = results.into_iter().filter_map(Result::err).map(|error| error.to_string()).collect::<Vec<_>>();
        if errors.is_empty() { Ok(()) } else { Err(format!("MCP 会话回收失败：{}", errors.join("；"))) }
    }
    async fn close_session(&self, session_id: &str) -> Result<(), String> {
        self.remove_session(session_id);
        let id: SessionId = session_id.to_owned().into();
        self.manager
            .close_session(&id)
            .await
            .map_err(|error| error.to_string())
    }
}

struct SessionRequestGuard {
    state: Arc<HttpState>,
    session_id: String,
    cancellation: CancellationToken,
}

impl Drop for SessionRequestGuard {
    fn drop(&mut self) {
        self.state.end_session_request(&self.session_id);
    }
}

fn attach_guards(
    response: Response<Body>,
    permit: OwnedSemaphorePermit,
    session_guard: Option<SessionRequestGuard>,
) -> Response<Body> {
    let (parts, body) = response.into_parts();
    let stream = stream::unfold(
        (body.into_data_stream(), Some(permit), session_guard),
        |(mut body, permit, session_guard)| async move {
            body.next()
                .await
                .map(|chunk| (chunk, (body, permit, session_guard)))
        },
    );
    let mut response = Response::from_parts(parts, Body::from_stream(stream));
    apply_response_headers(response.headers_mut());
    response
}

async fn read_post_body(
    body: Body,
    content_length: Option<&HeaderValue>,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, BodyReadError> {
    if content_length
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse::<usize>().ok())
        .is_some_and(|length| length > MCP_MAX_BODY_BYTES)
    {
        return Err(BodyReadError::TooLarge);
    }
    let mut stream = body.into_data_stream();
    let mut collected = Vec::new();
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    loop {
        let next = timeout_at_with_idle(deadline, &mut stream, cancellation).await?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|_| BodyReadError::Read)?;
        if collected.len().saturating_add(chunk.len()) > MCP_MAX_BODY_BYTES {
            return Err(BodyReadError::TooLarge);
        }
        collected.extend_from_slice(&chunk);
    }
    Ok(collected)
}

async fn timeout_at_with_idle<S>(
    deadline: Instant,
    stream: &mut S,
    cancellation: &CancellationToken,
) -> Result<Option<S::Item>, BodyReadError>
where
    S: futures_util::Stream + Unpin,
{
    let next = timeout(HEADER_TIMEOUT, stream.next());
    tokio::select! {
        _ = cancellation.cancelled() => Err(BodyReadError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(BodyReadError::Timeout),
        result = next => result.map_err(|_| BodyReadError::Timeout),
    }
}

enum BodyReadError {
    TooLarge,
    Timeout,
    Read,
    Cancelled,
}

fn valid_host(host: Option<&HeaderValue>, port: u16) -> bool {
    let Some(host) = host.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    host == format!("{MCP_HOST}:{port}") || host == format!("localhost:{port}")
}

fn valid_origin(origin: Option<&HeaderValue>, port: u16) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    origin == format!("http://{MCP_HOST}:{port}") || origin == format!("http://localhost:{port}")
}

fn is_json_content_type(content_type: Option<&HeaderValue>) -> bool {
    content_type
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
}

fn valid_bearer(header: Option<&HeaderValue>, expected: Option<&[u8; 32]>) -> bool {
    let Some(expected) = expected else { return false; };
    let supplied = header.and_then(|value| value.to_str().ok()).unwrap_or_default();
    let supplied_digest = Sha256::digest(supplied.as_bytes());
    bool::from(supplied_digest.as_slice().ct_eq(expected.as_slice()))
}

fn error_response(
    request: Request<Body>,
    status: StatusCode,
    message: &str,
    code: i32,
) -> Response<Body> {
    error_response_from_parts(request.into_parts().0, status, message, code)
}

fn error_response_from_parts(
    parts: axum::http::request::Parts,
    status: StatusCode,
    message: &str,
    code: i32,
) -> Response<Body> {
    let mut response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .header(CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .body(Body::from(
            json!({
                "jsonrpc":"2.0",
                "id":null,
                "error":{"code":code,"message":message}
            })
            .to_string(),
        ))
        .expect("valid MCP error response");
    if parts.uri.path() == MCP_PATH {
        apply_response_headers(response.headers_mut());
    }
    response
}

fn apply_response_headers(headers: &mut axum::http::HeaderMap) {
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
}

#[cfg(test)]
mod interop;

#[cfg(test)]
mod tests {
    use super::*;

    struct Handler;
    impl rmcp::handler::server::ServerHandler for Handler {}

    async fn exchange(
        router: &Router,
        method: Method,
        session: Option<&str>,
        version: &str,
        message: serde_json::Value,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut request = Request::builder()
            .method(method)
            .uri(MCP_PATH)
            .header(HOST, "127.0.0.1:19528")
            .header(CONTENT_TYPE, "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", version);
        if let Some(session) = session { request = request.header(SESSION_HEADER, session); }
        let response = router.clone().oneshot(request.body(Body::from(message.to_string())).unwrap())
            .await.expect("dispatch through application middleware and SDK transport");
        let (parts, body) = response.into_parts();
        let body = axum::body::to_bytes(body, 64 * 1024).await.expect("finish response and release activity guards");
        (parts.status, parts.headers, body.to_vec())
    }

    #[tokio::test]
    async fn rejected_delete_preserves_session_until_successful_termination() {
        timeout(Duration::from_secs(5), async {
            // Exercise the real SDK session transport without desktop services,
            // sockets, credentials or user data. Only the RPC handler is empty.
            let shutdown = CancellationToken::new();
            let _guard = shutdown.clone().drop_guard();
            let manager = Arc::new(LocalSessionManager::default());
            let state = Arc::new(HttpState {
                settings: RuntimeSettings {
                    value: super_paste_format::McpSettings { enabled: true, port: 19528, auth_enabled: false },
                    reference: None,
                    authorization_digest: None,
                },
                sessions: Mutex::new(HashMap::new()),
                manager: manager.clone(),
                requests: Arc::new(Semaphore::new(MCP_MAX_REQUESTS)),
                initialize_lock: AsyncMutex::new(()),
                shutdown: shutdown.clone(),
                tasks: tokio_util::task::TaskTracker::new(),
            });
            let config = StreamableHttpServerConfig::default()
                .with_legacy_session_mode(true)
                .with_json_response(true)
                .with_allowed_hosts(["127.0.0.1:19528".to_owned()])
                .with_cancellation_token(shutdown.clone());
            let service = StreamableHttpService::new(|| Ok(Handler), manager.clone(), config);
            let router = Router::new().route_service(MCP_PATH, any_service(service))
                .layer(middleware::from_fn_with_state(state.clone(), handle_request));
            let version = "2025-03-26";
            let (status, headers, _) = exchange(&router, Method::POST, None, version, json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                    "protocolVersion": version, "capabilities": {},
                    "clientInfo": { "name": "session-regression", "version": "1" }
                }
            })).await;
            assert_eq!(status, StatusCode::OK);
            let session = headers.get(SESSION_HEADER).expect("SDK issued a session").to_str().unwrap();
            let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
            assert_eq!(exchange(&router, Method::POST, Some(session), version, initialized).await.0, StatusCode::ACCEPTED);

            let rejected = exchange(&router, Method::DELETE, Some(session), "1990-01-01", serde_json::Value::Null).await;
            assert_eq!(rejected.0, StatusCode::BAD_REQUEST);
            assert_eq!(manager.sessions.read().await.len(), 1, "SDK retained the rejected DELETE's session");
            let ping = json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" });
            let (status, headers, body) = exchange(&router, Method::POST, Some(session), version, ping.clone()).await;
            assert_eq!(status, StatusCode::OK, "a rejected DELETE must not invalidate subsequent requests");
            // Legacy sessions use SSE even when stateless requests prefer JSON.
            let text = std::str::from_utf8(&body).expect("UTF-8 RPC response");
            let payload = if is_json_content_type(headers.get(CONTENT_TYPE)) {
                text
            } else {
                assert!(headers.get(CONTENT_TYPE).unwrap().to_str().unwrap().starts_with("text/event-stream"));
                // The SDK may precede the RPC reply with an empty primer event.
                text.lines().filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                    .find(|payload| !payload.is_empty())
                    .expect("SDK emitted a JSON RPC SSE event")
            };
            let reply: serde_json::Value = serde_json::from_str(payload).expect("JSON RPC ping response");
            assert_eq!(reply["id"], 2);
            assert_eq!(reply["result"], json!({}));

            assert_eq!(exchange(&router, Method::DELETE, Some(session), version, serde_json::Value::Null).await.0, StatusCode::ACCEPTED);
            assert!(manager.sessions.read().await.is_empty(), "successful DELETE closed the SDK session");
            assert_eq!(exchange(&router, Method::POST, Some(session), version, ping).await.0, StatusCode::NOT_FOUND);
            shutdown.cancel();
            state.close_all().await.expect("close remaining sessions");
            state.tasks.close();
            state.tasks.wait().await;
        }).await.expect("session lifecycle completes within five seconds");
    }
}
