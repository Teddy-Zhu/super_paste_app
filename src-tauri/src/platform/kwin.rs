// Library/system API adapter adapted from super_paste_rs_native; see THIRD-PARTY-NOTICES.md.
use anyhow::{Context, Result, anyhow};
use dbus::{
    blocking::{Connection, SyncConnection},
    channel::MatchingReceiver,
    message::MatchRule,
};
use serde::Deserialize;
use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static QUERY_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ActiveWindow {
    pub id: String,
    pub class_name: String,
    pub pid: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Area {
    pub display_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkAreaSnapshot {
    pub cursor_area: Area,
    pub areas: Vec<Area>,
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveWindowEvent {
    pub id: String,
    pub class_name: String,
    pub pid: u32,
}

pub(crate) type Activation = Result<Option<ActiveWindowEvent>, String>;

pub(crate) struct ActiveWindowWatcher {
    pub initial: Activation,
    pub events: Receiver<Activation>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for ActiveWindowWatcher {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(crate) fn watch_active_window(
    source_cancelled: Arc<AtomicBool>,
) -> Result<ActiveWindowWatcher> {
    ensure_kde6()?;
    anyhow::ensure!(
        !source_cancelled.load(Ordering::Acquire),
        "KWin 活动窗口监听已取消"
    );
    let (event_sender, events) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker_ready = ready_sender.clone();
    let worker_source_cancelled = source_cancelled.clone();
    let worker = thread::Builder::new()
        .name("kwin-active-window-watch".into())
        .spawn(move || {
            if let Err(error) = active_window_listener(
                event_sender,
                ready_sender,
                worker_cancelled.clone(),
                worker_source_cancelled,
            ) {
                tracing::warn!(%error, "KWin active-window listener stopped");
                worker_cancelled.store(true, Ordering::Release);
                let _ = worker_ready.send(Err(error.to_string()));
            }
        })?;
    let initial = match ready_receiver.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(initial)) => initial,
        Ok(Err(error)) => {
            cancelled.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(anyhow!(error));
        }
        Err(error) => {
            cancelled.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(anyhow!("等待 KWin 活动窗口监听初始化失败: {error}"));
        }
    };
    Ok(ActiveWindowWatcher {
        initial,
        events,
        cancelled,
        worker: Some(worker),
    })
}

fn active_window_listener(
    event_sender: mpsc::Sender<Activation>,
    ready_sender: mpsc::SyncSender<Result<Activation, String>>,
    cancelled: Arc<AtomicBool>,
    source_cancelled: Arc<AtomicBool>,
) -> Result<()> {
    let self_connection = SyncConnection::new_session().context("连接用户 D-Bus 失败")?;
    let dbus_address = self_connection.unique_name().to_string();
    let (message_sender, message_receiver) = mpsc::channel::<String>();
    let _signal_match = self_connection.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |message, _connection| {
            if message
                .member()
                .is_some_and(|member| &*member == "active_window")
                && let Some(payload) = message.get1::<String>()
            {
                let _ = message_sender.send(payload);
            }
            true
        }),
    );

    let script_name = format!(
        "super-paste-active-window-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("读取系统时间失败")?
            .as_nanos()
    );
    let script = format!(
        r#"
function publishActiveWindow(window) {{
    const active = window ? {{
        active: true,
        id: String(window.windowId || window.internalId || window.pid),
        class_name: String(window.resourceClass || window.resourceName || window.caption || ""),
        pid: Number(window.pid)
    }} : {{ active: false }};
    callDBus("{dbus_address}", "/", "", "active_window", JSON.stringify(active));
}}
try {{
    workspace.windowActivated.connect(window => publishActiveWindow(window));
    publishActiveWindow(workspace.activeWindow);
}} catch (e) {{
    callDBus("{dbus_address}", "/", "", "active_window", JSON.stringify({{ error: e.toString() }}));
}}
"#
    );
    let mut file = tempfile::NamedTempFile::with_prefix("super-paste-kwin-")
        .context("无法创建 KWin 临时脚本")?;
    file.write_all(script.as_bytes())
        .context("无法写入 KWin 临时脚本")?;
    let script_file_path = file.into_temp_path();
    let kwin_connection = Connection::new_session().context("连接 KWin D-Bus 失败")?;
    let scripting_proxy =
        kwin_connection.with_proxy("org.kde.KWin", "/Scripting", Duration::from_secs(5));
    let (script_id,): (i32,) = scripting_proxy
        .method_call(
            "org.kde.kwin.Scripting",
            "loadScript",
            (
                script_file_path
                    .to_str()
                    .ok_or_else(|| anyhow!("KWin 临时脚本路径不是有效 UTF-8"))?,
                &script_name,
            ),
        )
        .context("加载 KWin 活动窗口监听脚本失败")?;
    anyhow::ensure!(script_id >= 0, "KWin 拒绝加载活动窗口监听脚本");
    drop(script_file_path);
    let script_proxy = kwin_connection.with_proxy(
        "org.kde.KWin",
        format!("/Scripting/Script{script_id}"),
        Duration::from_secs(5),
    );
    if let Err(error) = script_proxy.method_call::<(), _, _, _>("org.kde.kwin.Script", "run", ()) {
        stop_active_window_script(&kwin_connection, script_id, &script_name);
        return Err(error).context("启动 KWin 活动窗口监听脚本失败");
    }

    let initial_started = Instant::now();
    let initial = loop {
        if cancelled.load(Ordering::Acquire) || source_cancelled.load(Ordering::Acquire) {
            stop_active_window_script(&kwin_connection, script_id, &script_name);
            return Err(anyhow!("KWin 活动窗口监听已取消"));
        }
        if let Err(error) = self_connection.process(Duration::from_millis(100)) {
            stop_active_window_script(&kwin_connection, script_id, &script_name);
            return Err(error).context("处理 KWin 活动窗口初始化事件失败");
        }
        if let Ok(payload) = message_receiver.try_recv() {
            break parse_activation(&payload);
        }
        if initial_started.elapsed() >= Duration::from_secs(8) {
            stop_active_window_script(&kwin_connection, script_id, &script_name);
            return Err(anyhow!("等待 KWin 初始活动窗口事件超时"));
        }
    };
    let initial = match initial {
        Ok(initial) => initial,
        Err(error) => {
            stop_active_window_script(&kwin_connection, script_id, &script_name);
            return Err(anyhow!(error));
        }
    };
    if ready_sender.send(Ok(Ok(initial))).is_err() {
        stop_active_window_script(&kwin_connection, script_id, &script_name);
        return Ok(());
    }

    while !cancelled.load(Ordering::Acquire) && !source_cancelled.load(Ordering::Acquire) {
        if let Err(error) = self_connection.process(Duration::from_millis(100)) {
            stop_active_window_script(&kwin_connection, script_id, &script_name);
            return Err(error).context("处理 KWin 活动窗口事件失败");
        }
        while let Ok(payload) = message_receiver.try_recv() {
            if event_sender.send(parse_activation(&payload)).is_err() {
                stop_active_window_script(&kwin_connection, script_id, &script_name);
                return Ok(());
            }
        }
    }
    stop_active_window_script(&kwin_connection, script_id, &script_name);
    Ok(())
}

fn parse_activation(payload: &str) -> Activation {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|error| format!("KWin 活动窗口事件 JSON 无效: {error}"))?;
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        return Err(format!("KWin 无法订阅活动窗口事件: {error}"));
    }
    if value.get("active").and_then(serde_json::Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let id = value
        .get("id")
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_u64().map(|id| id.to_string()))
        })
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "KWin 活动窗口事件缺少窗口 ID".to_owned())?;
    let class_name = value
        .get("class_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let pid = value
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| "KWin 活动窗口事件缺少有效 PID".to_owned())?;
    Ok(Some(ActiveWindowEvent {
        id,
        class_name,
        pid,
    }))
}

