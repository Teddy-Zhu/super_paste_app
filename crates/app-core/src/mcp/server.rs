// Adapted from Super Paste (MIT); see THIRD-PARTY-NOTICES.md.
use std::{
    borrow::Cow,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::future::join_all;
use rmcp::{
    ErrorData, RoleServer,
    handler::server::ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode,
        Implementation, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
        ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
        Resource, ResourceContents, ResourceUpdatedNotificationParam, ServerCapabilities,
        ServerConfig, SubscribeRequestParams, Tool, ToolAnnotations, UnsubscribeRequestParams,
    },
    service::{Peer, RequestContext},
};
use serde_json::{Map, Value, json};
use tokio::{sync::watch, time::timeout};
use tokio_util::sync::CancellationToken;

use crate::mcp::{
    broker::{ChangeBroker, CursorState, SessionSubscriptions, resource_update},
    types::{
        MCP_CLIPBOARD_EVENT_META_KEY, MCP_CLIPBOARD_RESOURCE_URI, MCP_MAX_IMAGE_BYTES, McpChange, McpHistorySearch,
    },
};

const SERVER_NAME: &str = "super-paste";
const MAX_IMAGE_BASE64_CHARS: usize = (MCP_MAX_IMAGE_BYTES + 2) / 3 * 4 + 100;
const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) type SessionCloserFuture =
    Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;
pub(crate) type SessionCloser = Arc<dyn Fn(String) -> SessionCloserFuture + Send + Sync>;

pub(crate) struct McpServerContext {
    pub backend: Arc<super::backend::Backend>,
    pub version: String,
    pub changes: Arc<ChangeBroker>,
    pub recognizing: AtomicBool,
    session_closer: Mutex<Option<SessionCloser>>,
}

impl McpServerContext {
    pub fn new(backend: Arc<super::backend::Backend>, version: String) -> Arc<Self> {
        Arc::new(Self {
            backend,
            version,
            changes: ChangeBroker::new(),
            recognizing: AtomicBool::new(false),
            session_closer: Mutex::new(None),
        })
    }

    pub(crate) fn set_session_closer(&self, closer: SessionCloser) {
        *self
            .session_closer
            .lock()
            .expect("MCP session closer lock poisoned") = Some(closer);
    }

    async fn close_session(&self, session_id: String) -> Result<(), String> {
        let closer = self
            .session_closer
            .lock()
            .expect("MCP session closer lock poisoned")
            .clone()
            .ok_or_else(|| "MCP 会话关闭器尚未初始化".to_owned())?;
        closer(session_id).await
    }

    pub async fn notification_worker(self: Arc<Self>, cancellation: CancellationToken) {
        let mut updates = self.changes.subscribe_updates();
        let initial_event = { updates.borrow_and_update().clone() };
        if let Some(event) = initial_event {
            tokio::select! {
                _ = cancellation.cancelled() => return,
                _ = notify_subscribers(&self, &event) => {}
            }
        }
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => break,
                changed = updates.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let event = { updates.borrow_and_update().clone() };
                    let Some(event) = event else {
                        continue;
                    };
                    tokio::select! {
                        _ = cancellation.cancelled() => break,
                        _ = notify_subscribers(&self, &event) => {}
                    }
                }
            }
        }
    }

    pub fn new_session(self: &Arc<Self>) -> McpHandler {
        McpHandler {
            shared: self.clone(),
            subscriptions: SessionSubscriptions::new(&self.changes),
        }
    }
}

pub(super) struct McpHandler {
    shared: Arc<McpServerContext>,
    subscriptions: Arc<SessionSubscriptions>,
}

impl McpHandler {
    fn bind_transport(&self, context: &RequestContext<RoleServer>) {
        if let Some(parts) = context.extensions.get::<axum::http::request::Parts>() {
            if let Some(cancel) = parts.extensions.get::<super::types::SessionCancellation>() {
                self.subscriptions.bind_transport(cancel.0.clone());
            }
        }
    }

