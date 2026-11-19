use std::{collections::BTreeMap, rc::Rc, sync::Arc};
use gpui_kit::*;
use gpui_kit::component::{WindowExt, dialog::DialogButtonProps};
use super_paste_core::{Error, Result};
use super_paste_platform::geometry::WorkAreaSnapshot;
use super_paste_platform::instance::{Activation, LaunchRequest};
use super_paste_platform::tray::TrayAction;
use super_paste_format::{ConfigChange, ConfigField};
use uuid::Uuid;
use tokio::{runtime::Handle, sync::{mpsc, watch}};
use crate::{services::Services, ui::{self, settings::SettingsView}};

actions!(super_paste, [ShowSettings, ShowOverlay, ToggleOverlay, Quit]);

pub struct DesktopGlobal(pub Entity<Desktop>);
impl Global for DesktopGlobal {}

pub struct Desktop {
    runtime: Handle,
    services: Option<Arc<Services>>,
    settings: Option<AnyWindowHandle>,
    opening_settings: bool,
    settings_activation_token: Option<String>,
    overlay: Option<OverlayWindow>,
    opening_overlay: bool,
    overlay_query_running: bool,
    overlay_activation_token: Option<String>,
    overlay_source_freeze: Option<super_paste_platform::access::SourceFreeze>,
    overlay_request: u64,
    quitting: bool,
    error: Option<AnyWindowHandle>,
    restart: Arc<super_paste_platform::restart::Restart>,
    restarting: bool,
    restart_revision: u64,
    restart_prompted: u64,
    restart_prompt_open: bool,
    sync_watch: Option<Task<()>>,
    activation_watch: Option<Task<()>>,
    tray_client: Uuid,
    tray_client_lifetime: Option<Rc<()>>,
    tray_pending: BTreeMap<ConfigField, Uuid>,
    tray_subscriptions: Vec<Subscription>,
    _power_subscriptions: Vec<Subscription>,
    pending_power: Option<bool>,
    pending_error: Option<(&'static str, Error)>,
}
struct OverlayWindow { window: AnyWindowHandle, view: Entity<ui::overlay::OverlayView>, work: ui::geometry::WorkArea, position: super_paste_format::OverlayPosition }

pub fn install(runtime: Handle, boot: watch::Receiver<Option<Result<Arc<Services>>>>, restart: Arc<super_paste_platform::restart::Restart>, launch: LaunchRequest, activations: mpsc::Receiver<Activation>, cx: &mut App) {
    let quit_boot = boot.clone();
    let desktop = cx.new(|cx| Desktop::new(runtime, boot, restart, launch, activations, cx));
    cx.set_global(DesktopGlobal(desktop));
    cx.on_action(|_: &ShowSettings, cx| {
        let desktop = cx.global::<DesktopGlobal>().0.clone();
        desktop.update(cx, |desktop, cx| desktop.show_settings(cx));
    });
    cx.on_action(|_: &Quit, cx| {
        let desktop = cx.global::<DesktopGlobal>().0.clone();
        desktop.update(cx, |desktop, cx| desktop.quit(cx));
    });
    cx.on_action(|_: &ShowOverlay, cx| {
        let desktop = cx.global::<DesktopGlobal>().0.clone(); desktop.update(cx, |desktop, cx| desktop.show_overlay(None, cx));
    });
    cx.on_action(|_: &ToggleOverlay, cx| {
        let desktop = cx.global::<DesktopGlobal>().0.clone(); desktop.update(cx, |desktop, cx| desktop.toggle_overlay(None, cx));
    });
    cx.bind_keys([
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-," } else { "ctrl-," }, ShowSettings, None),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-q" } else { "ctrl-q" }, Quit, None),
    ]);
    cx.on_app_quit(move |cx| {
        let desktop = cx.global::<DesktopGlobal>().0.clone(); desktop.update(cx, |desktop, cx| { desktop.quitting = true; desktop.stop_overlay(cx); });
        let mut boot = quit_boot.clone();
        async move {
            if let Ok(services) = wait_startup(&mut boot).await {
                if let Err(error) = services.shutdown().await { tracing::error!(%error, "退出清理失败"); }
            }
        }
    }).detach();
}