fn stop_active_window_script(connection: &Connection, script_id: i32, script_name: &str) {
    let script_proxy = connection.with_proxy(
        "org.kde.KWin",
        format!("/Scripting/Script{script_id}"),
        Duration::from_secs(1),
    );
    let scripting_proxy =
        connection.with_proxy("org.kde.KWin", "/Scripting", Duration::from_secs(1));
    let _ = script_proxy.method_call::<(), _, _, _>("org.kde.kwin.Script", "stop", ());
    let _ = scripting_proxy.method_call::<(), _, _, _>(
        "org.kde.kwin.Scripting",
        "unloadScript",
        (script_name,),
    );
}

pub(crate) fn active_window() -> Result<ActiveWindow> {
    ensure_kde6()?;
    let _guard = QUERY_LOCK
        .lock()
        .map_err(|error| anyhow!(error.to_string()))?;
    let window = kdotool::get_active_window_info()
        .map_err(|error| anyhow!("KWin 活动窗口查询失败：{error}"))?;
    Ok(ActiveWindow {
        id: window.id,
        class_name: window.class_name,
        pid: window.pid,
    })
}

pub(crate) fn work_area(display_id: Option<&str>) -> Result<Area> {
    ensure_kde6()?;
    let _guard = QUERY_LOCK
        .lock()
        .map_err(|error| anyhow!(error.to_string()))?;
    let requested_screen = display_id
        .map(serde_json::to_string)
        .transpose()
        .context("无法编码 KWin 输出名称")?;
    let script = format!(
        r#"
    const outputs = workspace.screens;
    if (!outputs || outputs.length === 0) {{
        output_error("KWin 没有可用屏幕");
    }} else {{
        const requested = {requested};
        const output = requested === null
            ? workspace.screenAt(workspace.cursorPos)
            : outputs.find(item => item.name === requested);
        if (!output) {{
            output_error("请求的 KWin 输出不存在");
        }} else {{
            const area = workspace.clientArea(
                KWin.MaximizeArea,
                output,
                typeof workspace.currentDesktopForScreen === "function"
                    ? workspace.currentDesktopForScreen(output) : workspace.currentDesktop
            );
            output_result(JSON.stringify({{
                display_id: output.name,
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height
            }}));
        }}
    }}
"#,
        requested = requested_screen.as_deref().unwrap_or("null"),
    );
    let payload = run_script(&script)?;
    serde_json::from_str(&payload).context("无法解析 KWin MaximizeArea 响应")
}