    fn read_only_annotations(title: &str) -> ToolAnnotations {
        ToolAnnotations::with_title(title)
            .read_only(true)
            .destructive(false)
            .open_world(false)
    }

    fn tools() -> Vec<Tool> {
        let read_only = |title| Self::read_only_annotations(title);
        vec![
            tool(
                "get_clipboard",
                json!({"includeImage":{"type":"boolean","default":false}}),
                &[],
                read_only("读取剪贴板"),
            ),
            tool(
                "wait_for_clipboard_change",
                json!({
                    "cursor":{"type":"string","maxLength":100},
                    "timeoutMs":{"type":"integer","minimum":0,"maximum":30000,"default":25000}
                }),
                &[],
                read_only("等待剪贴板变化"),
            ),
            tool(
                "subscribe_clipboard_changes",
                json!({}),
                &[],
                read_only("订阅剪贴板变化"),
            ),
            tool(
                "unsubscribe_clipboard_changes",
                json!({}),
                &[],
                read_only("取消剪贴板订阅"),
            ),
            tool(
                "search_clipboard_history",
                json!({
                    "searchText":{"type":"string","maxLength":1000},
                    "limit":{"type":"integer","minimum":1,"maximum":100,"default":20},
                    "offset":{"type":"integer","minimum":0,"maximum":1000000,"default":0}
                }),
                &[],
                read_only("搜索剪贴板历史"),
            ),
            tool(
                "get_clipboard_history_item",
                json!({
                    "itemId":{"type":"string","minLength":1,"maxLength":256},
                    "includeImage":{"type":"boolean","default":false}
                }),
                &["itemId"],
                read_only("读取历史记录"),
            ),
            tool(
                "set_clipboard_text",
                json!({"text":{"type":"string","minLength":1,"maxLength":1000000}}),
                &["text"],
                ToolAnnotations::with_title("写入剪贴板文本")
                    .read_only(false)
                    .destructive(true)
                    .idempotent(true)
                    .open_world(false),
            ),
            tool(
                "ocr_image",
                json!({
                    "imageBase64":{"type":"string","minLength":1,"maxLength":MAX_IMAGE_BASE64_CHARS},
                    "itemId":{"type":"string","minLength":1,"maxLength":256}
                }),
                &[],
                ToolAnnotations::with_title("识别图片文字")
                    .read_only(true)
                    .destructive(false)
                    .open_world(true),
            ),
        ]
    }

    async fn require_clipboard(&self) -> Result<(), String> {
        if !self.shared.backend.clipboard_is_running().await? {
            return Err("剪贴板服务尚未就绪，请检查系统访问权限".to_owned());
        }
        Ok(())
    }

    async fn require_listener(&self) -> Result<(), String> {
        self.require_clipboard().await?;
        if !self.shared.backend.clipboard_is_listening().await? {
            return Err("剪贴板监听未开启，请先在调试设置中启用监听".to_owned());
        }
        Ok(())
    }

    async fn current_clipboard(&self, include_image: bool, request: CancellationToken) -> Result<(Value, Vec<ContentBlock>), String> {
        let cancellation = self.request_cancellation(request);
        self.require_clipboard().await?;
        let cursor = self.shared.changes.cursor();
        let clipboard = self.shared.backend.current_clipboard(&cancellation.token).await?;
        super::codec::clipboard(clipboard, include_image, Some(&cursor), &self.shared.backend.images, &cancellation.token).await
    }

    async fn wait_for_change(
        &self,
        requested_cursor: Option<&str>,
        timeout_ms: u64,
        request_cancel: CancellationToken,
    ) -> Result<Value, String> {
        if request_cancel.is_cancelled() || self.subscriptions.cancellation().is_cancelled() {
            return Err("请求已取消".to_owned());
        }
        let initial = requested_cursor
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.shared.changes.cursor());
        self.require_clipboard().await?;
        if !self.shared.backend.clipboard_is_listening().await? {
            return Err("剪贴板监听未开启，请先在调试设置中启用监听".to_owned());
        }
        self.shared.changes.validate_cursor(&initial)?;

