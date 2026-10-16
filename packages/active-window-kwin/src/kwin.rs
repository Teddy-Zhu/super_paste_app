use crate::{WindowBounds, WindowInfo};
use std::sync::Mutex;

// kdotool names its temporary scripts using milliseconds. Guard the native
// entry point as well as the JS facade, including calls from Node workers.
static QUERY_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn get_active_window() -> Result<Option<WindowInfo>, String> {
    let _guard = QUERY_LOCK.lock().map_err(|error| error.to_string())?;
    let window = match kdotool::get_active_window_info() {
        Ok(window) => window,
        // Upstream exposes string errors; this exact error means an empty desktop.
        Err(error) if error.to_string() == "KWin script error: No active window" => {
            return Ok(None);
        }
        Err(error) => return Err(format!("KWin active window query failed: {error}")),
    };

    // /proc can disappear when the process exits, or be inaccessible in a sandbox.
    // Keep the window snapshot and explicitly mark its executable as unavailable.
    let path = std::fs::read_link(format!("/proc/{}/exe", window.pid))
        .ok()
        .map(|path| path.to_string_lossy().into_owned());

    Ok(Some(WindowInfo {
        id: window.id,
        title: window.title,
        application: window.class_name.clone(),
        window_class: window.class_name,
        pid: window.pid,
        path,
        bounds: WindowBounds {
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
        },
    }))
}
