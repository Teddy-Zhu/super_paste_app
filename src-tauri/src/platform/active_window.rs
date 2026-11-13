use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

const ICON_RETRY_DELAY: Duration = Duration::from_secs(5);
const IMAGE_SOURCE_GRACE_PERIOD: Duration = Duration::from_secs(2);
const EVENT_FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);

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
        if window.class_name.trim().is_empty() {
            return Err("当前活动窗口没有可识别的应用标识".into());
        }
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
    #[cfg(target_os = "linux")]
    if !clipboard_core::is_wayland_session() {
        return super::source_window_x11::current();
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
    interrupted_image_source: Option<(Value, Instant)>,
    error: Option<String>,
}

impl SourceState {
    fn image_source(&self) -> Option<&Value> {
        if self.screenshot_active
            && let Some(source) = &self.screenshot_source
        {
            return Some(source);
        }
        self.source.as_ref()
    }

    fn snapshot(&self, has_image: bool, now: Instant) -> Option<Value> {
        if !has_image {
            return self.source.clone();
        }
        self.image_source()
            .or_else(|| {
                self.interrupted_image_source
                    .as_ref()
                    .filter(|(_, interrupted_at)| {
                        now.saturating_duration_since(*interrupted_at) < IMAGE_SOURCE_GRACE_PERIOD
                    })
                    .map(|(source, _)| source)
            })
            .cloned()
    }