        let mut receiver = self.shared.changes.subscribe_state();
        if !wait_is_changed(&receiver, &initial) && timeout_ms > 0 {
            let deadline = tokio::time::sleep(Duration::from_millis(timeout_ms));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = request_cancel.cancelled() => return Err("请求已取消".to_owned()),
                    _ = self.subscriptions.cancellation().cancelled() => return Err("请求已取消".to_owned()),
                    _ = &mut deadline => break,
                    changed = receiver.changed() => {
                        if changed.is_err() {
                            return Err("MCP 服务已停止".to_owned());
                        }
                        if wait_is_changed(&receiver, &initial) {
                            break;
                        }
                    }
                }
            }
        }

        let state = receiver.borrow().clone();
        Ok(json!({
            "changed": state.cursor != initial,
            "cursor": state.cursor,
            "timestamp": state.timestamp
        }))
    }

    async fn subscribe(
        &self,
        peer: Peer<RoleServer>,
        transport_session_id: Option<String>,
    ) -> Result<(), String> {
        let _lifecycle = self.subscriptions.lifecycle.lock().await;
        if self.subscriptions.cancellation().is_cancelled() {
            return Err("MCP 会话已关闭".to_owned());
        }
        if self.subscriptions.is_subscribed() {
            return Ok(());
        }
        self.require_listener().await?;
        if self.subscriptions.cancellation().is_cancelled() {
            return Err("MCP 会话已关闭".to_owned());
        }
        self.subscriptions
            .set_transport_session_id(transport_session_id);
        self.shared
            .changes
            .begin_subscription(&self.subscriptions, peer);
        Ok(())
    }

    async fn search_history(&self, arguments: &Map<String, Value>, request: CancellationToken) -> Result<Value, String> {
        let cancellation = self.request_cancellation(request);
        if cancellation.token.is_cancelled() { return Err("请求已取消".into()); }
        let search_text = optional_string(arguments, "searchText", 1000).map_err(|_| "工具参数无效".to_owned())?;
        let limit = integer_argument(arguments, "limit", 1, 100, 20).map_err(|_| "工具参数无效".to_owned())? as u32;
        let offset = integer_argument(arguments, "offset", 0, 1_000_000, 0).map_err(|_| "工具参数无效".to_owned())? as u32;
        let records = self.shared.backend.search_history(McpHistorySearch { search_text, limit, offset }, &cancellation.token).await?;
        let mut items = Vec::with_capacity(records.len());
        for entry in records {
            let (content, _) = super::codec::clipboard(entry.content, false, None, &self.shared.backend.images, &cancellation.token).await?;
            items.push(super::codec::history(entry.metadata, content)?);
        }
        if cancellation.token.is_cancelled() { return Err("请求已取消".into()); }
        Ok(json!({ "items": items, "limit": limit, "offset": offset }))
    }

    async fn history_item(&self, arguments: &Map<String, Value>, include_image: bool, request: CancellationToken) -> Result<(Value, Vec<ContentBlock>), String> {
        let cancellation = self.request_cancellation(request);
        let item_id = required_string(arguments, "itemId", 1, 256).map_err(|_| "工具参数无效".to_owned())?;
        let entry = self.shared.backend.history_item(&item_id, &cancellation.token).await?;
        let (content, blocks) = super::codec::clipboard(entry.content, include_image, None, &self.shared.backend.images, &cancellation.token).await?;
        Ok((super::codec::history(entry.metadata, content)?, blocks))
    }

    async fn recognize_image(&self, mut arguments: Map<String, Value>, request: CancellationToken) -> Result<Value, String> {
        let image_base64 = optional_string_ref(&arguments, "imageBase64", MAX_IMAGE_BASE64_CHARS).map_err(|_| "工具参数无效".to_owned())?;
        let item_id = optional_string(&arguments, "itemId", 256).map_err(|_| "工具参数无效".to_owned())?;
        if image_base64.is_some_and(str::is_empty) { return Err("图片 Base64 无效".into()); }
        if item_id.as_deref().is_some_and(str::is_empty) { return Err("剪贴板历史记录不存在".into()); }
        if image_base64.is_some() && item_id.is_some() { return Err("imageBase64 和 itemId 不可同时提供".into()); }
        if self.shared.recognizing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() { return Err("MCP OCR 正在识别，请稍后重试".into()); }
        let _recognition = RecognitionGuard(&self.shared.recognizing);
        let cancellation = self.request_cancellation(request);
        if cancellation.token.is_cancelled() { return Err("请求已取消".into()); }
        self.require_clipboard().await?;
        let images = &self.shared.backend.images;
        let source = if image_base64.is_some() {
            let Some(Value::String(encoded)) = arguments.remove("imageBase64") else { return Err("工具参数无效".into()); };
            crate::images::ImageSource::Encoded(super::codec::import_image_base64(images, encoded, &cancellation.token).await?)
        } else if let Some(item_id) = item_id { self.shared.backend.history_image(&item_id, &cancellation.token).await? }
        else { self.shared.backend.current_clipboard(&cancellation.token).await?.image.ok_or("剪贴板内容不包含图片")? };
        let image = super::codec::validate_image(images, source, &cancellation.token).await?;
        let result = self.shared.backend.recognize_image(image, cancellation.token.clone()).await?;
        if cancellation.token.is_cancelled() { return Err("请求已取消".into()); }
        Ok(json!({ "text": result.text, "confidence": result.confidence, "mode": result.mode }))
    }
    fn request_cancellation(&self, request: CancellationToken) -> RequestCancellation {
        let token = request.child_token();
        let session = self.subscriptions.cancellation().clone();
        if session.is_cancelled() { token.cancel(); }
        let linked = token.clone();
        let relay = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            tokio::select! { _ = session.cancelled() => linked.cancel(), _ = linked.cancelled() => {} }
        }));
        RequestCancellation { _guard: token.clone().drop_guard(), token, _relay: relay }
    }

    async fn end_subscription(&self) {
        let _lifecycle = self.subscriptions.lifecycle.lock().await;
        self.subscriptions.end();
    }
}