impl Desktop {
    fn new(runtime: Handle, mut boot: watch::Receiver<Option<Result<Arc<Services>>>>, restart: Arc<super_paste_platform::restart::Restart>, launch: LaunchRequest, mut activations: mpsc::Receiver<Activation>, cx: &mut Context<Self>) -> Self {
        let activation_watch = cx.spawn(async move |weak, cx| {
            let result = wait_startup(&mut boot).await;
            let _ = weak.update(cx, |desktop, cx| match result {
                Ok(services) => {
                    let silent = services.configuration.snapshot().snapshot.value.settings.silent_start;
                    let configuration = cx.new(|cx| ui::configuration::ConfigurationModel::new(services.clone(), cx));
                    cx.set_global(ui::configuration::ConfigurationGlobal(configuration.clone()));
                    desktop.install_tray(&configuration, &services, cx);
                    let mut sync = services.sync.subscribe();
                    desktop.restart_revision = sync.borrow_and_update().restart_revision;
                    desktop.sync_watch = Some(cx.spawn(async move |weak, cx| {
                        while sync.changed().await.is_ok() {
                            let revision = sync.borrow_and_update().restart_revision;
                            if weak.update(cx, |desktop, cx| { desktop.restart_revision = revision; desktop.prompt_restart(cx); }).is_err() { break; }
                        }
                    }));
                    desktop.services = Some(services);
                    if let Some(sleeping) = desktop.pending_power.take() { desktop.power_changed(sleeping); }
                    if !desktop.quitting { match launch {
                        LaunchRequest::Default if !silent => desktop.show_settings(cx),
                        LaunchRequest::ToggleOverlay { activation_token } => desktop.show_overlay(activation_token, cx),
                        _ => {},
                    } }
                }
                Err(error) => desktop.show_error("启动失败", error, cx),
            });
            while let Some(activation) = activations.recv().await {
                if weak.update(cx, |desktop, cx| match activation {
                    Activation::ShowSettings => desktop.show_settings(cx),
                    Activation::ToggleOverlay { activation_token } => desktop.toggle_overlay(activation_token, cx),
                    Activation::Tray(action) => desktop.tray_action(action, cx),
                }).is_err() { break; }
            }
        });
        let sleeping = cx.entity().downgrade();
        let waking = sleeping.clone();
        let power_subscriptions = vec![
            cx.on_system_sleep(move |cx| { let _ = sleeping.update(cx, |desktop, _| desktop.power_changed(true)); }),
            cx.on_system_wake(move |cx| { let _ = waking.update(cx, |desktop, _| desktop.power_changed(false)); }),
        ];
        Self { runtime, services: None, settings: None, opening_settings: false, settings_activation_token: None, overlay: None, opening_overlay: false, overlay_query_running: false, overlay_activation_token: None, overlay_source_freeze: None, overlay_request: 0, quitting: false, error: None, restart, restarting: false,
            restart_revision: 0, restart_prompted: 0, restart_prompt_open: false, sync_watch: None, activation_watch: Some(activation_watch), pending_error: None,
            tray_client: Uuid::new_v4(), tray_client_lifetime: None, tray_pending: BTreeMap::new(), tray_subscriptions: Vec::new(), _power_subscriptions: power_subscriptions, pending_power: None }
    }

    fn power_changed(&mut self, sleeping: bool) {
        if self.quitting { return; }
        match &self.services {
            Some(services) => if let Err(error) = services.power_event(sleeping) { tracing::error!(%error, "系统电源状态接入失败"); },
            None => self.pending_power = Some(sleeping),
        }
    }

    pub fn show_settings(&mut self, cx: &mut Context<Self>) {
        self.show_settings_with_token(None, cx);
    }
    fn is_running(&self) -> bool {
        !self.quitting && self.services.as_ref().is_some_and(|services| !services.is_shutting_down())
    }
    fn show_settings_with_token(&mut self, token: Option<String>, cx: &mut Context<Self>) {
        if !self.is_running() { return; }
        self.settings_activation_token = token;
        self.close_overlay(cx);
        if let Some(settings) = self.settings {
            let token = self.settings_activation_token.as_deref();
            match settings.update(cx, |_, window, cx| ui::settings::show_window(window, token, cx)) {
                Ok(Ok(())) => { self.settings_activation_token = None; self.prompt_restart(cx); return; },
                Ok(Err(error)) => { self.settings_activation_token = None; self.show_error("无法显示设置", Error::Platform(error.to_string().into()), cx); return; }
                Err(_) => self.settings = None,
            }
        }
        if self.opening_settings || self.services.is_none() { return; }
        self.opening_settings = true;
        let runtime = self.runtime.clone();
        let query_runtime = runtime.clone();
        let query = runtime.spawn(async move { super_paste_platform::geometry::work_area_snapshot(&query_runtime).await });
        cx.spawn(async move |weak, cx| {
            let result = query.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update(cx, |desktop, cx| {
                desktop.opening_settings = false;
                if !desktop.is_running() { desktop.settings_activation_token = None; return; }
                match result.and_then(|snapshot| desktop.open_settings(snapshot, cx)) {
                    Ok(()) => {}
                    Err(error) => { desktop.settings_activation_token = None; desktop.show_error("无法打开设置", error, cx); }
                }
            });
        }).detach();
    }

