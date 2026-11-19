use super::*;
use crate::{
    clipboard::{ClipboardBackend, ClipboardService, ClipboardSnapshot},
    configuration::{ConfigurationEffects, ConfigurationMutation, ConfigurationService, PreparedConfigurationEffects, RuntimeConfiguration},
    credentials::{CredentialBackend, CredentialReference, CredentialVault, Secret},
    events::EventBus,
    images::{ImageBudgets, ImageService, ImageSource},
    mcp::backend::Backend,
    ocr::{ModelManager, OcrService},
    storage::{PreparedRecord, Storage},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::ImageEncoder;
use serde_json::Value;
use sha2::Digest as _;
use std::{collections::BTreeSet, path::PathBuf, sync::atomic::{AtomicU64, AtomicUsize, Ordering}};
use super_paste_format::{ConfigChange, ConfigField, Digest, HistoryLimits, OcrMode, TextContent};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).expect("remove isolated MCP directory"); }
}

struct NoCredentials;
#[async_trait::async_trait]
impl CredentialBackend for NoCredentials {
    async fn read(&self, _: CredentialReference) -> crate::Result<Option<Arc<Secret>>> { Err(crate::Error::CredentialUnavailable) }
    async fn write(&self, _: CredentialReference, _: Arc<Secret>) -> crate::Result<()> { Err(crate::Error::CredentialUnavailable) }
    async fn delete(&self, _: CredentialReference) -> crate::Result<()> { Err(crate::Error::CredentialUnavailable) }
}
struct StartupEffects;
impl ConfigurationEffects for StartupEffects {
    async fn prepare(&self, _: RuntimeConfiguration, _: RuntimeConfiguration, _: BTreeSet<ConfigField>) -> crate::Result<impl PreparedConfigurationEffects + 'static> { Ok(Self) }
}
impl PreparedConfigurationEffects for StartupEffects {
    async fn apply(&mut self) -> crate::Result<()> { Ok(()) }
    async fn rollback(&mut self) -> crate::Result<()> { Ok(()) }
}

// Only the operating system boundary is replaced. Requests still traverse the
// application HTTP middleware, SDK sessions, real handlers and core services.
struct Clipboard { value: Mutex<ClipboardSnapshot>, writes: AtomicUsize, read_gate: Mutex<Option<Arc<ReadGate>>> }
struct ReadGate { entered: tokio::sync::Semaphore, cancelled: tokio::sync::Semaphore, release: tokio::sync::Semaphore }
impl ReadGate {
    fn new() -> Arc<Self> {
        Arc::new(Self { entered: tokio::sync::Semaphore::new(0), cancelled: tokio::sync::Semaphore::new(0), release: tokio::sync::Semaphore::new(0) })
    }
}
#[async_trait::async_trait]
impl ClipboardBackend for Clipboard {
    async fn read(&self, cancel: &CancellationToken) -> crate::Result<ClipboardSnapshot> {
        if cancel.is_cancelled() { return Err(crate::Error::Cancelled); }
        let gate = self.read_gate.lock().expect("read gate").clone();
        if let Some(gate) = gate {
            gate.entered.add_permits(1);
            cancel.cancelled().await;
            gate.cancelled.add_permits(1);
            // Model a native call that cannot return until the OS completes,
            // even after cancellation or the original caller disappearing.
            gate.release.acquire().await.expect("release native read").forget();
        }
        Ok(self.value.lock().expect("isolated clipboard").clone())
    }
    async fn write(&self, value: ClipboardSnapshot, cancel: &CancellationToken) -> crate::Result<()> {
        if cancel.is_cancelled() { return Err(crate::Error::Cancelled); }
        *self.value.lock().expect("isolated clipboard") = value;
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

const VERSION: &str = "2025-03-26";
const KEY: &str = "isolated-interop-key";
struct Wire { client: reqwest::Client, url: String, session: String, next: AtomicU64 }
impl Wire {
    fn request(&self, method: Method) -> reqwest::RequestBuilder {
        self.client.request(method, &self.url).bearer_auth(KEY)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", VERSION).header(SESSION_HEADER, &self.session)
    }
    async fn rpc(&self, method: &str, params: Value) -> Value {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let response = self.request(Method::POST).json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .send().await.expect("MCP request");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.bytes().await.expect("complete RPC body");
        let reply = rpc_payload(&body).expect("RPC reply");
        assert_eq!(reply["id"], id);
        reply
    }
    async fn tool(&self, name: &str, arguments: Value) -> Value {
        let reply = self.rpc("tools/call", json!({ "name": name, "arguments": arguments })).await;
        assert!(reply.get("error").is_none(), "{reply}");
        let result = reply["result"].clone();
        if result["isError"] != true {
            let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().expect("JSON text block")).expect("tool JSON");
            assert_eq!(text, result["structuredContent"]);
        }
        result
    }
}

fn rpc_payload(body: &[u8]) -> Option<Value> {
    assert!(body.len() <= 64 * 1024, "bounded fixture reply");
    let text = match std::str::from_utf8(body) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => return None,
        Err(error) => panic!("invalid UTF-8 protocol response: {error}"),
    };
    if text.starts_with('{') { return Some(serde_json::from_str(text).expect("JSON reply")); }
    let text = text.replace("\r\n", "\n");
    let (complete, _) = text.rsplit_once("\n\n")?;
    complete.lines().filter_map(|line| line.strip_prefix("data:"))
        .filter(|payload| !payload.trim().is_empty())
        .map(|payload| serde_json::from_str(payload.trim()).expect("JSON SSE event")).next()
}