impl ServerHandler for McpHandler {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::builder()
            .enable_resources()
            .enable_tools()
            .build();
        if let Some(resources) = capabilities.resources.as_mut() {
            resources.subscribe = Some(true);
        }
        ServerConfig::new(capabilities).with_server_info(Implementation::new(
            SERVER_NAME,
            self.shared.version.clone(),
        ))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.bind_transport(&context);
        Ok(ListToolsResult::with_all_items(Self::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.bind_transport(&context);
        let arguments = request.arguments.unwrap_or_default();
        let result = match request.name.as_ref() {
            "get_clipboard" => {
                async {
                    let include_image = bool_argument(&arguments, "includeImage", false)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    self.current_clipboard(include_image, context.ct.clone()).await
                }
                .await
            }
            "wait_for_clipboard_change" => {
                async {
                    let cursor = optional_string(&arguments, "cursor", 100)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    let timeout_ms = integer_argument(&arguments, "timeoutMs", 0, 30_000, 25_000)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    self.wait_for_change(cursor.as_deref(), timeout_ms, context.ct.clone())
                        .await
                        .map(|data| (data, Vec::new()))
                }
                .await
            }
            "subscribe_clipboard_changes" => self
                .subscribe(context.peer.clone(), transport_session_id(&context))
                .await
                .map(|()| {
                    (
                        json!({
                            "subscribed": true,
                            "uri": MCP_CLIPBOARD_RESOURCE_URI,
                            "notification": "notifications/resources/updated",
                            "eventMetaKey": MCP_CLIPBOARD_EVENT_META_KEY
                        }),
                        Vec::new(),
                    )
                }),
            "unsubscribe_clipboard_changes" => {
                self.end_subscription().await;
                Ok((
                    json!({ "subscribed": false, "uri": MCP_CLIPBOARD_RESOURCE_URI }),
                    Vec::new(),
                ))
            }
            "search_clipboard_history" => self
                .search_history(&arguments, context.ct.clone())
                .await
                .map(|data| (data, Vec::new())),
            "get_clipboard_history_item" => {
                async {
                    let include_image = bool_argument(&arguments, "includeImage", false)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    self.history_item(&arguments, include_image, context.ct.clone()).await
                }
                .await
            }
            "set_clipboard_text" => {
                async {
                    let cancellation = self.request_cancellation(context.ct.clone());
                    if cancellation.token.is_cancelled() { return Err("请求已取消".into()); }
                    let text = required_string(&arguments, "text", 1, 1_000_000)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    self.require_clipboard().await?;
                    self.shared.backend.set_clipboard_text(&text, &cancellation.token).await?;
                    Ok((json!({ "success": true }), Vec::new()))
                }
                .await
            }
            "ocr_image" => self
                .recognize_image(arguments, context.ct.clone())
                .await
                .map(|data| (data, Vec::new())),
            _ => {
                return Err(ErrorData::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    "Tool not found",
                    None,
                ));
            }
        };
        match result {
            Ok((data, blocks)) => {
                let serialized = serde_json::to_string(&data)
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                let mut result = CallToolResult::success(
                    std::iter::once(ContentBlock::text(serialized))
                        .chain(blocks)
                        .collect(),
                );
                result.structured_content = Some(data);
                Ok(result.into())
            }
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(error)]).into()),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        self.bind_transport(&context);
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new(MCP_CLIPBOARD_RESOURCE_URI, "clipboard")
                .with_title("读取剪贴板")
                .with_description("读取当前系统剪贴板的文本、HTML、RTF、文件路径和图片信息；includeImage=true 时返回 PNG 图片。返回 cursor 可用于等待后续变化。")
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        self.bind_transport(&context);
        if request.uri != MCP_CLIPBOARD_RESOURCE_URI {
            return Err(ErrorData::invalid_params("资源不存在", None));
        }
        let (data, _) = self
            .current_clipboard(false, context.ct.clone())
            .await
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let serialized = serde_json::to_string(&data)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![
                ResourceContents::text(serialized, MCP_CLIPBOARD_RESOURCE_URI)
                    .with_mime_type("application/json"),
            ],
        )))
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.bind_transport(&context);
        if request.uri != MCP_CLIPBOARD_RESOURCE_URI {
            return Err(ErrorData::invalid_params("资源不存在", None));
        }
        self.subscribe(context.peer.clone(), transport_session_id(&context))
            .await
            .map_err(|error| ErrorData::internal_error(error, None))
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.bind_transport(&context);
        if request.uri != MCP_CLIPBOARD_RESOURCE_URI {
            return Err(ErrorData::invalid_params("资源不存在", None));
        }
        self.end_subscription().await;
        Ok(())
    }
}