    fn open_settings(&mut self, snapshot: Option<WorkAreaSnapshot>, cx: &mut Context<Self>) -> Result<()> {
        let token = self.settings_activation_token.take();
        let Some(services) = self.services.clone() else { return Err(Error::Cancelled); };
        let work = ui::geometry::work_area(snapshot.as_ref(), cx)?;
        let options = ui::geometry::settings_options(work)?;
        let (window, _) = gpui_kit::open_window(options, cx, move |window, cx| {
            window.set_window_title("Super Paste");
            window.on_window_should_close(cx, |window, cx| {
                if let Err(error) = ui::settings::hide_window(window, cx) { tracing::error!(%error, "隐藏设置窗口失败"); }
                false
            });
            cx.new(|cx| SettingsView::new(services, window, cx))
        }).map_err(|error| Error::Platform(error.to_string().into()))?;
        self.settings = Some(window);
        window.update(cx, |_, window, cx| ui::settings::show_window(window, token.as_deref(), cx))
            .map_err(|error| Error::Platform(error.to_string().into()))?
            .map_err(|error| Error::Platform(error.to_string().into()))?;
        if let Some((title, error)) = self.pending_error.take() { self.show_error(title, error, cx); }
        self.prompt_restart(cx);
        Ok(())
    }

    fn install_tray(&mut self, configuration: &Entity<ui::configuration::ConfigurationModel>, services: &Arc<Services>, cx: &mut Context<Self>) {
        let client = self.tray_client;
        self.tray_client_lifetime = Some(configuration.update(cx, |model, _| model.register_inline_client(client)));
        let observed_services = services.clone();
        self.tray_subscriptions.push(cx.observe(configuration, move |desktop, model, cx| {
            let model = model.read(cx);
            let pending = desktop.tray_pending.len();
            // A newer write from settings can replace the tray mutation and
            // consume its completion. Settle that field from the model, too.
            desktop.tray_pending.retain(|field, _| model.is_pending(*field));
            let settled = desktop.tray_pending.len() != pending;
            if let Err(error) = observed_services.update_tray(&model.projected.settings, settled) {
                if !observed_services.is_shutting_down() { tracing::warn!(%error, "更新托盘状态失败"); }
            }
        }));
        self.tray_subscriptions.push(cx.subscribe(configuration, |desktop, model, feedback: &ui::configuration::Feedback, cx| {
            if feedback.client_id != desktop.tray_client { return; }
            if !desktop.tray_pending.values().any(|mutation| *mutation == feedback.mutation_id) { return; }
            desktop.tray_pending.retain(|_, mutation| *mutation != feedback.mutation_id);
            if let Some(services) = &desktop.services {
                if let Err(error) = services.update_tray(&model.read(cx).projected.settings, true) {
                    if !services.is_shutting_down() { tracing::warn!(%error, "恢复托盘状态失败"); }
                }
            }
        }));
        if let Err(error) = services.update_tray(&configuration.read(cx).projected.settings, true) { tracing::warn!(%error, "同步初始托盘状态失败"); }
    }

