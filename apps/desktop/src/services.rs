use std::{sync::{Arc, Mutex}, time::Duration};
use super_paste_core::{
    Error, Result, clipboard::ClipboardService, configuration::ConfigurationService,
    credentials::CredentialVault, events::{AppEvent, EventBus},
    images::{ImageBudgets, ImageService}, ocr::{ModelManager, OcrService}, storage::Storage,
};
use super_paste_platform::{
    clipboard::NativeClipboard, credentials::SystemCredentials,
    main_thread::MainThread, ort_runtime::resolve_ort_runtime, paths::AppPaths,
};
use tokio::{runtime::Handle, sync::watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
mod report;
mod power;

/// The desktop owns native resources; business services remain free of GPUI types.
pub struct Services {
    pub paths: AppPaths,
    pub storage: Arc<Storage>,
    pub configuration: Arc<ConfigurationService>,
    pub effects: Arc<crate::effects::DesktopEffects>,
    pub immediate_configuration: Arc<crate::immediate_configuration::ImmediateConfiguration>,
    pub images: Arc<ImageService>,
    pub previews: Arc<super_paste_core::preview::PreviewService>,
    pub clipboard: Arc<ClipboardService>,
    pub tags: Arc<super_paste_core::tags::TagService>,
    pub statistics: Arc<super_paste_core::statistics::StatisticsService>,
    pub ocr: Arc<OcrService>,
    pub sync: Arc<super_paste_core::sync::SyncService<crate::effects::DesktopEffects>>,
    pub mcp: Arc<super_paste_core::mcp::McpService>,
    pub shortcuts: Arc<super_paste_platform::shortcut::ShortcutService>,
    pub access: Arc<super_paste_platform::access::NativeAccess>,
    tray: Mutex<Option<Arc<super_paste_platform::tray::TrayService>>>,
    pub events: EventBus,
    pub runtime: Handle,
    credentials: Arc<CredentialVault>,
    background: TaskTracker,
    cancelled: CancellationToken,
    power: power::PowerLifecycle,
    shutdown: Mutex<Option<watch::Receiver<Option<Result<()>>>>>,
    log: Arc<crate::diagnostics::EventLog>, export_slots: Arc<tokio::sync::Semaphore>,
}

impl Services {
    pub async fn start(runtime: Handle, main_thread: MainThread, paths: AppPaths, activations: tokio::sync::mpsc::Sender<super_paste_platform::instance::Activation>, log: Arc<crate::diagnostics::EventLog>) -> Result<Arc<Self>> {
        paths.initialize().await?;
        let events = EventBus::new(256)?;
        let storage = Storage::open(&paths.database).await?;
        let credentials = CredentialVault::new(storage.clone(), SystemCredentials::new(runtime.clone()));
        let configuration = ConfigurationService::open(storage.clone(), credentials.clone(), events.clone(), runtime.clone()).await?;
        let images = ImageService::new(runtime.clone(), storage.clone(), ImageBudgets::default())?;
        let previews = super_paste_core::preview::PreviewService::new(storage.clone(), runtime.clone());
        let models = ModelManager::new(paths.models.clone(), events.clone(), runtime.clone()).map_err(platform_error)?;
        models.cleanup_staging().await.map_err(platform_error)?;
        let ocr = OcrService::new(storage.clone(), configuration.clone(), images.clone(), models,
            resolve_ort_runtime(), events.clone(), runtime.clone()).await?;
        let native = NativeClipboard::new(runtime.clone(), images.clone());
        let clipboard = ClipboardService::new(runtime.clone(), storage.clone(), configuration.clone(), images.clone(), native.clone(), events.clone(), ocr.clone());
        let mcp = super_paste_core::mcp::McpService::new(configuration.clone(), clipboard.clone(), storage.clone(), images.clone(), ocr.clone(), env!("CARGO_PKG_VERSION").into(), events.clone(), runtime.clone());
        let tray_activations = activations.clone();
        let activate = Arc::new(move |activation_token| {
            if let Err(error) = activations.try_send(super_paste_platform::instance::Activation::ToggleOverlay { activation_token }) {
                tracing::warn!(closed = matches!(error, tokio::sync::mpsc::error::TrySendError::Closed(_)), "全局快捷键激活未接入：应用正在退出或激活队列已满");
            }
        });
        let shortcuts = super_paste_platform::shortcut::ShortcutService::new(main_thread.clone(), runtime.clone(), activate);
        let effects = Arc::new(crate::effects::DesktopEffects {
            ocr: ocr.clone(), auto_start: super_paste_platform::autostart::AutoStart::new(runtime.clone(), &paths),
            mcp: mcp.clone(), shortcuts: shortcuts.clone(),
        });
        // Match the original startup contract: repair this project's own
        // launch entry, report failure, and keep history/settings available.
        // Do this before sync or UI can submit configuration transactions.
        if let Err(error) = effects.auto_start.reconcile(configuration.snapshot().snapshot.value.settings.auto_start).await {
            tracing::error!(%error, "开机启动状态初始化失败");
        }
        let sync = match super_paste_core::sync::SyncService::new(storage.clone(), configuration.clone(), effects.clone(), events.clone(), paths.data.join("sync-conflicts"), runtime.clone()).await {
            Ok(sync) => sync,
            Err(error) => {
                let configuration = configuration.shutdown().await;
                let (mcp, shortcuts) = tokio::join!(mcp.shutdown(), shortcuts.shutdown());
                let clipboard = clipboard.shutdown().await;
                let (previews, ocr) = tokio::join!(previews.shutdown(), ocr.shutdown());
                let images = images.shutdown().await;
                let cleanup = [mcp, configuration, shortcuts, clipboard, previews, ocr, images].into_iter().filter_map(Result::err).map(|error| error.to_string()).collect::<Vec<_>>();
                if cleanup.is_empty() { return Err(error); }
                return Err(Error::Task(format!("{error}；初始化回收失败：{}", cleanup.join("；")).into()));
            }
        };

        // Capture failure does not prevent access to persisted history and settings.
        let access = super_paste_platform::access::NativeAccess::new(runtime.clone(), main_thread.clone(),
            images.clone(), native, clipboard.clone(), configuration.clone(), shortcuts.clone(), events.clone());
        if let Err(error) = access.retry().await { tracing::warn!(%error, "系统接入启动不可用，可在设置中重试"); }
        if let Err(error) = mcp.retry(configuration.snapshot()).await { tracing::warn!(%error, "MCP 启动不可用，可在设置中重试"); }
        let immediate_configuration = crate::immediate_configuration::ImmediateConfiguration::new(configuration.clone(), effects.clone(), runtime.clone());
        let tags = super_paste_core::tags::TagService::new(storage.clone(), events.clone(), runtime.clone());
        let statistics = super_paste_core::statistics::StatisticsService::new(storage.clone(), events.clone(), runtime.clone());
        let services = Arc::new(Self {
            paths, storage, configuration, effects, immediate_configuration, images, previews, clipboard, tags, statistics, ocr, sync, mcp, shortcuts, access, events, runtime,
            credentials, background: TaskTracker::new(), tray: Mutex::new(None),
            cancelled: CancellationToken::new(), power: power::PowerLifecycle::new(), shutdown: Mutex::new(None), log, export_slots: Arc::new(tokio::sync::Semaphore::new(1)),
        });
        let action = Arc::new(move |action| {
            if let Err(error) = tray_activations.try_send(super_paste_platform::instance::Activation::Tray(action)) {
                tracing::warn!(closed = matches!(error, tokio::sync::mpsc::error::TrySendError::Closed(_)), "托盘操作未接入：应用正在退出或激活队列已满");
            }
        });
        let initial = super_paste_platform::tray::TrayState::from(&services.configuration.snapshot().snapshot.value.settings);
        match super_paste_platform::tray::TrayService::start(&services.runtime, main_thread, initial, action).await {
            Ok(tray) => *services.tray.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(tray),
            Err(error) => {
                return match services.shutdown().await {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(Error::Task(format!("{error}；初始化回收失败：{cleanup}").into())),
                };
            }
        }
        let gc = services.clone();
        services.background.spawn_on(async move { gc.credential_cleanup().await }, &services.runtime);
        let resources = services.clone(); let events = services.events.subscribe();
        services.background.spawn_on(async move { resources.preview_invalidation(events).await }, &services.runtime);
        let power = services.clone();
        services.background.spawn_on(async move { power.power_lifecycle().await }, &services.runtime);
        Ok(services)
    }

    async fn preview_invalidation(self: Arc<Self>, mut events: tokio::sync::broadcast::Receiver<AppEvent>) {
        loop {
            let result = tokio::select! {
                _ = self.cancelled.cancelled() => return,
                event = events.recv() => match event {
                    Ok(AppEvent::HistoryChanged { ids, change: super_paste_core::events::RecordChange::Removed }) => self.previews.invalidate(&ids),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => self.previews.trim(),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    _ => continue,
                }
            };
            if let Err(error) = result { tracing::warn!(%error, "预览资源失效失败"); }
        }
    }

    pub fn subscribe_credential_cleanup(&self) -> watch::Receiver<Option<Error>> { self.credentials.subscribe_cleanup() }

    async fn credential_cleanup(self: Arc<Self>) {
        let mut updates = self.credentials.subscribe_cleanup();
        let mut retry = true;
        let mut delay = Duration::from_secs(5);
        loop {
            if self.cancelled.is_cancelled() { return; }
            if retry {
                // Consume before scanning, so a newer failure after a successful
                // scan cannot be acknowledged without scheduling its retry.
                updates.borrow_and_update();
                if let Err(error) = self.credentials.collect_garbage().await {
                    self.events.publish(AppEvent::CredentialCleanupFailed(error));
                    tokio::select! {
                        _ = self.cancelled.cancelled() => return,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    delay = (delay * 2).min(Duration::from_secs(60));
                    continue;
                }
                delay = Duration::from_secs(5);
                retry = false;
            }
            tokio::select! {
                _ = self.cancelled.cancelled() => return,
                changed = updates.changed() => {
                    if changed.is_err() { return; }
                    retry = updates.borrow_and_update().is_some();
                }
            }
        }
    }

    pub fn is_shutting_down(&self) -> bool { self.shutdown.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some() }
    pub fn update_tray(&self, settings: &super_paste_format::Preferences, force: bool) -> Result<()> {
        let tray = self.tray.lock().unwrap_or_else(|poison| poison.into_inner()).clone();
        let tray = tray.ok_or_else(|| Error::Platform("托盘尚未就绪或已停止".into()))?;
        let state = super_paste_platform::tray::TrayState::from(settings);
        if force { tray.refresh(state) } else { tray.update(state) }
    }
    /// An accepted shutdown is owned by the runtime, even if its requesting view disappears.
    pub async fn shutdown(self: &Arc<Self>) -> Result<()> {
        let mut completion = {
            let mut state = self.shutdown.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match &*state {
                Some(completion) => completion.clone(),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    *state = Some(receiver.clone());
                    let services = self.clone();
                    self.runtime.spawn(async move {
                        sender.send_replace(Some(services.shutdown_inner().await));
                    });
                    receiver
                }
            }
        };
        loop {
            if let Some(result) = completion.borrow_and_update().clone() { return result; }
            completion.changed().await.map_err(|_| Error::Task("应用退出任务意外停止".into()))?;
        }
    }

    async fn shutdown_inner(&self) -> Result<()> {
        self.power.stop();
        let mut errors = Vec::new();
        // Stop native observers while GPUI still services the main-thread queue;
        // accepted clipboard captures and configuration commits drain concurrently.
        let capture = self.access.shutdown();
        let tray = self.tray.lock().unwrap_or_else(|poison| poison.into_inner()).take();
        let tray = async move { match tray { Some(tray) => tray.shutdown().await, None => Ok(()) } };
        let configuration = async {
            let immediate = self.immediate_configuration.shutdown().await;
            let sync = self.sync.shutdown().await;
            let configuration = self.configuration.shutdown().await;
            // Configuration owns accepted transactions. Drain their MCP effects
            // before closing the service they may still need to apply or undo.
            let mcp = self.mcp.shutdown().await;
            let errors = [immediate, sync, mcp, configuration].into_iter().filter_map(Result::err).map(|error| error.to_string()).collect::<Vec<_>>();
            if errors.is_empty() { Ok(()) } else { Err(Error::Task(errors.join("；").into())) }
        };
        let (configuration, capture, tray, tags, statistics) = tokio::join!(configuration, capture, tray, self.tags.shutdown(), self.statistics.shutdown());
        collect(&mut errors, configuration);
        collect(&mut errors, capture);
        collect(&mut errors, tray);
        // Both accepted configuration effects and access retries may own shortcut work.
        collect(&mut errors, self.shortcuts.shutdown().await);
        collect(&mut errors, tags);
        collect(&mut errors, statistics);
        // Native writes own their following history commit. Stop producers first,
        // then drain that complete operation before OCR/image/storage teardown.
        collect(&mut errors, self.clipboard.shutdown().await);
        collect(&mut errors, self.previews.shutdown().await);
        collect(&mut errors, self.ocr.shutdown().await);
        self.cancelled.cancel();
        self.background.close();
        self.background.wait().await;
        collect(&mut errors, self.credentials.collect_garbage().await);
        collect(&mut errors, self.images.shutdown().await);
        collect(&mut errors, self.storage.checkpoint().await);
        if errors.is_empty() { Ok(()) } else { Err(Error::Task(errors.join("；").into())) }
    }
}

fn collect(errors: &mut Vec<String>, result: Result<()>) {
    if let Err(error) = result { errors.push(error.to_string()); }
}
fn platform_error(error: String) -> Error { Error::Platform(error.into()) }
