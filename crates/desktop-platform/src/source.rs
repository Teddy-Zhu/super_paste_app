// Adapted from Super Paste, fc90106d5a25233fe45a6ec9facd1ad3ffc4823b; MIT.
use super_paste_format::{SourceApp, SourceId, APPLICATION_ID};
use crate::{main_thread::MainThread, source_icon::SourceIcons};
use bytes::Bytes;
use crossbeam_channel::{self as mpsc, RecvTimeoutError};
use uuid::Uuid;
type Source = Arc<SourceApp>;
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
const IMAGE_SOURCE_GRACE_PERIOD: Duration = Duration::from_secs(2);
const EVENT_FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);

struct CachedIcon {
    data: Option<Bytes>,
    needs_retry: bool,
    window_id: String,
    attempted_at: Instant,
    last_used: Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveWindow {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub native: Option<x_win::WindowInfo>,
    pub process_id: Option<u32>,
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
            process_id: Some(window.pid),
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
        process_id: Some(window.info.process_id),
        native: Some(window),
    })
}

#[derive(Clone, Default)]
struct SourceState {
    source: Option<Source>,
    screenshot_source: Option<Source>,
    screenshot_active: bool,
    interrupted_image_source: Option<(Source, Instant)>,
    error: Option<String>,
    window_id: Option<String>,
    listener: SourceListener,
    listener_error: Option<String>,
}

impl SourceState {
    fn image_source(&self) -> Option<&Source> {
        if self.screenshot_active
            && let Some(source) = &self.screenshot_source
        {
            return Some(source);
        }
        self.source.as_ref()
    }

    fn snapshot(&self, has_image: bool, now: Instant) -> Option<Source> {
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
    control: mpsc::Sender<()>,
    icons: Arc<SourceIcons>,
    worker: tokio::sync::Mutex<Option<SourceWorker>>,
}

struct SourceWorker {
    completed: tokio::sync::oneshot::Receiver<()>,
    thread: thread::JoinHandle<()>,
}

impl SourceWorker {
    fn spawn(run: impl FnOnce() + Send + 'static) -> std::io::Result<Self> {
        struct Completed(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Completed {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() { let _ = sender.send(()); }
            }
        }
        let (completed, completion) = tokio::sync::oneshot::channel();
        let thread = thread::Builder::new().name("source-tracker".into()).spawn(move || {
            let _completed = Completed(Some(completed));
            // Consuming the callback drops its captures before completion,
            // including native listeners and icon work, even during unwinding.
            run();
        })?;
        Ok(Self { completed: completion, thread })
    }

    async fn join(self, runtime: &tokio::runtime::Handle) -> Result<(), String> {
        let completed = self.completed.await.map_err(|error| format!("来源追踪完成信号异常: {error}"));
        // Source icons use this same pool. Await their actual completion before
        // occupying a worker with the physical join, including a one-worker pool.
        runtime.spawn_blocking(move || self.thread.join().map_err(|_| "来源追踪线程异常退出".to_string()))
            .await.map_err(|error| error.to_string())??;
        completed
    }
}

impl SourceTracker {
    /// `previous` has finished shutdown before a replacement starts. A frozen
    /// replacement inherits its captured source, sharing application/icon bytes.
    pub(crate) fn start(main_thread: MainThread, icons: Arc<SourceIcons>, initially_frozen: bool, previous: Option<&Self>) -> Result<Arc<Self>, String> {
        let mut initial = if initially_frozen {
            previous.map(|previous| previous.state.lock().map(|state| state.clone())
                .map_err(|_| String::from("来源状态锁已损坏"))).transpose()?.unwrap_or_default()
        } else { SourceState::default() };
        initial.listener = SourceListener::Starting;
        initial.listener_error = None;
        let state = Arc::new(Mutex::new(initial));
        let cancelled = Arc::new(AtomicBool::new(false));
        let frozen = Arc::new(AtomicBool::new(initially_frozen));
        let (control, receiver) = mpsc::bounded(1);
        let app = SourceContext { main_thread, icons: icons.clone(), control: receiver, frozen: frozen.clone() };
        let worker_state = state.clone(); let worker_cancelled = cancelled.clone(); let worker_frozen = frozen.clone();
        let worker = SourceWorker::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_source(&app, &worker_state, &worker_cancelled, &worker_frozen)));
            if !worker_cancelled.load(Ordering::Acquire) {
                set_source_error(&worker_state, "来源追踪线程异常退出".into(), &mut None);
            }
            if let Err(panic) = result { std::panic::resume_unwind(panic); }
        }).map_err(|e| e.to_string())?;
        Ok(Arc::new(Self { state, cancelled, frozen, control, icons, worker: tokio::sync::Mutex::new(Some(worker)) }))
    }

    pub fn snapshot(&self, has_image: bool) -> Result<Option<Source>, String> {
        self.state
            .lock()
            .map(|state| state.snapshot(has_image, Instant::now()))
            .map_err(|_| "来源状态锁已损坏".into())
    }
    pub fn available(&self) -> bool {
        if self.cancelled.load(Ordering::Acquire) { return false; }
        self.state
            .lock()
            .is_ok_and(|s| s.error.is_none() && s.source.is_some())
    }
    pub fn freeze(&self, frozen: bool) {
        self.frozen.store(frozen, Ordering::Release);
        let _ = self.control.try_send(());
    }
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.icons.cancel();
        let _ = self.control.try_send(());
    }
}