pub(crate) fn work_areas() -> Result<Vec<Area>> {
    ensure_kde6()?;
    let _guard = QUERY_LOCK
        .lock()
        .map_err(|error| anyhow!(error.to_string()))?;
    let script = r#"
    const outputs = workspace.screens;
    if (!outputs || outputs.length === 0) {
        output_error("KWin 没有可用屏幕");
    } else {
        const areas = [];
        for (const output of outputs) {
            const area = workspace.clientArea(
                KWin.MaximizeArea,
                output,
                typeof workspace.currentDesktopForScreen === "function"
                    ? workspace.currentDesktopForScreen(output) : workspace.currentDesktop
            );
            areas.push({
                display_id: output.name,
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height
            });
        }
        output_result(JSON.stringify(areas));
    }
"#;
    let payload = run_script(script)?;
    serde_json::from_str(&payload).context("无法解析 KWin MaximizeArea 响应")
}

pub(crate) fn work_area_snapshot() -> Result<WorkAreaSnapshot> {
    ensure_kde6()?;
    let _guard = QUERY_LOCK
        .lock()
        .map_err(|error| anyhow!(error.to_string()))?;
    let script = r#"
    const outputs = workspace.screens;
    if (!outputs || outputs.length === 0) {
        output_error("KWin 没有可用屏幕");
    } else {
        const desktopFor = output =>
            typeof workspace.currentDesktopForScreen === "function"
                ? workspace.currentDesktopForScreen(output) : workspace.currentDesktop;
        const toArea = output => {
            const area = workspace.clientArea(KWin.MaximizeArea, output, desktopFor(output));
            return {
                display_id: output.name,
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height
            };
        };
        const currentOutput = workspace.screenAt(workspace.cursorPos);
        if (!currentOutput) {
            output_error("光标所在的 KWin 输出不存在");
        } else {
            output_result(JSON.stringify({
                cursor_area: toArea(currentOutput),
                areas: outputs.map(toArea)
            }));
        }
    }
"#;
    let payload = run_script(script)?;
    serde_json::from_str(&payload).context("无法解析 KWin 工作区快照")
}