    fn tray_action(&mut self, action: TrayAction, cx: &mut Context<Self>) {
        if self.quitting { return; }
        // Exit must remain available after a failed shutdown; the other tray
        // actions require live services.
        if matches!(action, TrayAction::Quit) { self.quit(cx); return; }
        if !self.is_running() { return; }
        let change = match action {
            TrayAction::ShowSettings { activation_token } => { self.show_settings_with_token(activation_token, cx); return; },
            TrayAction::ToggleOverlay { activation_token } => { self.toggle_overlay(activation_token, cx); return; },
            TrayAction::Restart => { self.restart(cx); return; },
            action => {
                let configuration = cx.global::<ui::configuration::ConfigurationGlobal>().0.read(cx);
                let settings = &configuration.projected.settings;
                match action {
                    TrayAction::SetColorMode(mode) => ConfigChange::ColorMode(mode),
                    TrayAction::ToggleAutoStart => ConfigChange::AutoStart(!settings.auto_start),
                    TrayAction::ToggleLocalOcr => ConfigChange::EnableLocalOcr(!settings.enable_local_ocr),
                    _ => unreachable!("window and lifecycle actions returned above"),
                }
            }
        };
        let field = change.field();
        let configuration = cx.global::<ui::configuration::ConfigurationGlobal>().0.clone();
        match configuration.update(cx, |model, cx| model.submit(self.tray_client, vec![change], Vec::new(), None, cx)) {
            Ok(mutation) => { self.tray_pending.insert(field, mutation); },
            Err(error) => tracing::warn!(%error, "托盘设置提交失败"),
        }
        if let Some(services) = &self.services {
            if let Err(error) = services.update_tray(&configuration.read(cx).projected.settings, true) { tracing::warn!(%error, "刷新托盘状态失败"); }
        }
    }