    fn unavailable(&mut self, error: String, now: Instant) {
        // 只覆盖截图结束时的短暂焦点交接；重复失败不延长旧来源的有效期。
        if let Some(source) = self.image_source() {
            self.interrupted_image_source = Some((source.clone(), now));
        }
        self.source = None;
        self.screenshot_source = None;
        self.screenshot_active = false;
        self.error = Some(error);
    }
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
        thread::Builder::new()
            .name("source-tracker".into())
            .spawn(move || {
                let mut icons: HashMap<String, CachedIcon> = HashMap::new();
                let kde_wayland = cfg!(target_os = "linux")
                    && clipboard_core::is_wayland_session()
                    && std::env::var("KDE_SESSION_VERSION").as_deref() == Ok("6");
                if kde_wayland
                    && track_kde_events(&app, &state, &cancelled, &frozen, &mut icons)
                    && !cancelled.load(Ordering::Acquire)
                {
                    tracing::warn!(
                        "KWin active-window listener unavailable; using slower query fallback"
                    );
                    track_polling(
                        &app,
                        &state,
                        &cancelled,
                        &frozen,
                        &mut icons,
                        EVENT_FALLBACK_POLL_INTERVAL,
                    );
                } else if !cancelled.load(Ordering::Acquire) {
                    let event_listener_failed =
                        track_platform_events(&app, &state, &cancelled, &frozen, &mut icons);
                    if event_listener_failed && !cancelled.load(Ordering::Acquire) {
                        tracing::warn!(
                            "Active-window event listener unavailable; using slower query fallback"
                        );
                        track_polling(
                            &app,
                            &state,
                            &cancelled,
                            &frozen,
                            &mut icons,
                            EVENT_FALLBACK_POLL_INTERVAL,
                        );
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(tracker)
    }

    pub fn snapshot(&self, has_image: bool) -> Result<Option<Value>, String> {
        self.state
            .lock()
            .map(|state| state.snapshot(has_image, Instant::now()))
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

fn track_polling(
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    cancelled: &AtomicBool,
    frozen: &AtomicBool,
    icons: &mut HashMap<String, CachedIcon>,
    poll_interval: Duration,
) {
    let mut current_window = None;
    while !cancelled.load(Ordering::Acquire) {
        if !frozen.load(Ordering::Acquire) {
            match current() {
                Ok(window) => {
                    if !is_own_application(&window.name) {
                        update_source(window, app, state, icons, &mut current_window, frozen);
                    }
                }
                Err(error) => set_source_error(state, error, &mut current_window),
            }
        }
        thread::sleep(poll_interval);
    }
}

#[cfg(target_os = "linux")]
fn track_kde_events(
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    cancelled: &Arc<AtomicBool>,
    frozen: &AtomicBool,
    icons: &mut HashMap<String, CachedIcon>,
) -> bool {
    let watcher = match super::kwin::watch_active_window(cancelled.clone()) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(%error, "KWin active-window listener could not start");
            return true;
        }
    };
    let mut current_window = None;
    let mut pending: Option<Option<ActiveWindow>> = None;
    let mut was_frozen = frozen.load(Ordering::Acquire);
    apply_activation(
        watcher.initial.clone(),
        was_frozen,
        &mut pending,
        app,
        state,
        icons,
        &mut current_window,
        frozen,
    );

    while !cancelled.load(Ordering::Acquire) {
        let is_frozen = frozen.load(Ordering::Acquire);
        if is_frozen && !was_frozen {
            pending = None;
        } else if !is_frozen && was_frozen {
            restore_latest_source(&mut pending, app, state, icons, &mut current_window, frozen);
        }
        was_frozen = is_frozen;

        if !is_frozen
            && let Some(window) = current_window.clone()
            && icon_retry_due(&window, icons)
        {
            update_source(window, app, state, icons, &mut current_window, frozen);
        }

        match watcher.events.recv_timeout(Duration::from_millis(50)) {
            Ok(mut activation) => {
                while let Ok(next) = watcher.events.try_recv() {
                    activation = next;
                }
                apply_activation(
                    activation,
                    frozen.load(Ordering::Acquire),
                    &mut pending,
                    app,
                    state,
                    icons,
                    &mut current_window,
                    frozen,
                );
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if !cancelled.load(Ordering::Acquire) {
                    if !frozen.load(Ordering::Acquire) {
                        set_source_error(
                            state,
                            "KWin 活动窗口监听连接已关闭".into(),
                            &mut current_window,
                        );
                    }
                    return true;
                }
            }
        }
    }
    false
}

fn track_platform_events(
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    cancelled: &Arc<AtomicBool>,
    frozen: &AtomicBool,
    icons: &mut HashMap<String, CachedIcon>,
) -> bool {
    let watcher = match super::active_events::watch(app, cancelled.clone()) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(%error, "Active-window event listener could not start");
            return true;
        }
    };
    let mut current_window = None;
    let mut pending: Option<Option<ActiveWindow>> = None;
    let mut was_frozen = frozen.load(Ordering::Acquire);
    apply_current_snapshot(
        current(),
        was_frozen,
        &mut pending,
        app,
        state,
        icons,
        &mut current_window,
        frozen,
    );

    while !cancelled.load(Ordering::Acquire) {
        let is_frozen = frozen.load(Ordering::Acquire);
        if is_frozen && !was_frozen {
            pending = None;
        } else if !is_frozen && was_frozen {
            restore_latest_source(&mut pending, app, state, icons, &mut current_window, frozen);
        }
        was_frozen = is_frozen;

        if !is_frozen
            && let Some(window) = current_window.clone()
            && icon_retry_due(&window, icons)
        {
            update_source(window, app, state, icons, &mut current_window, frozen);
        }

        match watcher.events.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(())) => {
                let mut listener_error = None;
                loop {
                    match watcher.events.try_recv() {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            listener_error = Some(error);
                            break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            listener_error = Some("活动窗口事件通道已关闭".into());
                            break;
                        }
                    }
                }
                if let Some(error) = listener_error {
                    if !frozen.load(Ordering::Acquire) {
                        set_source_error(state, error, &mut current_window);
                    }
                    return true;
                }
                apply_current_snapshot(
                    current(),
                    frozen.load(Ordering::Acquire),
                    &mut pending,
                    app,
                    state,
                    icons,
                    &mut current_window,
                    frozen,
                );
            }
            Ok(Err(error)) => {
                if !frozen.load(Ordering::Acquire) {
                    set_source_error(state, error, &mut current_window);
                }
                return true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if !frozen.load(Ordering::Acquire) {
                    set_source_error(state, "活动窗口事件通道已关闭".into(), &mut current_window);
                }
                return true;
            }
        }
    }
    false
}