#[tokio::test]
async fn real_tools_resources_notifications_and_authentication_interoperate() {
    timeout(Duration::from_secs(15), async {
        let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-mcp-interop-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&directory.0).expect("isolated directory");
        let storage = Storage::open(directory.0.join("history.sqlite3")).await.expect("database");
        let events = EventBus::new(32).expect("events");
        let runtime = tokio::runtime::Handle::current();
        let configuration = ConfigurationService::open(storage.clone(), CredentialVault::new(storage.clone(), Arc::new(NoCredentials)), events.clone(), runtime.clone()).await.expect("configuration");
        let listener = HttpRuntime::bind_listener(0).await.expect("loopback listener");
        let address = listener.local_addr().expect("listener address");
        let initial = configuration.snapshot();
        configuration.commit(ConfigurationMutation {
            client_id: uuid::Uuid::new_v4(), mutation_id: uuid::Uuid::new_v4(),
            base_versions: [ConfigField::OcrMode, ConfigField::OcrRemoteUrl].into_iter().map(|field| (field, initial.fields.get(&field).copied().unwrap_or(0))).collect(),
            changes: vec![ConfigChange::OcrMode(OcrMode::Remote), ConfigChange::OcrRemoteUrl(format!("http://{address}/recognize"))], credentials: vec![],
        }, Arc::new(StartupEffects)).await.expect("startup OCR configuration");
        let images = ImageService::new(runtime.clone(), storage.clone(), ImageBudgets::default()).expect("images");
        let cancel = CancellationToken::new();
        let _shutdown = cancel.clone().drop_guard();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[64, 128, 192, 255], 1, 1, image::ExtendedColorType::Rgba8).expect("PNG fixture");
        let original = images.import_encoded(png.clone(), &cancel).await.expect("original image");
        let content = TextContent { text: Some("原始\0文本 😀".into()), html: Some("<b>原始文本</b>".into()),
            rtf: Some(r"{\rtf1 original}".into()), files: vec!["/isolated/路径.txt".into()].into() };
        let (record, _) = storage.upsert_record(PreparedRecord { digest: Digest([7; 32]), content: content.clone(), image: Some(original.clone()),
            timestamp: crate::now_ms().expect("clock"), source: None }, HistoryLimits { max_records: 20, keep_days: -1 }).await.expect("history fixture");
        let models = ModelManager::new(directory.0.join("models"), events.clone(), runtime.clone()).expect("model manager");
        let ocr = OcrService::new(storage.clone(), configuration.clone(), images.clone(), models, Err("local OCR is not used in this protocol regression".into()), events.clone(), runtime.clone()).await.expect("OCR service");
        let native = Arc::new(Clipboard { value: Mutex::new(ClipboardSnapshot { content: content.clone(), image: Some(ImageSource::Encoded(original.clone())) }), writes: AtomicUsize::new(0), read_gate: Mutex::new(None) });
        let clipboard = ClipboardService::new(runtime, storage.clone(), configuration.clone(), images.clone(), native.clone(), events, ocr.clone());
        clipboard.set_native_listening(true);
        let context = McpServerContext::new(Arc::new(Backend { clipboard: clipboard.clone(), storage: storage.clone(), images: images.clone(), ocr: ocr.clone(), shutdown: cancel.clone() }), "interop".into());
        let settings = RuntimeSettings { value: super_paste_format::McpSettings { enabled: true, port: address.port(), auth_enabled: true },
            reference: None, authorization_digest: Some(Sha256::digest(format!("Bearer {KEY}").as_bytes()).into()) };
        let mut http = HttpRuntime::with_listener(listener, settings, context.clone(), cancel.clone());
        let state = http.state.clone();
        let expected_image = STANDARD.encode(&png);
        let remote_calls = Arc::new(AtomicUsize::new(0));
        let calls = remote_calls.clone();
        http.router = http.router.route("/recognize", axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let expected_image = expected_image.clone(); let calls = calls.clone();
            async move {
                assert_eq!(body["image"], expected_image, "remote OCR receives the original encoding");
                calls.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({ "data": { "result": "识别结果 😀" } }))
            }
        }));
        let server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(http.serve()));
        let notifications = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(context.clone().notification_worker(cancel.clone())));
        let client = reqwest::Client::builder().timeout(Duration::from_secs(3)).build().expect("HTTP client");
        let url = format!("http://{address}{MCP_PATH}");
        let initialize = json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": VERSION, "capabilities": {}, "clientInfo": { "name": "real-handler-regression", "version": "1" }
        } });
        let unauthorized = client.post(&url).json(&initialize).send().await.expect("missing auth request");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert!(unauthorized.headers().contains_key(WWW_AUTHENTICATE));
        let rejected = client.post(&url).bearer_auth("wrong").json(&initialize).send().await.expect("invalid auth request");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        let response = client.post(&url).bearer_auth(KEY).header("accept", "application/json, text/event-stream").json(&initialize).send().await.expect("initialize");
        assert_eq!(response.status(), StatusCode::OK);
        let session = response.headers().get(SESSION_HEADER).expect("session header").to_str().expect("session ID").to_owned();
        let init = rpc_payload(&response.bytes().await.expect("initialize body")).expect("initialize reply");
        assert_eq!(init["result"]["capabilities"]["resources"]["subscribe"], true);
        let wire = Arc::new(Wire { client, url, session, next: AtomicU64::new(1) });
        assert_eq!(wire.request(Method::POST).json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).send().await.expect("initialized").status(), StatusCode::ACCEPTED);

        let tools = wire.rpc("tools/list", json!({})).await;
        let names = tools["result"]["tools"].as_array().expect("tool catalog").iter().map(|tool| tool["name"].as_str().expect("tool name")).collect::<BTreeSet<_>>();
        assert_eq!(names, super_paste_format::MCP_TOOLS.iter().map(|tool| tool.name).collect());
        assert_eq!(names.len(), 8);
        let resources = wire.rpc("resources/list", json!({})).await;
        assert_eq!(resources["result"]["resources"][0]["uri"], super::super::types::MCP_CLIPBOARD_RESOURCE_URI);
        let current = wire.tool("get_clipboard", json!({})).await;
        let data = &current["structuredContent"];
        assert_eq!(data["text"], content.text.as_deref().unwrap());
        assert_eq!(data["html"], content.html.as_deref().unwrap());
        assert_eq!(data["rtf"], content.rtf.as_deref().unwrap());
        assert_eq!(data["files"][0], content.files[0]);
        assert_eq!(data["image"]["bytes"], png.len());
        assert!(data["image"].get("mimeType").is_none());
        let image = wire.tool("get_clipboard", json!({ "includeImage": true })).await;
        assert_eq!(image["content"][1]["mimeType"], "image/png");
        let encoded = STANDARD.decode(image["content"][1]["data"].as_str().expect("PNG block")).expect("PNG Base64");
        assert_eq!(image::load_from_memory(&encoded).expect("PNG pixels").to_rgba8().into_raw(), [64, 128, 192, 255]);
        let uri = super::super::types::MCP_CLIPBOARD_RESOURCE_URI;
        let resource = wire.rpc("resources/read", json!({ "uri": uri })).await;
        let resource_data: Value = serde_json::from_str(resource["result"]["contents"][0]["text"].as_str().expect("resource JSON")).expect("resource fields");
        assert_eq!(resource_data, *data);
        let invalid_resource = wire.rpc("resources/read", json!({ "uri": "clipboard://missing" })).await;
        assert_eq!(invalid_resource["error"]["code"], -32602);
        let history = wire.tool("search_clipboard_history", json!({ "searchText": "原始", "limit": 1, "offset": 0 })).await;
        assert_eq!(history["structuredContent"]["items"][0]["id"], record.id.to_string());
        assert_eq!(history["structuredContent"]["items"][0]["content"]["text"], data["text"]);
        assert_eq!(wire.tool("search_clipboard_history", json!({ "searchText": "原始", "limit": 1, "offset": 1 })).await["structuredContent"]["items"], json!([]));
        assert_eq!(wire.tool("search_clipboard_history", json!({ "limit": 0 })).await["isError"], true);
        let item = wire.tool("get_clipboard_history_item", json!({ "itemId": record.id, "includeImage": true })).await;
        assert_eq!(item["structuredContent"]["content"]["text"], data["text"]);
        assert_eq!(item["content"][1]["mimeType"], "image/png");
        assert_eq!(wire.tool("get_clipboard_history_item", json!({ "itemId": uuid::Uuid::new_v4() })).await["isError"], true);
        ocr.set_suspended(true);
        let queued = ocr.enqueue_record(record.id, crate::ocr::OcrTrigger::Manual).await.expect("queue while suspended").expect("manual ticket");
        let paused = ocr.diagnostics().await;
        assert!(paused.suspended);
        assert_eq!((paused.queued, paused.running), (1, 0));
        assert!(timeout(Duration::from_millis(30), queued.clone().wait(&cancel)).await.is_err(), "suspension retains queued recognition without dispatching it");
        assert_eq!(remote_calls.load(Ordering::SeqCst), 0);
        // Explicit image recognition remains available independently of the
        // history queue, matching the original scheduler's pause contract.
        let recognized = wire.tool("ocr_image", json!({ "itemId": record.id })).await;
        assert_eq!(recognized["structuredContent"]["text"], "识别结果 😀");
        assert_eq!(recognized["structuredContent"]["mode"], "remote");
        assert_eq!(remote_calls.load(Ordering::SeqCst), 1);
        assert_eq!(wire.tool("ocr_image", json!({ "imageBase64": "AQ", "itemId": record.id })).await["isError"], true);
        assert_eq!(wire.tool("ocr_image", json!({ "imageBase64": "%%%" })).await["isError"], true);
        assert_eq!(remote_calls.load(Ordering::SeqCst), 1, "invalid OCR requests do not contact the provider");
        ocr.set_suspended(false);
        assert_eq!(queued.wait(&cancel).await.expect("resume queued recognition").text.as_ref(), "识别结果 😀");
        assert!(!ocr.diagnostics().await.suspended);
        assert_eq!(remote_calls.load(Ordering::SeqCst), 2, "resuming runs the retained record exactly once");
        assert_eq!(wire.tool("set_clipboard_text", json!({ "text": "写入\0文字 😀" })).await["structuredContent"]["success"], true);
        assert_eq!(native.writes.load(Ordering::SeqCst), 1);
        let written = native.value.lock().expect("written clipboard").clone();
        assert_eq!(written.content.text.as_deref(), Some("写入\0文字 😀"));
        assert!(written.content.html.is_none() && written.content.rtf.is_none() && written.content.files.is_empty() && written.image.is_none());
        assert_eq!(wire.rpc("tools/call", json!({ "name": "missing" })).await["error"]["code"], -32601);

        let cursor = data["cursor"].as_str().expect("change cursor");
        assert_eq!(wire.tool("wait_for_clipboard_change", json!({ "cursor": cursor, "timeoutMs": 0 })).await["structuredContent"]["changed"], false);
        assert_eq!(wire.tool("wait_for_clipboard_change", json!({ "cursor": "foreign:0", "timeoutMs": 0 })).await["isError"], true);
        assert!(wire.rpc("resources/subscribe", json!({ "uri": uri })).await.get("error").is_none());
        assert_eq!(wire.tool("subscribe_clipboard_changes", json!({})).await["structuredContent"]["subscribed"], true);
        assert_eq!(context.changes.subscribed_peers().len(), 1, "tool and resource subscriptions share one session registration");
        let stream = wire.request(Method::GET).send().await.expect("notification stream");
        assert_eq!(stream.status(), StatusCode::OK);
        let mut stream = stream.bytes_stream();
        let waiting = wire.request(Method::POST).json(&json!({ "jsonrpc": "2.0", "id": "await-update", "method": "tools/call",
            "params": { "name": "wait_for_clipboard_change", "arguments": { "cursor": cursor, "timeoutMs": 30000 } }
        })).send().await.expect("admitted long wait");
        assert_eq!(waiting.status(), StatusCode::OK);
        assert!(state.sessions.lock().expect("session activity").get(&wire.session).expect("live session").active_requests >= 2,
            "the notification stream and unfinished wait are both admitted");
        context.changes.record(Arc::new(crate::clipboard::ClipboardChange { cursor: uuid::Uuid::new_v4(), timestamp: 123456,
            content: TextContent { text: Some("通知 😀".into()), ..Default::default() }, image_bytes: None }));
        let notification = timeout(Duration::from_secs(2), async {
            let mut bytes = Vec::new();
            loop {
                let part = stream.next().await.expect("notification connection remains open").expect("SSE bytes");
                bytes.extend_from_slice(&part);
                assert!(bytes.len() < 64 * 1024);
                if let Some(value) = rpc_payload(&bytes).filter(|value| value["method"] == "notifications/resources/updated") { break value; }
            }
        }).await.expect("resource update delivered");
        assert_eq!(notification["params"]["uri"], uri);
        let event = &notification["params"]["_meta"][super::super::types::MCP_CLIPBOARD_EVENT_META_KEY];
        assert_eq!(event["content"]["text"], "通知 😀");
        assert_eq!(event["timestamp"], 123456.0);
        let changed = rpc_payload(&waiting.bytes().await.expect("completed long wait")).expect("long wait RPC reply");
        assert_eq!(changed["id"], "await-update");
        assert_eq!(changed["result"]["structuredContent"]["changed"], true);
        assert_eq!(changed["result"]["structuredContent"]["cursor"], event["cursor"]);
        assert!(wire.rpc("resources/unsubscribe", json!({ "uri": uri })).await.get("error").is_none());
        assert!(context.changes.subscribed_peers().is_empty());
        assert_eq!(wire.tool("subscribe_clipboard_changes", json!({})).await["structuredContent"]["subscribed"], true);
        assert_eq!(wire.tool("unsubscribe_clipboard_changes", json!({})).await["structuredContent"]["subscribed"], false);
        assert!(context.changes.subscribed_peers().is_empty());
        assert_eq!(wire.tool("subscribe_clipboard_changes", json!({})).await["structuredContent"]["subscribed"], true);
        let (subscription, session_cancel) = {
            let peers = context.changes.subscribed_peers();
            assert_eq!(peers.len(), 1);
            (Arc::downgrade(&peers[0].1), peers[0].1.cancellation().clone())
        };
        let mut rejected = wire.request(Method::DELETE).build().expect("rejected close request");
        rejected.headers_mut().insert("mcp-protocol-version", HeaderValue::from_static("1990-01-01"));
        assert_eq!(wire.client.execute(rejected).await.expect("rejected close").status(), StatusCode::BAD_REQUEST);
        assert!(!session_cancel.is_cancelled(), "a rejected DELETE must preserve application request lifetimes");
        let waiting = wire.request(Method::POST).json(&json!({ "jsonrpc": "2.0", "id": "await-close", "method": "tools/call",
            "params": { "name": "wait_for_clipboard_change", "arguments": { "cursor": event["cursor"], "timeoutMs": 30000 } }
        })).send().await.expect("wait admitted before session close");
        assert_eq!(waiting.status(), StatusCode::OK);
        assert!(state.sessions.lock().expect("session activity").get(&wire.session).expect("live session").active_requests >= 2);
        assert_eq!(wire.request(Method::DELETE).send().await.expect("terminate session").status(), StatusCode::ACCEPTED);
        assert!(session_cancel.is_cancelled(), "successful DELETE cancels the application's session token");
        match timeout(Duration::from_secs(2), waiting.bytes()).await.expect("closing a session must stop its long wait before the request timeout") {
            Ok(body) => if let Some(reply) = rpc_payload(&body) {
                assert_eq!(reply["id"], "await-close");
                assert!(reply.get("error").is_some() || reply["result"]["isError"] == true, "a closed wait must not report a successful timeout: {reply}");
            },
            Err(error) => assert!(error.is_body() && !error.is_timeout(), "only a transport body closure is expected: {error}"),
        }
        timeout(Duration::from_secs(2), async {
            while subscription.strong_count() != 0 { tokio::task::yield_now().await; }
        }).await.expect("closed session finishes its handler and releases the actual subscription allocation");
        assert!(context.changes.subscribed_peers().is_empty());
        drop(stream);
        assert!(state.manager.sessions.read().await.is_empty());
        assert_eq!(wire.request(Method::POST).json(&json!({ "jsonrpc": "2.0", "id": 99, "method": "ping" })).send().await.expect("closed session request").status(), StatusCode::NOT_FOUND);
        cancel.cancel();
        server.await.expect("HTTP task").expect("HTTP shutdown");
        notifications.await.expect("notification shutdown");
        let gate = ReadGate::new();
        *native.read_gate.lock().expect("read gate") = Some(gate.clone());
        let readers = CancellationToken::new();
        let mut accepted = Vec::new();
        for _ in 0..16 {
            let clipboard = clipboard.clone(); let readers = readers.clone();
            accepted.push(tokio::spawn(async move { clipboard.read(&readers).await }));
        }
        gate.entered.acquire_many(16).await.expect("all accepted reads reached the backend").forget();
        assert!(matches!(clipboard.read(&CancellationToken::new()).await, Err(crate::Error::ResourceLimit(_))), "reads share the existing operation admission bound");
        readers.cancel();
        gate.cancelled.acquire_many(16).await.expect("caller cancellation reached every read").forget();
        assert!(accepted.iter().all(|read| !read.is_finished()), "cancellation keeps accepted operations until their actual return");
        gate.release.add_permits(16);
        for read in accepted { assert!(matches!(read.await.expect("cancelled read joined"), Err(crate::Error::Cancelled))); }
        *native.read_gate.lock().expect("read gate") = None;
        assert_eq!(clipboard.read(&CancellationToken::new()).await.expect("read after cancellation").content.text.as_deref(), Some("写入\0文字 😀"));

        *native.read_gate.lock().expect("read gate") = Some(gate.clone());
        let abandoned = tokio::spawn({ let clipboard = clipboard.clone(); async move { clipboard.read(&CancellationToken::new()).await } });
        gate.entered.acquire().await.expect("abandoned read entered").forget();
        abandoned.abort();
        assert!(matches!(abandoned.await, Err(error) if error.is_cancelled()));
        gate.cancelled.acquire().await.expect("caller destruction cancels its owned read").forget();
        let stopping = tokio::spawn({ let clipboard = clipboard.clone(); async move { clipboard.read(&CancellationToken::new()).await } });
        gate.entered.acquire().await.expect("live read before shutdown").forget();
        let shutdown = clipboard.shutdown();
        tokio::pin!(shutdown);
        assert!(futures_util::poll!(shutdown.as_mut()).is_pending(), "shutdown retains the abandoned physical read");
        gate.cancelled.acquire().await.expect("shutdown cancels the live read").forget();
        assert!(!stopping.is_finished());
        assert!(futures_util::poll!(shutdown.as_mut()).is_pending(), "shutdown also waits for the cancelled live read");
        gate.release.add_permits(2);
        shutdown.await.expect("clipboard shutdown drains both reads");
        assert!(matches!(stopping.await.expect("shutdown read joined"), Err(crate::Error::Cancelled)));
        assert!(matches!(clipboard.read(&CancellationToken::new()).await, Err(crate::Error::Cancelled)));
        assert_eq!(gate.entered.available_permits(), 0, "closed reads do not enter the backend");
        ocr.set_suspended(true);
        let queued = ocr.enqueue_record(record.id, crate::ocr::OcrTrigger::Manual).await.expect("queue before suspended shutdown").expect("shutdown ticket");
        ocr.shutdown().await.expect("OCR shutdown");
        assert!(matches!(queued.wait(&CancellationToken::new()).await, Err(crate::Error::Cancelled)), "shutdown cancels queued work without requiring a wake event");
        configuration.shutdown().await.expect("configuration shutdown");
        assert_eq!(images.memory().scratch, 0);
        images.shutdown().await.expect("image shutdown");
        storage.close_for_test().await.expect("close database");
    }).await.expect("isolated protocol interop completes within fifteen seconds");
}
