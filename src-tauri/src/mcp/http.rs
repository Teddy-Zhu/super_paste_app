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
use serde_json::{Value, json};
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
        MCP_SESSION_IDLE_SECS, McpSettings,
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
}

struct HttpState {
    settings: McpSettings,
    sessions: Mutex<HashMap<String, SessionActivity>>,
    manager: Arc<LocalSessionManager>,
    requests: Arc<Semaphore>,
    initialize_lock: AsyncMutex<()>,
    shutdown: CancellationToken,
}

pub(crate) struct HttpRuntime {
    pub listener: tokio::net::TcpListener,
    pub router: Router,
    pub cancellation: CancellationToken,
}

impl HttpRuntime {
    pub async fn bind(
        port: u16,
        settings: McpSettings,
        server: Arc<McpServerContext>,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind((MCP_HOST, port))
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AddrInUse {
                    format!("端口 {port} 已被占用，请更换 MCP 端口")
                } else {
                    format!("MCP 服务启动失败：{error}")
                }
            })?;
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
        spawn_session_expirer(state.clone());
        let router = Router::new()
            .route_service(MCP_PATH, any_service(service))
            .fallback(not_found)
            .layer(middleware::from_fn_with_state(state, handle_request));
        Ok(Self {
            listener,
            router,
            cancellation,
        })
    }

    pub async fn serve(self) -> Result<(), String> {
        let listener = self.listener;
        let router = self.router;
        let cancellation = self.cancellation;
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => break,
                accepted = listener.accept() => {
                    let (stream, _) = accepted.map_err(|error| format!("MCP 监听失败：{error}"))?;
                    let router = router.clone();
                    let cancellation = cancellation.clone();
                    connections.spawn(async move {
                        let service = service_fn(move |request: Request<Incoming>| {
                            let router = router.clone();
                            async move { router.oneshot(request.map(Body::new)).await }
                        });
                        let io = TokioIo::new(stream);
                        let mut builder = http1::Builder::new();
                        builder
                            .timer(TokioTimer::new())
                            .header_read_timeout(HEADER_TIMEOUT);
                        let mut shutdown = Box::pin(cancellation.clone().cancelled_owned());
                        let mut shutting_down = false;
                        let mut connection = std::pin::pin!(
                            builder.serve_connection(io, service)
                        );
                        loop {
                            tokio::select! {
                                _ = &mut shutdown, if !shutting_down => {
                                    connection.as_mut().graceful_shutdown();
                                    shutting_down = true;
                                }
                                _ = &mut connection => break,
                            }
                        }
                    });
                }
                _ = connections.join_next(), if !connections.is_empty() => {}
            }
        }
        while connections.join_next().await.is_some() {}
        Ok(())
    }
}

async fn not_found(request: Request<Body>) -> Response<Body> {
    error_response(request, StatusCode::NOT_FOUND, "接口不存在", -32000)
}

async fn handle_request(
    State(state): State<Arc<HttpState>>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
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
            &state.settings.api_key,
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
    let mut parsed_body = None;
    let request_body = if is_post {
        match read_post_body(body, parts.headers.get(CONTENT_LENGTH), &state.shutdown).await {
            Ok(bytes) => {
                match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => parsed_body = Some(value),
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
    let request = Request::from_parts(parts, request_body);
    let is_delete = request.method() == Method::DELETE;
    let is_initialize = request.method() == Method::POST
        && parsed_body
            .as_ref()
            .and_then(|body| body.get("method"))
            .and_then(Value::as_str)
            == Some("initialize");
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

    if is_delete {
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
        })
    }

    fn register_session(self: &Arc<Self>, session_id: String) -> SessionRequestGuard {
        let activity = SessionActivity {
            last_activity: Instant::now(),
            active_requests: 1,
            generation: 1,
            expiry: CancellationToken::new(),
        };
        self.sessions
            .lock()
            .expect("MCP session map poisoned")
            .insert(session_id.clone(), activity);
        SessionRequestGuard {
            state: self.clone(),
            session_id,
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
        tokio::spawn(async move {
            tokio::select! {
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
                    sessions.remove(&session_id);
                    true
                }
            };
            if should_close {
                let _ = state.close_session(&session_id).await;
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
        }
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
}

impl Drop for SessionRequestGuard {
    fn drop(&mut self) {
        self.state.end_session_request(&self.session_id);
    }
}

fn spawn_session_expirer(state: Arc<HttpState>) {
    tokio::spawn(async move {
        state.shutdown.cancelled().await;
        let session_ids = state
            .sessions
            .lock()
            .expect("MCP session map poisoned")
            .drain()
            .map(|(id, activity)| {
                activity.expiry.cancel();
                id
            })
            .collect::<Vec<_>>();
        for session_id in session_ids {
            let id: SessionId = session_id.into();
            let _ = state.manager.close_session(&id).await;
        }
    });
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

fn valid_bearer(header: Option<&HeaderValue>, key: &str) -> bool {
    let supplied = header
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let expected = format!("Bearer {key}");
    let supplied_digest = Sha256::digest(supplied.as_bytes());
    let expected_digest = Sha256::digest(expected.as_bytes());
    bool::from(supplied_digest.as_slice().ct_eq(expected_digest.as_slice()))
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