    pub fn toggle_overlay(&mut self, token: Option<String>, cx: &mut Context<Self>) {
        if !self.is_running() { return; }
        if self.opening_overlay || self.overlay.as_ref().is_some_and(|overlay| overlay.view.read(cx).is_open()) { self.close_overlay(cx); }
        else { self.show_overlay(token, cx); }
    }
    pub fn show_overlay(&mut self, token: Option<String>, cx: &mut Context<Self>) {
        if !self.is_running() { return; }
        let Some(services) = self.services.clone() else { return; };
        self.overlay_request = self.overlay_request.wrapping_add(1);
        self.opening_overlay = true;
        self.overlay_activation_token = token;
        match services.access.freeze_source() {
            Ok(freeze) => self.overlay_source_freeze = Some(freeze),
            Err(error) => { self.opening_overlay = false; self.overlay_activation_token = None; self.overlay_source_freeze = None; self.show_error("无法打开覆盖层", error, cx); return; },
        }
        self.query_overlay_geometry(cx);
    }
    fn query_overlay_geometry(&mut self, cx: &mut Context<Self>) {
        if self.overlay_query_running || !self.opening_overlay || !self.is_running() { return; }
        self.overlay_query_running = true;
        let request = self.overlay_request;
        let runtime = self.runtime.clone(); let query_runtime = runtime.clone();
        let query = runtime.spawn(async move { super_paste_platform::geometry::work_area_snapshot(&query_runtime).await });
        cx.spawn(async move |weak, cx| {
            let result = query.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update(cx, |desktop, cx| {
                desktop.overlay_query_running = false;
                if !desktop.is_running() { return; }
                if desktop.overlay_request != request {
                    // A blocking native query keeps running after cancellation.
                    // Wait for it to finish, then query only the latest live
                    // request; its cursor/display snapshot must be fresh.
                    desktop.query_overlay_geometry(cx);
                    return;
                }
                desktop.opening_overlay = false;
                let token = desktop.overlay_activation_token.take();
                // Keep this activation's lease through window replacement and
                // the new view acquiring its own lease, without an unfrozen gap.
                let _source_freeze = desktop.overlay_source_freeze.take();
                if let Err(error) = result.and_then(|snapshot| desktop.open_overlay(snapshot, token, cx)) {
                    desktop.show_error("无法打开覆盖层", error, cx);
                }
            });
        }).detach();
    }
    fn open_overlay(&mut self, snapshot: Option<WorkAreaSnapshot>, token: Option<String>, cx: &mut Context<Self>) -> Result<()> {
        let Some(services) = self.services.clone() else { return Err(Error::Cancelled); };
        let work = ui::geometry::work_area(snapshot.as_ref(), cx)?;
        let position = services.configuration.snapshot().snapshot.value.settings.overlay_position;
        if let Some(overlay) = &self.overlay {
            if overlay.work == work && overlay.position == position {
                let view = overlay.view.clone();
                overlay.window.update(cx, |_, window, cx| view.update(cx, |view, cx| view.open(token, window, cx)))
                    .map_err(|error| Error::Platform(error.to_string().into()))?;
                return Ok(());
            }
        }
        let (saved, reader) = if let Some(previous) = self.overlay.take() {
            let saved = previous.view.read(cx).saved(cx); let reader = previous.view.read(cx).reader.clone(); let view = previous.view;
            previous.window.update(cx, |_, window, cx| { view.update(cx, |view, cx| view.retire(window, cx)); window.remove_window(); })
                .map_err(|error| Error::Platform(error.to_string().into()))?;
            (saved, reader)
        } else { (ui::overlay::SavedState::default(), Arc::new(ui::overlay::HistoryReader::new(services.storage.clone(), &self.runtime))) };
        let wayland = cx.compositor_name() == "Wayland";
        let options = ui::geometry::overlay_options(work, position, wayland)?;
        let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
            window.set_window_title("Super Paste");
            let view = cx.new(|cx| ui::overlay::OverlayView::new(services, reader, position, work, wayland, saved, token, window, cx));
            let weak = view.downgrade();
            window.on_window_should_close(cx, move |window, cx| { let _ = weak.update(cx, |view, cx| view.close(window, cx)); false });
            view
        }).map_err(|error| Error::Platform(error.to_string().into()))?;
        window.update(cx, |_, window, cx| {
            // The animated panel owns its background; keep the root behind it transparent.
            gpui_kit::base::Root::update(window, cx, |root, _, cx| {
                root.style().background = Some(transparent_black().into());
                cx.notify();
            });
        }).map_err(|error| Error::Platform(error.to_string().into()))?;
        self.overlay = Some(OverlayWindow { window, view, work, position });
        Ok(())
    }
    fn close_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay_request = self.overlay_request.wrapping_add(1);
        self.opening_overlay = false; self.overlay_activation_token = None; self.overlay_source_freeze = None;
        if let Some(overlay) = &self.overlay {
            let view = overlay.view.clone();
            if let Err(error) = overlay.window.update(cx, |_, window, cx| view.update(cx, |view, cx| view.close(window, cx))) { tracing::error!(%error, "关闭覆盖层失败"); }
        }
    }
    fn stop_overlay(&mut self, cx: &mut Context<Self>) {
        self.activation_watch = None;
        self.overlay_request = self.overlay_request.wrapping_add(1); self.opening_overlay = false; self.overlay_activation_token = None; self.overlay_source_freeze = None;
        if let Some(overlay) = &self.overlay {
            let view = overlay.view.clone();
            if let Err(error) = overlay.window.update(cx, |_, window, cx| view.update(cx, |view, cx| view.retire(window, cx))) { tracing::error!(%error, "回收覆盖层失败"); view.read(cx).reader.suspend(); }
        }
    }

    pub fn quit(&mut self, cx: &mut Context<Self>) {
        if self.quitting { return; }
        self.quitting = true;
        self.stop_overlay(cx);
        let Some(services) = self.services.clone() else { cx.quit(); return; };
        cx.spawn(async move |_, cx| {
            if let Err(error) = services.shutdown().await { tracing::error!(%error, "退出清理失败"); }
            let _ = cx.update(|cx| cx.quit());
        }).detach();
    }

    fn prompt_restart(&mut self, cx: &mut Context<Self>) {
        if !self.is_running() || self.restart_revision <= self.restart_prompted { return; }
        if self.restart_prompt_open { self.restart_prompted = self.restart_revision; return; }
        let Some(settings) = self.settings else { return; };
        let weak = cx.entity().downgrade(); let closed = weak.clone();
        let shown = settings.update(cx, |_, window, cx| {
            if !window.is_visible() { return false; }
            window.open_alert_dialog(cx, move |dialog, _, _| {
                let weak = weak.clone(); let closed = closed.clone();
                dialog.title("同步完成").child("配置已下载。为了应用这些更改，需要重启应用。")
                    .child("已下载的文件：config.json")
                    .button_props(DialogButtonProps::default().ok_text("立即重启").cancel_text("稍后重启").show_cancel(true)
                        .on_ok(move |_, _, cx| { let _ = weak.update(cx, |desktop, cx| desktop.restart(cx)); true }))
                    .on_close(move |_, _, cx| { let _ = closed.update(cx, |desktop, _| desktop.restart_prompt_open = false); })
            });
            true
        });
        if matches!(shown, Ok(true)) { self.restart_prompt_open = true; self.restart_prompted = self.restart_revision; }
    }
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if !self.is_running() || self.restarting { return; }
        let Some(services) = self.services.clone() else { return; };
        self.restarting = true;
        let restart = self.restart.clone(); let prepare_restart = restart.clone(); let runtime = self.runtime.clone();
        let prepare = runtime.spawn(async move { prepare_restart.prepare().await });
        cx.spawn(async move |weak, cx| {
            let prepared = prepare.await.map_err(Error::from).and_then(|result| result);
            let prepared = weak.update(cx, |desktop, cx| {
                desktop.restarting = false;
                if desktop.quitting { return None; }
                match prepared {
                    Ok(prepared) => { desktop.quitting = true; desktop.stop_overlay(cx); Some(prepared) },
                    Err(error) => { desktop.restart_failed(error, false, cx); None },
                }
            });
            let Ok(Some(prepared)) = prepared else { return; };
            let shutdown = runtime.spawn(async move { services.shutdown().await?; restart.authorize(prepared).await });
            let result = shutdown.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update(cx, |desktop, cx| match result {
                Ok(()) => cx.quit(),
                Err(error) => { desktop.quitting = false; desktop.restart_failed(error, true, cx); },
            });
        }).detach();
    }
    fn restart_failed(&mut self, error: Error, stopped: bool, cx: &mut Context<Self>) {
        tracing::error!(%error, "应用重启失败");
        let text: SharedString = if stopped { format!("重启失败：{error}。应用已停止，请退出后手动重新启动。") } else { format!("重启失败：{error}。请稍后重试。") }.into();
        if stopped {
            // Service shutdown is irreversible. Retire the settings view and
            // keep the terminal error independent of those stopped services.
            if let Some(settings) = self.settings.take() {
                if let Err(error) = settings.update(cx, |_, window, _| window.remove_window()) {
                    tracing::debug!(%error, "停止后的设置窗口已不可用");
                }
            }
            self.open_error_window("重启失败", text, cx);
            return;
        }
        let shown = self.settings.map(|settings| settings.update(cx, |_, window, cx| {
            ui::settings::show_window(window, None, cx)?;
            window.open_alert_dialog(cx, move |dialog, _, _| dialog.title("重启失败").child(text.clone())
                .button_props(DialogButtonProps::default().ok_text("确定")));
            Ok::<_, anyhow::Error>(())
        }));
        if !matches!(shown, Some(Ok(Ok(())))) { self.show_error("重启失败", error, cx); }
    }

    pub(crate) fn report_overlay_error(&mut self, error: Error, cx: &mut Context<Self>) {
        if self.quitting || self.services.as_ref().is_none_or(|services| services.is_shutting_down()) { return; }
        self.show_error("覆盖层操作失败", error, cx);
    }
    pub(crate) fn open_external_link(&self, link: &super_paste_core::preview::ExternalLink, cx: &mut Context<Self>) {
        if self.quitting || self.services.as_ref().is_none_or(|services| services.is_shutting_down()) { return; }
        cx.open_url(link.href());
    }

    pub(crate) fn show_error(&mut self, title: &'static str, error: Error, cx: &mut Context<Self>) {
        tracing::error!(%error, title, "桌面操作失败");
        if self.quitting { return; }
        if self.is_running() {
            if let Some(settings) = self.settings {
                let message = format!("{title}：{error}");
                if matches!(settings.update(cx, |_, window, cx| {
                    ui::settings::show_window(window, None, cx)?; ui::dialogs::message(message, window, cx); Ok::<_, anyhow::Error>(())
                }), Ok(Ok(()))) { return; }
                self.settings = None;
            }
            if self.opening_settings || self.pending_error.is_none() {
                self.pending_error = Some((title, error)); self.show_settings(cx); return;
            }
        }
        let (title, message): (_, SharedString) = match self.pending_error.take() {
            Some((pending_title, pending_error)) =>
                (pending_title, format!("{pending_error}\n\n{title}：{error}").into()),
            None => (title, error.to_string().into()),
        };
        self.open_error_window(title, message, cx);
    }
    fn open_error_window(&mut self, title: &'static str, message: SharedString, cx: &mut Context<Self>) {
        if let Some(existing) = self.error.take() {
            let _ = existing.update(cx, |_, window, _| window.remove_window());
        }
        match gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| cx.new(|cx| ui::ErrorView::new(title, message, window, cx))) {
            Ok((window, _)) => self.error = Some(window),
            Err(error) => { tracing::error!(%error, "无法显示错误窗口"); cx.quit(); }
        }
    }
}

async fn wait_startup(boot: &mut watch::Receiver<Option<Result<Arc<Services>>>>) -> Result<Arc<Services>> {
    loop {
        if let Some(result) = boot.borrow_and_update().clone() { return result; }
        boot.changed().await.map_err(|_| Error::Task("应用初始化任务意外停止".into()))?;
    }
}
