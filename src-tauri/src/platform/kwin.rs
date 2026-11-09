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
    sync::Mutex,
    sync::mpsc,
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