fn ensure_kde6() -> Result<()> {
    anyhow::ensure!(
        std::env::var("KDE_SESSION_VERSION").as_deref() == Ok("6"),
        "当前 Wayland 会话不是 KDE Plasma 6"
    );
    Ok(())
}

fn run_script(body: &str) -> Result<String> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("读取系统时间失败")?
        .as_nanos();
    let script_name = format!("super-paste-{suffix}");
    let self_connection = SyncConnection::new_session().context("连接用户 D-Bus 失败")?;
    let dbus_address = self_connection.unique_name().to_string();
    let script = format!(
        r#"
function output_result(message) {{
    callDBus("{dbus_address}", "/", "", "result", message.toString());
}}
function output_error(message) {{
    callDBus("{dbus_address}", "/", "", "error", message.toString());
}}
function run() {{
    try {{
{body}
    }} catch (e) {{
        output_error("KWin script error: " + e.toString());
    }}
}}
run();
callDBus("{dbus_address}", "/", "", "finished", "{script_name}");
"#
    );

    enum ScriptMessage {
        Result(String),
        Error(String),
    }
    let (sender, receiver) = mpsc::channel();
    let _signal_match = self_connection.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |message, _connection| {
            if let Some(member) = message.member()
                && let Some(payload) = message.get1::<String>()
            {
                match member.as_ref() {
                    "result" => {
                        let _ = sender.send(ScriptMessage::Result(payload));
                    }
                    "error" => {
                        let _ = sender.send(ScriptMessage::Error(payload));
                    }
                    _ => {}
                }
            }
            true
        }),
    );
    let mut file = tempfile::NamedTempFile::with_prefix("super-paste-kwin-")
        .context("无法创建 KWin 临时脚本")?;
    file.write_all(script.as_bytes())
        .context("无法写入 KWin 临时脚本")?;
    let script_file_path = file.into_temp_path();
    let kwin_connection = Connection::new_session().context("连接 KWin D-Bus 失败")?;
    let scripting_proxy =
        kwin_connection.with_proxy("org.kde.KWin", "/Scripting", Duration::from_secs(5));
    let (script_id,): (i32,) = scripting_proxy
        .method_call(
            "org.kde.kwin.Scripting",
            "loadScript",
            (
                script_file_path
                    .to_str()
                    .ok_or_else(|| anyhow!("KWin 临时脚本路径不是有效 UTF-8"))?,
                &script_name,
            ),
        )
        .context("加载 KWin 查询脚本失败")?;
    anyhow::ensure!(script_id >= 0, "KWin 拒绝加载查询脚本");
    let script_proxy = kwin_connection.with_proxy(
        "org.kde.KWin",
        format!("/Scripting/Script{script_id}"),
        Duration::from_secs(5),
    );
    let run_result = script_proxy
        .method_call::<(), _, _, _>("org.kde.kwin.Script", "run", ())
        .context("运行 KWin 查询脚本失败");
    if run_result.is_ok() {
        let _ = script_proxy.method_call::<(), _, _, _>("org.kde.kwin.Script", "stop", ());
    }

    let response = if let Err(error) = run_result {
        Err(error)
    } else {
        let started = Instant::now();
        loop {
            self_connection
                .process(Duration::from_millis(100))
                .context("处理 KWin D-Bus 响应失败")?;
            match receiver.try_recv() {
                Ok(ScriptMessage::Result(payload)) => break Ok(payload),
                Ok(ScriptMessage::Error(message)) => break Err(anyhow!(message)),
                Err(mpsc::TryRecvError::Empty) if started.elapsed() < Duration::from_secs(5) => {}
                Err(mpsc::TryRecvError::Empty) => break Err(anyhow!("等待 KWin 查询响应超时")),
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(anyhow!("KWin 查询响应通道已关闭"));
                }
            }
        }
    };
    let _ = scripting_proxy.method_call::<(), _, _, _>(
        "org.kde.kwin.Scripting",
        "unloadScript",
        (&script_name,),
    );
    response
}