impl Drop for SourceTracker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn track_polling(
    app: &SourceContext,
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
                    if !is_own_window(&window) {
                        update_source(window, app, state, icons, &mut current_window, frozen);
                    }
                }
                Err(error) => set_source_error(state, error, &mut current_window),
            }
        }
        let _ = app.control.recv_timeout(poll_interval);
    }
}

#[cfg(target_os = "linux")]
fn track_kde_events(
    app: &SourceContext,
    state: &Arc<Mutex<SourceState>>,
    cancelled: &Arc<AtomicBool>,
    frozen: &AtomicBool,
    icons: &mut HashMap<String, CachedIcon>,
) -> bool {
    let watcher = match super::kwin::watch_active_window(cancelled.clone()) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(%error, "KWin active-window listener could not start");
            set_listener(state, SourceListener::Polling, Some(error.to_string()));
            return true;
        }
    };
    set_listener(state, SourceListener::Kwin, None);
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

        match receive_event(&watcher.events, app, current_window.as_ref(), icons) {
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
    app: &SourceContext,
    state: &Arc<Mutex<SourceState>>,
    cancelled: &Arc<AtomicBool>,
    frozen: &AtomicBool,
    icons: &mut HashMap<String, CachedIcon>,
) -> bool {
    let watcher = match super::active_events::watch(&app.main_thread, cancelled.clone()) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(%error, "Active-window event listener could not start");
            set_listener(state, SourceListener::Polling, Some(error));
            return true;
        }
    };
    set_listener(state, SourceListener::Events, None);
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

        match receive_event(&watcher.events, app, current_window.as_ref(), icons) {
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
    app: &SourceContext,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    match result {
        Ok(window) if is_own_window(&window) => {}
        Ok(window) if frozen_now => *pending = Some(Some(window)),
        Ok(window) => update_source(window, app, state, icons, current_window, frozen),
        Err(_) if frozen_now => {}
        Err(error) => set_source_error(state, error, current_window),
    }
}

