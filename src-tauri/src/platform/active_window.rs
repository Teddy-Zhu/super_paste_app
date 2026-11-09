use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const ICON_RETRY_DELAY: Duration = Duration::from_secs(5);

struct CachedIcon {
    data: Option<String>,
    needs_retry: bool,
    window_id: String,
    attempted_at: Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveWindow {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub native: Option<x_win::WindowInfo>,
}

fn current() -> Result<ActiveWindow, String> {
    #[cfg(target_os = "linux")]
    if clipboard_core::is_wayland_session()
        && std::env::var("KDE_SESSION_VERSION").as_deref() == Ok("6")
    {
        let window = super::kwin::active_window().map_err(|e| e.to_string())?;
        let path = std::fs::read_link(format!("/proc/{}/exe", window.pid))
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        return Ok(ActiveWindow {
            id: window.id,
            name: window.class_name,
            path,
            native: None,
        });
    }
    let window = x_win::get_active_window().map_err(|e| format!("活动窗口查询失败: {e}"))?;
    if window.id == 0 || window.info.name.trim().is_empty() {
        return Err("当前没有可识别的活动应用窗口".into());
    }
    Ok(ActiveWindow {
        id: window.id.to_string(),
        name: window.info.name.clone(),
        path: (!window.info.path.is_empty()).then(|| window.info.path.clone()),
        native: Some(window),
    })
}

#[derive(Default)]
struct SourceState {
    source: Option<Value>,
    screenshot_source: Option<Value>,
    screenshot_active: bool,
    error: Option<String>,
}

pub struct SourceTracker {
    state: Arc<Mutex<SourceState>>,
    cancelled: Arc<AtomicBool>,
    frozen: Arc<AtomicBool>,
}

impl SourceTracker {
    pub fn start(app: tauri::AppHandle) -> Result<Arc<Self>, String> {
        let tracker = Arc::new(Self {
            state: Arc::new(Mutex::new(SourceState::default())),
            cancelled: Arc::new(AtomicBool::new(false)),
            frozen: Arc::new(AtomicBool::new(false)),
        });
        let state = tracker.state.clone();
        let cancelled = tracker.cancelled.clone();
        let frozen = tracker.frozen.clone();
        thread::Builder::new().name("source-tracker".into()).spawn(move || {
            let mut icons: HashMap<String, CachedIcon> = HashMap::new();
            while !cancelled.load(Ordering::Acquire) {
                if !frozen.load(Ordering::Acquire) {
                    match current() {
                        Ok(window) => {
                            let name = window.name.to_lowercase();
                            // Our windows retain the target; mark-shot's prior target is selected for images only.
                            if !name.contains("super_paste") && !name.contains("super paste") && !name.contains("super-paste") && !name.contains("com.v2dawn.superpaste") {
                                let key = format!("{}\0{}", window.name, window.path.as_deref().unwrap_or(""));
                                if icons.get(&key).is_none_or(|cached| {
                                    cached.needs_retry && (cached.window_id != window.id || cached.attempted_at.elapsed() >= ICON_RETRY_DELAY)
                                }) {
                                    let data = match super::source_icon::load(&window, &app) {
                                        Ok(data) => Some(data),
                                        Err(error) => {
                                            tracing::debug!(application = %window.name, error = ?error, "source application icon unavailable; retry scheduled");
                                            None
                                        }
                                    };
                                    let needs_retry = data.as_deref().is_none_or(super::source_icon::needs_refresh);
                                    icons.insert(key.clone(), CachedIcon { data, needs_retry, window_id: window.id.clone(), attempted_at: Instant::now() });
                                }
                                let icon = icons.get(&key).and_then(|cached| cached.data.clone());
                                let now = crate::events::now_ms();
                                if let Ok(mut guard) = state.lock() {
                                    let screenshot=name=="mark-shot";
                                    if screenshot&&!guard.screenshot_active{guard.screenshot_source=guard.source.clone();}else if !screenshot{guard.screenshot_source=None;}
                                    guard.screenshot_active=screenshot;
                                    guard.source = Some(json!({"application":window.name,"path":window.path.unwrap_or_default(),"icon":icon,"createdAt":now,"updatedAt":now}));
                                    guard.error = None;
                                } else { break; }
                            }
                        }
                        Err(error) => {
                            if let Ok(mut guard) = state.lock() { guard.error = Some(error); guard.source = None; guard.screenshot_source=None;guard.screenshot_active=false; } else { break; }
                        }
                    }
                }
                thread::sleep(Duration::from_millis(80));
            }
        }).map_err(|e| e.to_string())?;
        Ok(tracker)
    }

    pub fn snapshot(&self, has_image: bool) -> Result<Option<Value>, String> {
        self.state
            .lock()
            .map(|s| {
                if has_image
                    && s.screenshot_active
                    && s.screenshot_source.as_ref().is_some_and(|v| {
                        v["application"].as_str().is_some_and(|v| !v.is_empty())
                            && v["path"].as_str().is_some_and(|v| !v.is_empty())
                    })
                {
                    s.screenshot_source.clone()
                } else {
                    s.source.clone()
                }
            })
            .map_err(|_| "来源状态锁已损坏".into())
    }
    pub fn available(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|s| s.error.is_none() && s.source.is_some())
    }
    pub fn freeze(&self, frozen: bool) {
        self.frozen.store(frozen, Ordering::Release);
    }
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Drop for SourceTracker {
    fn drop(&mut self) {
        self.stop();
    }
}
