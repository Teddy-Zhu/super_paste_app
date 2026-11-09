use std::{
    borrow::Cow,
    future::Future,
    io::Cursor,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::future::join_all;
use image::{ImageFormat, ImageReader};
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
        MCP_CLIPBOARD_EVENT_META_KEY, MCP_CLIPBOARD_RESOURCE_URI, MCP_MAX_IMAGE_BYTES, McpBackend,
        McpClipboardData, McpHistoryEntry, McpHistorySearch,
    },
};

const SERVER_NAME: &str = "super-paste";
const IMAGE_PIXEL_LIMIT: u64 = 40_000_000;
const MAX_IMAGE_BASE64_CHARS: usize = (MCP_MAX_IMAGE_BYTES + 2) / 3 * 4 + 100;
const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) type SessionCloserFuture =
    Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;
pub(crate) type SessionCloser = Arc<dyn Fn(String) -> SessionCloserFuture + Send + Sync>;

pub(crate) struct McpServerContext {
    pub backend: Arc<dyn McpBackend>,
    pub version: String,
    pub changes: Arc<ChangeBroker>,
    pub recognizing: AtomicBool,
    session_closer: Mutex<Option<SessionCloser>>,
}

impl McpServerContext {
    pub fn new(backend: Arc<dyn McpBackend>, version: String) -> Arc<Self> {
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
                "读取剪贴板",
                "读取当前系统剪贴板的文本、HTML、RTF、文件路径和图片信息；includeImage=true 时返回 PNG 图片。返回 cursor 可用于等待后续变化。",
                json!({"includeImage":{"type":"boolean","default":false}}),
                &[],
                read_only("读取剪贴板"),
            ),
            tool(
                "wait_for_clipboard_change",
                "等待剪贴板变化",
                "等待软件监听到剪贴板变化，最多等待 30 秒。首次可省略 cursor，之后传入上次返回的 cursor。返回 changed、cursor、timestamp；变化后调用 get_clipboard 读取当前内容，不保留每次变化的内容快照。",
                json!({
                    "cursor":{"type":"string","maxLength":100},
                    "timeoutMs":{"type":"integer","minimum":0,"maximum":30000,"default":25000}
                }),
                &[],
                read_only("等待剪贴板变化"),
            ),
            tool(
                "subscribe_clipboard_changes",
                "订阅剪贴板变化",
                "在当前 MCP 会话订阅剪贴板变化，通过 Streamable HTTP GET /mcp 的 SSE 连接持续推送 notifications/resources/updated。params._meta[\"super-paste/clipboard\"] 包含 cursor、timestamp 和 content（文本、HTML、RTF、文件路径、图片字节数）。图片数据用 get_clipboard 获取。客户端须保持 SSE 连接；重连后重新读取当前内容，通知不提供离线重放。",
                json!({}),
                &[],
                read_only("订阅剪贴板变化"),
            ),
            tool(
                "unsubscribe_clipboard_changes",
                "取消剪贴板订阅",
                "取消当前 MCP 会话的剪贴板变化推送，不影响其他客户端的订阅。",
                json!({}),
                &[],
                read_only("取消剪贴板订阅"),
            ),
            tool(
                "search_clipboard_history",
                "搜索剪贴板历史",
                "按内容或 OCR 文字搜索本地剪贴板历史，按复制时间倒序分页返回内容和记录 ID，图片仅返回元数据。",
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
                "读取历史记录",
                "按记录 ID 获取一条剪贴板历史，包括文本、OCR 结果、格式和来源；includeImage=true 时返回 PNG 图片。",
                json!({
                    "itemId":{"type":"string","minLength":1,"maxLength":256},
                    "includeImage":{"type":"boolean","default":false}
                }),
                &["itemId"],
                read_only("读取历史记录"),
            ),
            tool(
                "set_clipboard_text",
                "写入剪贴板文本",
                "将指定纯文本写入系统剪贴板，覆盖当前内容；不会模拟粘贴或键盘输入。",
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
                "识别图片文字",
                "使用软件当前 OCR 设置识别图片并直接返回文字。提供 imageBase64（原始 Base64 或图片 data URL）或历史 itemId，均不提供则识别当前剪贴板图片。两者不可同时提供。图片最大 10 MiB，不读取任意文件路径或 URL。本地模式须先在 OCR 设置中下载模型；远程或 AI 模式会使用已配置服务并可能产生费用。",
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

    async fn current_clipboard(
        &self,
        include_image: bool,
    ) -> Result<(Value, Vec<ContentBlock>), String> {
        self.require_clipboard().await?;
        let cursor = self.shared.changes.cursor();
        let clipboard = self.shared.backend.current_clipboard().await?;
        serialize_clipboard(clipboard, include_image, Some(cursor))
    }

    async fn wait_for_change(
        &self,
        requested_cursor: Option<&str>,
        timeout_ms: u64,
        request_cancel: CancellationToken,
    ) -> Result<Value, String> {
        if request_cancel.is_cancelled() || self.subscriptions.cancellation.is_cancelled() {
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
                    _ = self.subscriptions.cancellation.cancelled() => return Err("请求已取消".to_owned()),
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
        if self.subscriptions.cancellation.is_cancelled() {
            return Err("MCP 会话已关闭".to_owned());
        }
        if self.subscriptions.is_subscribed() {
            return Ok(());
        }
        self.require_listener().await?;
        if self.subscriptions.cancellation.is_cancelled() {
            return Err("MCP 会话已关闭".to_owned());
        }
        self.subscriptions
            .set_transport_session_id(transport_session_id);
        self.shared
            .changes
            .begin_subscription(&self.subscriptions, peer);
        Ok(())
    }

    async fn search_history(&self, arguments: &Map<String, Value>) -> Result<Value, String> {
        let search_text = optional_string(arguments, "searchText", 1000)
            .map_err(|_| "工具参数无效".to_owned())?;
        let limit = integer_argument(arguments, "limit", 1, 100, 20)
            .map_err(|_| "工具参数无效".to_owned())? as u32;
        let offset = integer_argument(arguments, "offset", 0, 1_000_000, 0)
            .map_err(|_| "工具参数无效".to_owned())? as u32;
        let records = self
            .shared
            .backend
            .search_history(McpHistorySearch {
                search_text,
                limit,
                offset,
            })
            .await?;
        let items = records
            .into_iter()
            .map(|entry| serialize_history_search_item(entry))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "items": items, "limit": limit, "offset": offset }))
    }

    async fn history_item(
        &self,
        arguments: &Map<String, Value>,
        include_image: bool,
    ) -> Result<(Value, Vec<ContentBlock>), String> {
        let item_id =
            required_string(arguments, "itemId", 1, 256).map_err(|_| "工具参数无效".to_owned())?;
        let entry = self
            .shared
            .backend
            .history_item(&item_id)
            .await
            .map_err(map_history_error)?;
        let (content, blocks) = serialize_clipboard(entry.content, include_image, None)?;
        let mut fields = object_or_error(entry.fields)?;
        fields.insert("content".to_owned(), content);
        Ok((Value::Object(fields), blocks))
    }

    async fn recognize_image(
        &self,
        arguments: &Map<String, Value>,
        request_cancel: CancellationToken,
    ) -> Result<Value, String> {
        let image_base64 = optional_string(arguments, "imageBase64", MAX_IMAGE_BASE64_CHARS)
            .map_err(|_| "工具参数无效".to_owned())?;
        let item_id =
            optional_string(arguments, "itemId", 256).map_err(|_| "工具参数无效".to_owned())?;
        if image_base64.as_deref().is_some_and(str::is_empty) {
            return Err("图片 Base64 无效".to_owned());
        }
        if item_id.as_deref().is_some_and(str::is_empty) {
            return Err("剪贴板历史记录不存在".to_owned());
        }
        if image_base64.is_some() && item_id.is_some() {
            return Err("imageBase64 和 itemId 不可同时提供".to_owned());
        }
        if self
            .shared
            .recognizing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("MCP OCR 正在识别，请稍后重试".to_owned());
        }
        let _recognition = RecognitionGuard(&self.shared.recognizing);

        if self.subscriptions.cancellation.is_cancelled() || request_cancel.is_cancelled() {
            return Err("MCP 服务已停止".to_owned());
        }
        self.require_clipboard().await?;
        let image = if let Some(encoded) = image_base64 {
            decode_image_base64(&encoded)?
        } else {
            let entry = if let Some(item_id) = item_id {
                self.shared
                    .backend
                    .history_item(&item_id)
                    .await
                    .map_err(map_history_error)?
            } else {
                McpHistoryEntry {
                    fields: Value::Object(Map::new()),
                    content: self.shared.backend.current_clipboard().await?,
                }
            };
            entry
                .content
                .image
                .filter(|bytes| !bytes.is_empty())
                .ok_or_else(|| "剪贴板内容不包含图片".to_owned())?
        };
        validate_image(&image)?;

        let cancellation = CancellationToken::new();
        let cancellation_for_task = cancellation.clone();
        let request_cancel_for_task = request_cancel.clone();
        let session_cancel_for_task = self.subscriptions.cancellation.clone();
        let cancellation_task = tokio::spawn(async move {
            tokio::select! {
                _ = request_cancel_for_task.cancelled() => cancellation_for_task.cancel(),
                _ = session_cancel_for_task.cancelled() => cancellation_for_task.cancel(),
            }
        });
        let result = self
            .shared
            .backend
            .recognize_image(image, cancellation.clone())
            .await;
        cancellation_task.abort();
        if cancellation.is_cancelled() {
            return Err("请求已取消".to_owned());
        }
        result.map(|mut result| {
            if let Value::Object(fields) = &mut result {
                fields.remove("details");
            }
            result
        })
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
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = request.arguments.unwrap_or_default();
        let result = match request.name.as_ref() {
            "get_clipboard" => {
                async {
                    let include_image = bool_argument(&arguments, "includeImage", false)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    self.current_clipboard(include_image).await
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
                .search_history(&arguments)
                .await
                .map(|data| (data, Vec::new())),
            "get_clipboard_history_item" => {
                async {
                    let include_image = bool_argument(&arguments, "includeImage", false)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    self.history_item(&arguments, include_image).await
                }
                .await
            }
            "set_clipboard_text" => {
                async {
                    let text = required_string(&arguments, "text", 1, 1_000_000)
                        .map_err(|_| "工具参数无效".to_owned())?;
                    self.require_clipboard().await?;
                    self.shared.backend.set_clipboard_text(&text).await?;
                    Ok((json!({ "success": true }), Vec::new()))
                }
                .await
            }
            "ocr_image" => self
                .recognize_image(&arguments, context.ct.clone())
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
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
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
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != MCP_CLIPBOARD_RESOURCE_URI {
            return Err(ErrorData::invalid_params("资源不存在", None));
        }
        let (data, _) = self
            .current_clipboard(false)
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
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if request.uri != MCP_CLIPBOARD_RESOURCE_URI {
            return Err(ErrorData::invalid_params("资源不存在", None));
        }
        self.end_subscription().await;
        Ok(())
    }
}

async fn notify_subscribers(shared: &McpServerContext, event: &Value) {
    let Some(cursor) = event.get("cursor").and_then(Value::as_str) else {
        return;
    };
    let notifications = shared
        .changes
        .subscribed_peers()
        .into_iter()
        .filter(|(_, session, _)| session.should_notify(cursor))
        .map(|(session_id, session, peer)| async move {
            let notification: ResourceUpdatedNotificationParam = resource_update(event);
            let error = match timeout(
                NOTIFICATION_TIMEOUT,
                peer.notify_resource_updated(notification),
            )
            .await
            {
                Ok(Ok(())) => return,
                Ok(Err(error)) => error.to_string(),
                Err(_) => "剪贴板订阅通知发送超时".to_owned(),
            };
            session.end();
            session.cancellation.cancel();
            shared.changes.remove_session(&session_id);
            let close_result = match session.transport_session_id() {
                Some(transport_session_id) => shared.close_session(transport_session_id).await,
                None => Err("无法定位 MCP 会话 ID".to_owned()),
            };
            tracing::warn!(
                subscription_id = %session_id,
                error = %error,
                close_error = close_result
                    .as_ref()
                    .err()
                    .map(String::as_str)
                    .unwrap_or(""),
                "closing MCP session after clipboard notification failure"
            );
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
    title: &'static str,
    description: &'static str,
    properties: Value,
    required: &[&str],
    annotations: ToolAnnotations,
) -> Tool {
    let mut input_schema = json!({
        "type":"object",
        "properties":properties
    });
    if !required.is_empty() {
        input_schema["required"] = json!(required);
    }
    Tool::new(
        name,
        description,
        input_schema.as_object().cloned().unwrap_or_default(),
    )
    .with_title(title)
    .annotate(annotations)
}

fn wait_is_changed(receiver: &watch::Receiver<CursorState>, initial: &str) -> bool {
    receiver.borrow().cursor != initial
}

fn serialize_clipboard(
    clipboard: McpClipboardData,
    include_image: bool,
    cursor: Option<String>,
) -> Result<(Value, Vec<ContentBlock>), String> {
    let mut data = object_or_error(clipboard.data)?;
    let mut blocks = Vec::new();
    if let Some(original) = clipboard.image.filter(|bytes| !bytes.is_empty()) {
        let mut image_info = Map::new();
        if include_image {
            let image = validate_image(&original)?;
            let mut png = Cursor::new(Vec::new());
            image
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|_| "图片格式无效或不受支持".to_owned())?;
            let png = png.into_inner();
            if png.len() > MCP_MAX_IMAGE_BYTES {
                return Err("PNG 图片超过 10 MiB，请关闭 includeImage".to_owned());
            }
            image_info.insert("bytes".to_owned(), Value::from(png.len()));
            image_info.insert("mimeType".to_owned(), Value::String("image/png".to_owned()));
            image_info.insert("width".to_owned(), Value::from(image.width()));
            image_info.insert("height".to_owned(), Value::from(image.height()));
            blocks.push(ContentBlock::image(STANDARD.encode(png), "image/png"));
        } else {
            image_info.insert("bytes".to_owned(), Value::from(original.len()));
        }
        data.insert("image".to_owned(), Value::Object(image_info));
    }
    if let Some(cursor) = cursor {
        data.insert("cursor".to_owned(), Value::String(cursor));
    }
    Ok((Value::Object(data), blocks))
}

fn serialize_history_search_item(entry: McpHistoryEntry) -> Result<Value, String> {
    let (content, _) = serialize_clipboard(entry.content, false, None)?;
    let mut fields = object_or_error(entry.fields)?;
    fields.remove("content");
    fields.insert("content".to_owned(), content);
    Ok(Value::Object(fields))
}

fn object_or_error(value: Value) -> Result<Map<String, Value>, String> {
    match value {
        Value::Object(object) => Ok(object),
        _ => Err("剪贴板数据格式无效".to_owned()),
    }
}

fn map_history_error(error: String) -> String {
    if error == "历史记录不存在" {
        "剪贴板历史记录不存在".to_owned()
    } else {
        error
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

fn required_string(
    arguments: &Map<String, Value>,
    name: &str,
    minimum: usize,
    maximum: usize,
) -> Result<String, ErrorData> {
    let value = optional_string(arguments, name, maximum)?.ok_or_else(invalid_arguments)?;
    let length = value.encode_utf16().count();
    if length < minimum {
        return Err(invalid_arguments());
    }
    Ok(value)
}

fn invalid_arguments() -> ErrorData {
    ErrorData::invalid_params("工具参数无效", None)
}

fn decode_image_base64(input: &str) -> Result<Vec<u8>, String> {
    let input = if let Some(data_uri) = input.strip_prefix("data:image/") {
        let Some((mime, data)) = data_uri.split_once(";base64,") else {
            return Err("图片 Base64 无效".to_owned());
        };
        if mime.is_empty()
            || !mime
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
        {
            return Err("图片 Base64 无效".to_owned());
        }
        data
    } else {
        input
    };
    let encoded: String = input
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if encoded.is_empty()
        || !encoded.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' || byte == b'='
        })
    {
        return Err("图片 Base64 无效".to_owned());
    }
    let unpadded = encoded.trim_end_matches('=');
    if unpadded.is_empty() || encoded.len() - unpadded.len() > 2 {
        return Err("图片 Base64 无效".to_owned());
    }
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(unpadded)
        .map_err(|_| "图片 Base64 无效".to_owned())?;
    if base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bytes) != unpadded {
        return Err("图片 Base64 无效".to_owned());
    }
    validate_image(&bytes)?;
    Ok(bytes)
}

fn validate_image(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    if bytes.is_empty() || bytes.len() > MCP_MAX_IMAGE_BYTES {
        return Err("图片须为 1 字节至 10 MiB".to_owned());
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "图片格式无效或不受支持".to_owned())?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        return Err("图片格式无效或不受支持".to_owned());
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "图片格式无效或不受支持".to_owned())?;
    if u64::from(width) * u64::from(height) > IMAGE_PIXEL_LIMIT {
        return Err("图片超过 4000 万像素，请缩小后重试".to_owned());
    }
    image::load_from_memory(bytes).map_err(|_| "图片格式无效或不受支持".to_owned())
}

struct RecognitionGuard<'a>(&'a AtomicBool);

impl Drop for RecognitionGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