fn restore_latest_source(
    pending: &mut Option<Option<ActiveWindow>>,
    app: &SourceContext,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    let pending_window = pending.take();
    match current() {
        Ok(window) if !is_own_window(&window) => {
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
    _app: &SourceContext,
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
    app: &SourceContext,
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
            if window.pid == std::process::id() {
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
                process_id: Some(window.pid),
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
    app: &SourceContext,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    match window {
        Some(window) if !is_own_window(&window) => {
            update_source(window, app, state, icons, current_window, frozen);
        }
        Some(_) => {}
        None => set_source_error(state, "当前没有可识别的活动应用窗口".into(), current_window),
    }
}

fn update_source(
    window: ActiveWindow,
    app: &SourceContext,
    state: &Arc<Mutex<SourceState>>,
    icons: &mut HashMap<String, CachedIcon>,
    current_window: &mut Option<ActiveWindow>,
    frozen: &AtomicBool,
) {
    if frozen.load(Ordering::Acquire) || is_own_window(&window) {
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
    if let Some(cached) = icons.get_mut(&key) { cached.last_used = Instant::now(); }
    if same_window && !retry_due {
        return;
    }
    if icons.get(&key).is_none_or(|cached| {
        cached.needs_retry
            && (cached.window_id != window.id || cached.attempted_at.elapsed() >= ICON_RETRY_DELAY)
    }) {
        let data = match app.icons.load(&window) {
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
                last_used: Instant::now(),
            },
        );
    }
    while icons.len() > 256 || icons.values().filter_map(|cached| cached.data.as_ref()).map(|data| data.len()).sum::<usize>() > super::source_icon::ICON_CACHE_BYTES {
        let victim = icons.iter().filter(|(candidate, _)| *candidate != &key).min_by_key(|(_, cached)| cached.last_used).map(|(key, _)| key.clone());
        match victim { Some(victim) => { icons.remove(&victim); }, None => break }
    }
    if frozen.load(Ordering::Acquire) {
        return;
    }
    let icon = icons.get(&key).and_then(|cached| cached.data.clone());
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
        guard.window_id = Some(window.id.clone());
        let id = SourceId(Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("{APPLICATION_ID}/sources/{key}").as_bytes()));
        guard.source = Some(Arc::new(SourceApp { id, application: window.name.clone().into(), path: window.path.clone().unwrap_or_default().into(), icon }));
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

fn is_own_window(window: &ActiveWindow) -> bool {
    match window.process_id {
        Some(pid) => pid == std::process::id(),
        None => matches!(window.name.to_lowercase().as_str(), "super-paste" | "super paste" | "com.v2dawn.superpaste.rs"),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceListener { #[default] Starting, Events, Kwin, Polling }
#[derive(Clone, Debug)]
pub struct SourceDiagnostics { pub available: bool, pub frozen: bool, pub closing: bool, pub error: Option<String>, pub listener: SourceListener, pub listener_error: Option<String> }
struct SourceContext { main_thread: MainThread, icons: Arc<SourceIcons>, control: mpsc::Receiver<()>, frozen: Arc<AtomicBool> }
impl SourceTracker {
    pub fn window_id(&self) -> Result<Option<String>, String> { self.state.lock().map(|state| state.window_id.clone()).map_err(|_| "来源状态锁已损坏".into()) }
    pub fn diagnostics(&self) -> Result<SourceDiagnostics, String> {
        let state = self.state.lock().map_err(|_| "来源状态锁已损坏")?;
        Ok(SourceDiagnostics { available: state.error.is_none() && state.source.is_some() && !self.cancelled.load(Ordering::Acquire), frozen: self.frozen.load(Ordering::Acquire), closing: self.cancelled.load(Ordering::Acquire), error: state.error.clone(), listener: state.listener, listener_error: state.listener_error.clone() })
    }
    pub async fn shutdown(&self, runtime: &tokio::runtime::Handle) -> Result<(), String> {
        self.stop();
        let mut worker = self.worker.lock().await;
        if let Some(worker) = worker.take() { worker.join(runtime).await?; }
        Ok(())
    }
}
fn set_listener_kind(state: &Mutex<SourceState>, listener: SourceListener) { if let Ok(mut state) = state.lock() { if listener == SourceListener::Polling && state.listener_error.is_none() { state.listener_error = state.error.clone(); } state.listener = listener; } }
fn set_listener(state: &Mutex<SourceState>, listener: SourceListener, error: Option<String>) { if let Ok(mut state) = state.lock() { state.listener = listener; state.listener_error = error; } }
fn receive_event<T>(events: &mpsc::Receiver<T>, app: &SourceContext, window: Option<&ActiveWindow>, icons: &HashMap<String, CachedIcon>) -> Result<T, RecvTimeoutError> {
    let retry = if app.frozen.load(Ordering::Acquire) { None } else { window.and_then(|window| icons.get(&format!("{}\0{}", window.name, window.path.as_deref().unwrap_or("")))).filter(|icon| icon.needs_retry).map(|icon| ICON_RETRY_DELAY.saturating_sub(icon.attempted_at.elapsed())) };
    let deadline = retry.map(mpsc::after).unwrap_or_else(mpsc::never);
    mpsc::select! {
        recv(events) -> event => event.map_err(|_| RecvTimeoutError::Disconnected),
        recv(app.control) -> _ => Err(RecvTimeoutError::Timeout),
        recv(deadline) -> _ => Err(RecvTimeoutError::Timeout),
    }
}

fn run_source(app: &SourceContext, state: &Arc<Mutex<SourceState>>, cancelled: &Arc<AtomicBool>, frozen: &Arc<AtomicBool>) {
    let mut icons = HashMap::new();
    let kde_wayland = cfg!(target_os = "linux") && clipboard_core::is_wayland_session()
        && std::env::var("KDE_SESSION_VERSION").as_deref() == Ok("6");
    let failed = if kde_wayland { track_kde_events(app, state, cancelled, frozen, &mut icons) }
    else { track_platform_events(app, state, cancelled, frozen, &mut icons) };
    if failed && !cancelled.load(Ordering::Acquire) {
        tracing::warn!("Active-window event listener unavailable; using query fallback");
        set_listener_kind(state, SourceListener::Polling);
        track_polling(app, state, cancelled, frozen, &mut icons, EVENT_FALLBACK_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_shutdown_keeps_pool_available_and_waits_for_resource_and_thread_exit() {
        struct Resource(Arc<AtomicBool>);
        impl Drop for Resource {
            fn drop(&mut self) { self.0.store(true, Ordering::Release); }
        }
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).max_blocking_threads(1)
            .enable_all().build().expect("isolated runtime");
        let handle = runtime.handle().clone();
        runtime.block_on(async move {
            tokio::time::timeout(Duration::from_secs(5), async move {
                let dropped = Arc::new(AtomicBool::new(false));
                let resource = Resource(dropped.clone());
                let (release, waiting) = std::sync::mpsc::channel();
                let (finished, work) = tokio::sync::oneshot::channel();
                let source = SourceWorker::spawn({ let handle = handle.clone(); move || {
                    let _captured = &resource;
                    waiting.recv_timeout(Duration::from_secs(2)).expect("release source icon work");
                    let value = handle.block_on(handle.spawn_blocking(|| 42)).expect("source icon work completed");
                    finished.send(value).expect("announce source work");
                } }).expect("spawn source worker");
                let mut stopping = Box::pin(source.join(&handle));
                assert!(futures_util::poll!(stopping.as_mut()).is_pending());
                assert!(!dropped.load(Ordering::Acquire), "source captures must remain alive during its work");
                assert_eq!(handle.spawn_blocking(|| 17).await.expect("shutdown leaves the only blocking thread available"), 17);
                release.send(()).expect("source can finish its runtime-dependent work");
                assert_eq!(work.await.expect("source used the shared pool"), 42);
                stopping.await.expect("source joined");
                assert!(dropped.load(Ordering::Acquire), "shutdown waits for captured resources to drop");

                let (completed, completion) = tokio::sync::oneshot::channel();
                let (announced, ready) = tokio::sync::oneshot::channel();
                let (release, exit) = std::sync::mpsc::channel();
                let thread = thread::spawn(move || {
                    completed.send(()).expect("announce completed source resources");
                    announced.send(()).expect("completion ready to observe");
                    exit.recv_timeout(Duration::from_secs(2)).expect("release final thread teardown");
                });
                ready.await.expect("completion announced");
                let source = SourceWorker { completed: completion, thread };
                let mut stopping = Box::pin(source.join(&handle));
                assert!(futures_util::poll!(stopping.as_mut()).is_pending(), "completion still requires the actual thread to exit");
                release.send(()).expect("thread can exit");
                stopping.await.expect("physical source thread joined");

                let dropped = Arc::new(AtomicBool::new(false));
                let resource = Resource(dropped.clone());
                let source = SourceWorker::spawn(move || {
                    let _captured = &resource;
                    panic!("source worker ownership regression");
                }).expect("spawn failing source worker");
                assert!(matches!(source.join(&handle).await, Err(error) if error == "来源追踪线程异常退出"));
                assert!(dropped.load(Ordering::Acquire), "panic also releases source resources before returning");
            }).await.expect("source work and shutdown complete within five seconds");
        });
    }
}
