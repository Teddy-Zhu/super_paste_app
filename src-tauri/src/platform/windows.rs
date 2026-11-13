use crate::{
    events::{SettingsProvider, emit_window},
    platform::active_window::SourceTracker,
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl};
use tokio::sync::Mutex as AsyncMutex;

const BLUR_CONFIRM_DELAY: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq)]
pub struct WorkArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

fn panel_height(area: &WorkArea) -> f64 {
    // Use logical work-area units so taskbars and per-monitor scaling are respected.
    (area.height * 0.36)
        .round()
        .clamp(260., 480.)
        .min(area.height)
}

fn panel_size(area: &WorkArea, position: &str) -> Result<LogicalSize<f64>, String> {
    let thickness = panel_height(area);
    match position {
        "top" | "bottom" => Ok(LogicalSize::new(area.width, thickness)),
        "left" | "right" => Ok(LogicalSize::new(thickness.min(area.width), area.height)),
        _ => Err("浮层位置无效".into()),
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Hidden,
    Preparing,
    Opening,
    Opened,
    Closing,
}
struct OverlayState {
    ready: bool,
    wants_visible: bool,
    transition: i64,
    phase: Phase,
    position: String,
    area: Option<WorkArea>,
    deferred_blur: bool,
    focus_revision: u64,
    monitor_signature: Option<Vec<String>>,
}

pub struct WindowManager {
    app: tauri::AppHandle,
    settings: SettingsProvider,
    source: Arc<SourceTracker>,
    state: Mutex<OverlayState>,
    opening_gate: AsyncMutex<()>,
    main_gate: AsyncMutex<()>,
}

impl WindowManager {
    pub fn new(
        app: tauri::AppHandle,
        settings: SettingsProvider,
        source: Arc<SourceTracker>,
    ) -> Arc<Self> {
        Arc::new(Self {
            app,
            settings,
            source,
            state: Mutex::new(OverlayState {
                ready: false,
                wants_visible: false,
                transition: 0,
                phase: Phase::Hidden,
                position: "bottom".into(),
                area: None,
                deferred_blur: false,
                focus_revision: 0,
                monitor_signature: None,
            }),
            opening_gate: AsyncMutex::new(()),
            main_gate: AsyncMutex::new(()),
        })
    }

    pub async fn create(self: &Arc<Self>, data_dir: &std::path::Path) -> Result<(), String> {
        let area = self.work_area().await?;
        let width = (area.width * 0.78)
            .round()
            .clamp(960., 1400.)
            .min(area.width);
        let height = (area.height * 0.82)
            .round()
            .clamp(640., 1000.)
            .min(area.height);
        self.create_host("main", width, height, false, data_dir)
            .await?;
        let main = self.window("main")?;
        main.set_position(LogicalPosition::new(
            area.x + (area.width - width) / 2.,
            area.y + (area.height - height) / 2.,
        ))
        .map_err(|e| e.to_string())?;
        let settings = (self.settings)().await?;
        let position = settings["overlayPosition"].as_str().ok_or("浮层位置无效")?;
        let size = panel_size(&area, position)?;
        self.create_host("overlay", size.width, size.height, true, data_dir)
            .await?;
        let weak = Arc::downgrade(self);
        self.window("overlay")?.on_window_event(move |event| {
            if let Some(manager) = weak.upgrade() {
                match event {
                    tauri::WindowEvent::Focused(focused) => {
                        let (id, revision) = match manager.state.lock() {
                            Ok(mut state) => {
                                state.focus_revision = state.focus_revision.wrapping_add(1);
                                if *focused {
                                    state.deferred_blur = false;
                                }
                                (state.transition, state.focus_revision)
                            }
                            Err(_) => {
                                tracing::error!("overlay state lock poisoned");
                                return;
                            }
                        };
                        if !*focused {
                            tauri::async_runtime::spawn(async move {
                                if let Err(error) = manager.blur(id, revision).await {
                                    tracing::error!(%error,"overlay blur failed");
                                }
                            });
                        }
                    }
                    tauri::WindowEvent::ScaleFactorChanged { .. } => {
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = manager.position_changed().await {
                                tracing::error!(%error,"overlay scale reconciliation failed");
                            }
                        });
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = manager.request_close(true).await {
                                tracing::error!(%error,"overlay close failed");
                            }
                        });
                    }
                    _ => {}
                }
            }
        });
        let weak = Arc::downgrade(self);
        self.window("main")?.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                // Native close callbacks must return before mutating the GTK host:
                // hiding it here can re-enter window event dispatch on Linux.
                if let Some(manager) = weak.upgrade() {
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = manager.hide_main().await {
                            tracing::error!(%error,"main hide failed");
                        }
                    });
                }
            }
        });
        Ok(())
    }

    async fn create_host(
        &self,
        label: &str,
        width: f64,
        height: f64,
        overlay: bool,
        data_dir: &std::path::Path,
    ) -> Result<(), String> {
        let mut builder = tauri::window::WindowBuilder::new(&self.app, label)
            .title(if overlay {
                "Super Paste"
            } else {
                "Super Paste Settings"
            })
            .inner_size(width, height)
            .visible(false)
            .focused(false)
            .decorations(false)
            .transparent(overlay)
            .resizable(!overlay)
            .maximizable(!overlay)
            .skip_taskbar(true)
            .always_on_top(overlay);
        if !overlay {
            builder = builder.min_inner_size(width.min(720.), height.min(520.));
        }
        let window = builder.build().map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        if overlay && clipboard_core::is_wayland_session() {
            let host = window.clone();
            on_main(&self.app, move || super::layer_shell::initialize(&host)).await?;
        }
        #[cfg(target_os = "linux")]
        if overlay {
            let host = window.clone();
            let app = self.app.clone();
            on_main(&self.app, move || {
                super::native_frame::configure_overlay(&host)?;
                super::modifiers::install(&app, &host)
            })
            .await?;
        }
        // The Wayland layer role is assigned before Wry can realize its GTK host.
        let route = if overlay {
            "index.html#/overlay"
        } else {
            "index.html"
        };
        let view = tauri::webview::WebviewBuilder::new(label, WebviewUrl::App(route.into()))
            .auto_resize()
            .transparent(overlay)
            .background_color(tauri::utils::config::Color(
                0,
                0,
                0,
                if overlay { 0 } else { 255 },
            ))
            .data_directory(data_dir.join("webview"))
            .on_navigation(local_navigation);
        #[cfg(target_os = "windows")]
        let view = view.additional_browser_args("--disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --disable-features=CalculateNativeWinOcclusion");
        let webview = window
            .add_child(
                view,
                LogicalPosition::new(0., 0.),
                LogicalSize::new(width, height),
            )
            .map_err(|e| e.to_string())?;
        #[cfg(all(debug_assertions, target_os = "linux"))]
        webview
            .with_webview(|native| {
                use webkit2gtk::{SettingsExt, WebViewExt};
                if let Some(settings) = native.inner().settings() {
                    settings.set_enable_write_console_messages_to_stdout(true);
                }
            })
            .map_err(|e| e.to_string())?;
        #[cfg(not(all(debug_assertions, target_os = "linux")))]
        let _ = webview;
        Ok(())
    }

    pub fn window(&self, label: &str) -> Result<tauri::Window, String> {
        self.app
            .get_window(label)
            .ok_or_else(|| format!("窗口不存在: {label}"))
    }

    pub async fn open_main(self: &Arc<Self>) -> Result<(), String> {
        let _main_gate = self.main_gate.lock().await;
        let _overlay_gate = self.opening_gate.lock().await;
        {
            let transition = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "overlay state lock poisoned")?;
                state.wants_visible = false;
                if state.phase == Phase::Hidden {
                    None
                } else {
                    state.transition += 1;
                    Some(state.transition)
                }
            };
            // Settings activation must not race a delayed overlay animation/focus.
            if let Some(id) = transition {
                self.hide_locked(id).await?;
            }
        }
        #[cfg(target_os = "macos")]
        let app = self.app.clone();
        let window = self.window("main")?;
        on_main(&self.app, move || {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular)
                .map_err(|e| e.to_string())?;
            #[cfg(target_os = "linux")]
            {
                let host = window.gtk_window().map_err(|e| e.to_string())?;
                present_linux_window(&host)
            }
            #[cfg(not(target_os = "linux"))]
            {
                window.unminimize().map_err(|e| e.to_string())?;
                window.show().map_err(|e| e.to_string())?;
                window.set_focus().map_err(|e| e.to_string())
            }
        })
        .await?;
        self.app
            .get_webview("main")
            .ok_or("设置 WebView 已关闭")?
            .set_focus()
            .map_err(|e| e.to_string())
    }

    pub async fn hide_main(&self) -> Result<(), String> {
        let _gate = self.main_gate.lock().await;
        #[cfg(target_os = "macos")]
        let app = self.app.clone();
        let window = self.window("main")?;
        on_main(&self.app, move || {
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::*;
                window.gtk_window().map_err(|e| e.to_string())?.hide();
            }
            #[cfg(not(target_os = "linux"))]
            window.hide().map_err(|e| e.to_string())?;
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory)
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    pub async fn toggle(self: &Arc<Self>, token: Option<String>) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        let (wants, id) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            // A shortcut activation supersedes any focus loss caused by its native key grab.
            state.focus_revision = state.focus_revision.wrapping_add(1);
            state.deferred_blur = false;
            (state.wants_visible, state.transition)
        };
        tracing::debug!(
            transition = id,
            action = if wants { "close" } else { "open" },
            "overlay toggle"
        );
        if wants {
            self.request_close_locked(true).await
        } else {
            self.show_locked(token).await
        }
    }

    pub async fn ready(self: &Arc<Self>) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        let (pending, restart) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            let restart = (state.ready && state.phase != Phase::Hidden).then_some(state.transition);
            state.ready = true;
            (state.wants_visible, restart)
        };
        tracing::debug!(
            pending,
            restarted = restart.is_some(),
            "overlay renderer ready"
        );
        // A reloaded renderer has reset its DOM and animation state. Preserve
        // visibility intent while invalidating the previous native transition.
        if let Some(id) = restart {
            self.hide_locked(id).await?;
        }
        if pending {
            self.show_locked(None).await?;
        }
        Ok(())
    }

    pub async fn show(self: &Arc<Self>, token: Option<String>) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        self.show_locked(token).await
    }

    async fn show_locked(self: &Arc<Self>, token: Option<String>) -> Result<(), String> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if !state.ready {
                state.wants_visible = true;
                return Ok(());
            }
            if [Phase::Preparing, Phase::Opening, Phase::Opened].contains(&state.phase) {
                return Ok(());
            }
        }
        let settings = (self.settings)().await?;
        let position = settings["overlayPosition"]
            .as_str()
            .ok_or("浮层位置无效")?
            .to_owned();
        if !["top", "bottom", "left", "right"].contains(&position.as_str()) {
            return Err("浮层位置无效".into());
        }
        let (area, monitor_signature) = self.work_area_and_signature().await?;
        self.place(&area, &position).await?;
        let id = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            state.transition += 1;
            state.wants_visible = true;
            state.phase = Phase::Preparing;
            state.position = position.clone();
            state.area = Some(area);
            state.monitor_signature = Some(monitor_signature);
            state.deferred_blur = false;
            state.transition
        };
        self.source.freeze(true);
        // Map the native surface invisibly so hidden-WebView rAF and native paints can complete.
        let result = async {
            super::native_frame::opacity(&self.app, "overlay", 0.).await?;
            #[cfg(target_os = "windows")]
            self.stage_offscreen()?;
            #[cfg(target_os = "linux")]
            {
                let window = self.window("overlay")?;
                on_main(&self.app, move || {
                    let host = window.gtk_window().map_err(|e| e.to_string())?;
                    prepare_linux_activation(&host, token.as_deref())?;
                    Ok(())
                })
                .await?;
            }
            #[cfg(not(target_os = "linux"))]
            let _ = token;
            self.window("overlay")?.show().map_err(|e| e.to_string())?;
            emit_window(
                &self.app,
                "overlay",
                "prepare-overlay-open",
                vec![json!(position), json!(id)],
            )?;
            self.timeout_hide(id, Duration::from_millis(1000), Phase::Preparing);
            Ok::<(), String>(())
        }
        .await;
        if result.is_err()
            && let Err(error) = self.hide_locked(id).await
        {
            tracing::error!(%error,"failed overlay preparation cleanup");
        }
        result
    }

    pub async fn prepared(self: &Arc<Self>, id: i64) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        if !self.is_current(id, Phase::Preparing)? {
            return Ok(());
        }
        if let Err(error) =
            super::native_frame::wait_frame(&self.app, "overlay", Duration::from_millis(250)).await
        {
            self.hide_locked(id).await?;
            return Err(error);
        }
        let (area, position) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if state.transition != id || !state.wants_visible || state.phase != Phase::Preparing {
                return Ok(());
            }
            state.phase = Phase::Opening;
            (
                state.area.clone().ok_or("浮层布局未准备")?,
                state.position.clone(),
            )
        };
        let result = async {
            self.place(&area, &position).await?;
            super::native_frame::opacity(&self.app, "overlay", 1.).await?;
            #[cfg(target_os = "linux")]
            {
                let window = self.window("overlay")?;
                on_main(&self.app, move || {
                    let host = window.gtk_window().map_err(|e| e.to_string())?;
                    present_linux_window(&host)
                })
                .await?;
            }
            #[cfg(not(target_os = "linux"))]
            self.window("overlay")?
                .set_focus()
                .map_err(|e| e.to_string())?;
            // The native host and child WebView have separate keyboard focus.
            self.app
                .get_webview("overlay")
                .ok_or("浮层 WebView 已关闭")?
                .set_focus()
                .map_err(|e| e.to_string())?;
            emit_window(
                &self.app,
                "overlay",
                "trigger-slide-in-animation",
                vec![json!(position), json!(id)],
            )?;
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if let Err(error) = manager.opened(id).await {
                    tracing::error!(%error,"overlay opening fallback failed");
                }
            });
            Ok::<(), String>(())
        }
        .await;
        if result.is_err()
            && let Err(error) = self.hide_locked(id).await
        {
            tracing::error!(%error,"failed overlay opening cleanup");
        }
        result
    }

    pub async fn opened(self: &Arc<Self>, id: i64) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        let blur = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if state.transition != id || state.phase != Phase::Opening {
                return Ok(());
            }
            state.phase = Phase::Opened;
            std::mem::take(&mut state.deferred_blur)
        };
        if blur
            && !self
                .window("overlay")?
                .is_focused()
                .map_err(|e| e.to_string())?
        {
            self.request_close_locked(true).await?;
        }
        Ok(())
    }

    pub async fn request_close(self: &Arc<Self>, force: bool) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        self.request_close_locked(force).await
    }

    async fn request_close_locked(self: &Arc<Self>, force: bool) -> Result<(), String> {
        let id = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if state.phase == Phase::Hidden {
                state.wants_visible = false;
                return Ok(());
            }
            if state.phase == Phase::Closing && !state.wants_visible {
                return Ok(());
            }
            if force {
                state.wants_visible = false;
                state.transition += 1;
                state.phase = Phase::Closing;
            }
            state.transition
        };
        if force {
            self.timeout_hide(id, Duration::from_millis(250), Phase::Closing);
        }
        if let Err(error) = emit_window(
            &self.app,
            "overlay",
            "trigger-close-window",
            vec![json!(force), json!(id)],
        ) {
            self.hide_locked(id).await?;
            return Err(error);
        }
        Ok(())
    }

    pub async fn close_started(self: &Arc<Self>, id: i64) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if state.transition != id || state.phase == Phase::Hidden {
                return Ok(());
            }
            state.wants_visible = false;
            state.phase = Phase::Closing;
        }
        self.timeout_hide(id, Duration::from_millis(250), Phase::Closing);
        Ok(())
    }

    pub async fn close_ack(self: &Arc<Self>, id: i64) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        if !self.is_current(id, Phase::Closing)? {
            return Ok(());
        }
        if let Err(error) =
            super::native_frame::wait_frame(&self.app, "overlay", Duration::from_millis(80)).await
        {
            tracing::debug!(%error,"closing frame barrier expired; hiding native surface");
        }
        self.hide_locked(id).await
    }

    async fn blur(self: &Arc<Self>, id: i64, revision: u64) -> Result<(), String> {
        // Match Electron's dev behavior so inspecting another window does not
        // consume the close half of an explicit shortcut or test-button toggle.
        if cfg!(debug_assertions) {
            tracing::debug!(transition = id, "overlay blur ignored in development");
            return Ok(());
        }
        // The X11 shortcut backend polls every 50 ms. Allow its activation and
        // GTK focus-in to cancel this request before confirming an outside click.
        tokio::time::sleep(BLUR_CONFIRM_DELAY).await;
        let _gate = self.opening_gate.lock().await;
        if self
            .window("overlay")?
            .is_focused()
            .map_err(|e| e.to_string())?
        {
            return Ok(());
        }
        let should_close = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            // Focus callbacks are dispatched asynchronously and may belong to a
            // previous hide/show cycle by the time the transition gate is free.
            if state.transition != id || state.focus_revision != revision || !state.wants_visible {
                return Ok(());
            }
            if [Phase::Preparing, Phase::Opening].contains(&state.phase) {
                state.deferred_blur = true;
                false
            } else {
                state.phase == Phase::Opened
            }
        };
        if should_close {
            self.request_close_locked(true).await?;
        }
        Ok(())
    }

    async fn hide_if_phase(self: &Arc<Self>, id: i64, phase: Phase) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        if self.is_current(id, phase)? {
            self.hide_locked(id).await
        } else {
            Ok(())
        }
    }

    async fn hide_locked(self: &Arc<Self>, id: i64) -> Result<(), String> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if state.transition != id {
                return Ok(());
            }
            state.phase = Phase::Hidden;
            state.wants_visible = false;
            state.deferred_blur = false;
        }
        // Clear native opacity before unmapping so the compositor cannot animate
        // a stale visible strip after the renderer has completed its slide-out.
        let opacity = super::native_frame::opacity(&self.app, "overlay", 0.).await;
        let hidden = self
            .window("overlay")
            .and_then(|window| window.hide().map_err(|e| e.to_string()));
        self.source.freeze(false);
        let notified = emit_window(&self.app, "overlay", "overlay-hidden", vec![json!(id)]);
        opacity.and(hidden).and(notified)
    }

    fn is_current(&self, id: i64, phase: Phase) -> Result<bool, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "overlay state lock poisoned")?;
        Ok(state.transition == id && state.phase == phase)
    }
    fn timeout_hide(self: &Arc<Self>, id: i64, duration: Duration, phase: Phase) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(duration).await;
            if let Err(error) = manager.hide_if_phase(id, phase).await {
                tracing::error!(%error,"overlay transition timeout cleanup failed");
            }
        });
    }

    pub async fn position_changed(self: &Arc<Self>) -> Result<(), String> {
        let _gate = self.opening_gate.lock().await;
        let (visible, id) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            (state.wants_visible, state.transition)
        };
        if visible {
            self.hide_locked(id).await?;
            self.show_locked(None).await?;
        }
        let settings = (self.settings)().await?;
        emit_window(
            &self.app,
            "overlay",
            "overlay-position-updated",
            vec![settings["overlayPosition"].clone()],
        )
    }

    pub async fn reconcile_monitors(self: &Arc<Self>) -> Result<(), String> {
        let id = {
            let state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if !state.wants_visible || state.phase != Phase::Opened {
                return Ok(());
            }
            state.transition
        };
        let signature = self.monitor_signature().await?;
        let changed = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "overlay state lock poisoned")?;
            if state.transition != id || !state.wants_visible {
                return Ok(());
            }
            let changed = state.monitor_signature.as_ref() != Some(&signature);
            state.monitor_signature = Some(signature);
            changed
        };
        if changed {
            self.position_changed().await?;
        }
        Ok(())
    }

    async fn monitor_signature(&self) -> Result<Vec<String>, String> {
        #[cfg(target_os = "linux")]
        if clipboard_core::is_wayland_session() {
            return tokio::task::spawn_blocking(|| {
                let mut values = super::kwin::work_areas()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|area| {
                        json!([area.display_id, area.x, area.y, area.width, area.height])
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                values.sort();
                Ok(values)
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        let mut values = self
            .app
            .available_monitors()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|monitor| {
                let area = monitor.work_area();
                json!([
                    monitor.name(),
                    monitor.position().x,
                    monitor.position().y,
                    monitor.size().width,
                    monitor.size().height,
                    monitor.scale_factor(),
                    area.position.x,
                    area.position.y,
                    area.size.width,
                    area.size.height
                ])
                .to_string()
            })
            .collect::<Vec<_>>();
        values.sort();
        Ok(values)
    }

    async fn work_area_and_signature(&self) -> Result<(WorkArea, Vec<String>), String> {
        #[cfg(target_os = "linux")]
        if clipboard_core::is_wayland_session() {
            return tokio::task::spawn_blocking(|| {
                let snapshot = super::kwin::work_area_snapshot().map_err(|e| e.to_string())?;
                let area = snapshot.cursor_area;
                let mut signature = snapshot
                    .areas
                    .into_iter()
                    .map(|area| {
                        json!([area.display_id, area.x, area.y, area.width, area.height])
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                signature.sort();
                Ok((
                    WorkArea {
                        x: area.x,
                        y: area.y,
                        width: area.width,
                        height: area.height,
                    },
                    signature,
                ))
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        Ok((self.work_area().await?, self.monitor_signature().await?))
    }

    async fn place(&self, area: &WorkArea, position: &str) -> Result<(), String> {
        let size = panel_size(area, position)?;
        let (x, y) = match position {
            "top" | "left" => (area.x, area.y),
            "bottom" => (area.x, area.y + area.height - size.height),
            "right" => (area.x + area.width - size.width, area.y),
            _ => return Err("浮层位置无效".into()),
        };
        let window = self.window("overlay")?;
        #[cfg(target_os = "linux")]
        {
            let area = area.clone();
            let position = position.to_owned();
            return on_main(&self.app, move || {
                use gtk::prelude::*;
                let host = window.gtk_window().map_err(|e| e.to_string())?;
                let width = size.width.round() as i32;
                let height = size.height.round() as i32;
                // A non-resizable GTK window retains its old natural/default size.
                // Update both requests when switching between a horizontal and vertical strip.
                host.set_size_request(width, height);
                host.set_default_size(width, height);
                let wayland = clipboard_core::is_wayland_session();
                if wayland {
                    super::layer_shell::place(&window, &area, &position)?;
                }
                host.resize(width, height);
                if !wayland {
                    host.move_(x.round() as i32, y.round() as i32);
                }
                Ok(())
            })
            .await;
        }
        #[cfg(not(target_os = "linux"))]
        {
            window.set_size(size).map_err(|e| e.to_string())?;
            window
                .set_position(LogicalPosition::new(x, y))
                .map_err(|e| e.to_string())
        }
    }

    pub async fn work_area(&self) -> Result<WorkArea, String> {
        #[cfg(target_os = "linux")]
        if clipboard_core::is_wayland_session() {
            return tokio::task::spawn_blocking(|| {
                super::kwin::work_area(None)
                    .map(|a| WorkArea {
                        x: a.x,
                        y: a.y,
                        width: a.width,
                        height: a.height,
                    })
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        let window = self.app.get_window("main");
        let cursor = self.app.cursor_position().map_err(|e| e.to_string())?;
        let monitors = self.app.available_monitors().map_err(|e| e.to_string())?;
        let monitor = monitors
            .into_iter()
            .find(|monitor| {
                let p = monitor.position();
                let s = monitor.size();
                cursor.x >= p.x as f64
                    && cursor.y >= p.y as f64
                    && cursor.x < (p.x as f64 + s.width as f64)
                    && cursor.y < (p.y as f64 + s.height as f64)
            })
            .or_else(|| window.and_then(|w| w.current_monitor().ok().flatten()))
            .ok_or("当前显示器不可用")?;
        let rect = monitor.work_area();
        let scale = monitor.scale_factor();
        Ok(WorkArea {
            x: rect.position.x as f64 / scale,
            y: rect.position.y as f64 / scale,
            width: rect.size.width as f64 / scale,
            height: rect.size.height as f64 / scale,
        })
    }

    #[cfg(target_os = "windows")]
    fn stage_offscreen(&self) -> Result<(), String> {
        let monitors = self.app.available_monitors().map_err(|e| e.to_string())?;
        let right = monitors
            .iter()
            .map(|m| m.position().x.saturating_add(m.size().width as i32))
            .max()
            .ok_or("显示器不可用")?;
        self.window("overlay")?
            .set_position(tauri::PhysicalPosition::new(right.saturating_add(4096), 0))
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "linux")]
fn prepare_linux_activation(
    host: &gtk::ApplicationWindow,
    token: Option<&str>,
) -> Result<u32, String> {
    use gtk::prelude::*;

    // Initially unfocused Tao windows defer accepting focus until their first draw.
    host.set_accept_focus(true);
    host.set_focus_on_map(true);
    host.realize();
    let surface = host.window().ok_or("原生窗口尚未创建")?;
    let timestamp = if let Some(x11_surface) = surface.downcast_ref::<gdkx11::X11Window>() {
        // Tray and shortcut callbacks run outside GTK input dispatch. Refresh the
        // X11 user time so the window manager accepts this explicit activation.
        // The server-time query requires property events to avoid hanging.
        surface.set_events(surface.events() | gdk::EventMask::PROPERTY_CHANGE_MASK);
        let timestamp = gdkx11::functions::x11_get_server_time(x11_surface);
        x11_surface.set_user_time(timestamp);
        timestamp
    } else {
        gtk::current_event_time()
    };
    if let Some(token) = token {
        host.set_startup_id(token);
    }
    Ok(timestamp)
}

#[cfg(target_os = "linux")]
fn present_linux_window(host: &gtk::ApplicationWindow) -> Result<(), String> {
    use gtk::prelude::*;

    let timestamp = prepare_linux_activation(host, None)?;
    host.deiconify();
    // Include the child WebView's initially hidden parent containers.
    host.show_all();
    host.present_with_time(timestamp);
    Ok(())
}

fn local_navigation(url: &url::Url) -> bool {
    (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (["http", "https"].contains(&url.scheme()) && url.host_str() == Some("tauri.localhost"))
        || (cfg!(debug_assertions)
            && url.scheme() == "http"
            && [Some("localhost"), Some("127.0.0.1")].contains(&url.host_str())
            && url.port() == Some(1420))
}

pub async fn on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    run: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(run());
    })
    .map_err(|e| e.to_string())?;
    receiver.await.map_err(|_| "主线程操作已取消")?
}