fn apply_current_snapshot(
    result: Result<ActiveWindow, String>,
    frozen_now: bool,
    pending: &mut Option<Option<ActiveWindow>>,
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    match result {
        Ok(window) if is_own_application(&window.name) => {}
        Ok(window) if frozen_now => *pending = Some(Some(window)),
        Ok(window) => update_source(window, app, state, icons, current_window, frozen),
        Err(_) if frozen_now => {}
        Err(error) => set_source_error(state, error, current_window),
    }
}

fn restore_latest_source(
    pending: &mut Option<Option<ActiveWindow>>,
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    let pending_window = pending.take();
    match current() {
        Ok(window) if !is_own_application(&window.name) => {
            update_source(window, app, state, icons, current_window, frozen);
        }
        Ok(_) => {
            if let Some(window) = pending_window {
                apply_pending(window, app, state, icons, current_window, frozen);
            }
        }
        Err(error) => {
            if let Some(window) = pending_window {
                apply_pending(window, app, state, icons, current_window, frozen);
            } else {
                set_source_error(state, error, current_window);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn track_kde_events(
    _app: &tauri::AppHandle,
    _state: &Arc<Mutex<SourceState>>,
    _cancelled: &Arc<AtomicBool>,
    _frozen: &AtomicBool,
    _icons: &mut HashMap<String, CachedIcon>,
) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn apply_activation(
    activation: super::kwin::Activation,
    frozen_now: bool,
    pending: &mut Option<Option<ActiveWindow>>,
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    match activation {
        Err(error) => {
            if !frozen_now {
                set_source_error(state, error, current_window);
            }
        }
        Ok(None) => {
            if frozen_now {
                *pending = Some(None);
            } else {
                set_source_error(state, "当前没有可识别的活动应用窗口".into(), current_window);
            }
        }
        Ok(Some(window)) => {
            let name = window.class_name;
            if name.trim().is_empty() {
                if frozen_now {
                    *pending = Some(None);
                } else {
                    set_source_error(
                        state,
                        "当前活动窗口没有可识别的应用标识".into(),
                        current_window,
                    );
                }
                return;
            }
            if is_own_application(&name) {
                return;
            }
            let path = std::fs::read_link(format!("/proc/{}/exe", window.pid))
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
            let window = ActiveWindow {
                id: window.id,
                name,
                path,
                native: None,
            };
            if frozen_now {
                *pending = Some(Some(window));
            } else {
                update_source(window, app, state, icons, current_window, frozen);
            }
        }
    }
}

fn apply_pending(
    window: Option<ActiveWindow>,
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    match window {
        Some(window) if !is_own_application(&window.name) => {
            update_source(window, app, state, icons, current_window, frozen);
        }
        Some(_) => {}
        None => set_source_error(state, "当前没有可识别的活动应用窗口".into(), current_window),
    }
}

fn update_source(
    window: ActiveWindow,
    app: &tauri::AppHandle,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    if frozen.load(Ordering::Acquire) || is_own_application(&window.name) {
        return;
    }
    let name = window.name.to_lowercase();
    let key = format!("{}\0{}", window.name, window.path.as_deref().unwrap_or(""));
    let same_window = current_window.as_ref().is_some_and(|current| {
        current.id == window.id && current.name == window.name && current.path == window.path
    });
    let retry_due = icons.get(&key).is_some_and(|cached| {
        cached.needs_retry && cached.attempted_at.elapsed() >= ICON_RETRY_DELAY
    });
    if same_window && !retry_due {
        return;
    }
    if icons.get(&key).is_none_or(|cached| {
        cached.needs_retry
            && (cached.window_id != window.id || cached.attempted_at.elapsed() >= ICON_RETRY_DELAY)
    }) {
        let data = match super::source_icon::load(&window, app) {
            Ok(data) => Some(data),
            Err(error) => {
                tracing::debug!(application = %window.name, error = ?error, "source application icon unavailable; retry scheduled");
                None
            }
        };
        let needs_retry = data
            .as_deref()
            .is_none_or(super::source_icon::needs_refresh);
        icons.insert(
            key.clone(),
            CachedIcon {
                data,
                needs_retry,
                window_id: window.id.clone(),
                attempted_at: Instant::now(),
            },
        );
    }
    if frozen.load(Ordering::Acquire) {
        return;
    }
    let icon = icons.get(&key).and_then(|cached| cached.data.clone());
    let now = crate::events::now_ms();
    if let Ok(mut guard) = state.lock() {
        if frozen.load(Ordering::Acquire) {
            return;
        }
        let screenshot = name == "mark-shot";
        if screenshot && !guard.screenshot_active {
            guard.screenshot_source = guard.source.clone();
        } else if !screenshot {
            guard.screenshot_source = None;
        }
        guard.screenshot_active = screenshot;
        guard.source = Some(json!({
            "application": window.name.clone(),
            "path": window.path.clone().unwrap_or_default(),
            "icon": icon,
            "createdAt": now,
            "updatedAt": now
        }));
        guard.error = None;
        guard.interrupted_image_source = None;
        *current_window = Some(window);
    }
}

fn icon_retry_due(window: &ActiveWindow, icons: &HashMap<String, CachedIcon>) -> bool {
    let key = format!("{}\0{}", window.name, window.path.as_deref().unwrap_or(""));
    icons.get(&key).is_some_and(|cached| {
        cached.needs_retry && cached.attempted_at.elapsed() >= ICON_RETRY_DELAY
    })
}

fn set_source_error(
    state: &Arc<Mutex<SourceState>>,
    error: String,
    current_window: &mut Option<ActiveWindow>,
) {
    if let Ok(mut guard) = state.lock() {
        if guard.error.as_deref() != Some(error.as_str()) {
            tracing::debug!(%error, "活动窗口来源暂不可用");
        }
        guard.unavailable(error, Instant::now());
        *current_window = None;
    }
}

fn is_own_application(name: &str) -> bool {
    let name = name.to_lowercase();
    name.contains("super_paste")
        || name.contains("super paste")
        || name.contains("super-paste")
        || name.contains("com.v2dawn.superpaste")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_source_survives_brief_focus_gaps_without_extending_stale_attribution() {
        let now = Instant::now();
        let wechat = json!({"application": "wechat", "path": "/opt/wechat/wechat"});
        let mut state = SourceState {
            source: Some(wechat.clone()),
            ..Default::default()
        };
        state.unavailable("窗口正在切换".into(), now);
        assert_eq!(
            state.snapshot(true, now + Duration::from_secs(1)),
            Some(wechat)
        );
        assert_eq!(state.snapshot(false, now), None);
        state.unavailable("窗口仍不可用".into(), now + Duration::from_secs(1));
        assert_eq!(state.snapshot(true, now + IMAGE_SOURCE_GRACE_PERIOD), None);
    }

    #[test]
    fn screenshot_source_does_not_require_an_executable_path() {
        let now = Instant::now();
        let source = json!({"application": "wechat", "path": ""});
        let mut state = SourceState {
            source: Some(json!({"application": "mark-shot", "path": "/opt/mark-shot"})),
            screenshot_source: Some(source.clone()),
            screenshot_active: true,
            ..Default::default()
        };
        assert_eq!(state.snapshot(true, now), Some(source.clone()));
        assert_eq!(
            state.snapshot(false, now).unwrap()["application"],
            "mark-shot"
        );
        state.unavailable("截图窗口已关闭".into(), now);
        assert_eq!(state.snapshot(true, now), Some(source));
    }
}