async fn notify_subscribers(shared: &McpServerContext, event: &McpChange) {
    let peers = shared.changes.subscribed_peers().into_iter().filter(|(_, session, _)| session.should_notify(&event.cursor)).collect::<Vec<_>>();
    if peers.is_empty() { return; }
    let wire = match super::codec::change(event) { Ok(wire) => wire, Err(error) => { tracing::error!(%error, "MCP 剪贴板通知序列化失败"); return; } };
    let notifications = peers.into_iter().map(|(session_id, session, peer)| {
        let wire = &wire;
        async move {
            let notification: ResourceUpdatedNotificationParam = resource_update(wire);
            let delivered = tokio::select! { biased;
                _ = session.cancellation().cancelled() => return,
                delivered = timeout(NOTIFICATION_TIMEOUT, peer.notify_resource_updated(notification)) => delivered,
            };
            let error = match delivered {
                Ok(Ok(())) => return, Ok(Err(error)) => error.to_string(), Err(_) => "剪贴板订阅通知发送超时".to_owned(),
            };
            session.end(); session.cancellation().cancel(); shared.changes.remove_session(&session_id);
            let close_result = match session.transport_session_id() { Some(id) => shared.close_session(id).await, None => Err("无法定位 MCP 会话 ID".to_owned()) };
            tracing::warn!(subscription_id = %session_id, %error, close_error = ?close_result.err(), "MCP 剪贴板通知失败，已关闭会话");
        }
    });
    join_all(notifications).await;
}

fn transport_session_id(context: &RequestContext<RoleServer>) -> Option<String> {
    context
        .extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.headers.get("mcp-session-id"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn tool(
    name: &'static str,
    properties: Value,
    required: &[&str],
    annotations: ToolAnnotations,
) -> Tool {
    let info = super_paste_format::MCP_TOOLS.iter().find(|tool| tool.name == name).expect("tool must exist in the MCP catalog");
    let mut input_schema = json!({
        "type":"object",
        "properties":properties
    });
    if !required.is_empty() {
        input_schema["required"] = json!(required);
    }
    Tool::new(
        name,
        info.description,
        input_schema.as_object().cloned().unwrap_or_default(),
    )
    .with_title(info.title)
    .annotate(annotations)
}

fn wait_is_changed(receiver: &watch::Receiver<CursorState>, initial: &str) -> bool {
    receiver.borrow().cursor != initial
}

fn optional_string_ref<'a>(arguments: &'a Map<String, Value>, name: &str, max: usize) -> Result<Option<&'a str>, ErrorData> {
    match arguments.get(name) {
        None => Ok(None),
        Some(Value::String(value)) if value.len() <= max || value.chars().take(max.saturating_add(1)).count() <= max => Ok(Some(value)),
        _ => Err(invalid_arguments()),
    }
}

fn bool_argument(
    arguments: &Map<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, ErrorData> {
    match arguments.get(name) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(invalid_arguments()),
    }
}

fn integer_argument(
    arguments: &Map<String, Value>,
    name: &str,
    minimum: u64,
    maximum: u64,
    default: u64,
) -> Result<u64, ErrorData> {
    let Some(value) = arguments.get(name) else {
        return Ok(default);
    };
    let parsed = value
        .as_u64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|number| number.is_finite() && number.fract() == 0.0 && *number >= 0.0)
                .map(|number| number as u64)
        })
        .ok_or_else(invalid_arguments)?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err(invalid_arguments());
    }
    Ok(parsed)
}

fn optional_string(
    arguments: &Map<String, Value>,
    name: &str,
    maximum: usize,
) -> Result<Option<String>, ErrorData> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(invalid_arguments());
    };
    if value.encode_utf16().count() > maximum {
        return Err(invalid_arguments());
    }
    Ok(Some(value.to_owned()))
}

fn required_string<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
    minimum: usize,
    maximum: usize,
) -> Result<&'a str, ErrorData> {
    let value = arguments.get(name).and_then(Value::as_str).ok_or_else(invalid_arguments)?;
    let length = value.encode_utf16().count();
    if length < minimum || length > maximum {
        return Err(invalid_arguments());
    }
    Ok(value)
}

fn invalid_arguments() -> ErrorData {
    ErrorData::invalid_params("工具参数无效", None)
}

struct RequestCancellation {
    token: CancellationToken,
    _guard: tokio_util::sync::DropGuard,
    _relay: tokio_util::task::AbortOnDropHandle<()>,
}

struct RecognitionGuard<'a>(&'a AtomicBool);

impl Drop for RecognitionGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
