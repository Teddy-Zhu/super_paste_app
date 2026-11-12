mod effects;
use effects::EffectHooks;

use crate::{
    config::{ConfigService, StoreMutationMetadata, WindowAccess},
    core::{clipboard::ClipboardService, mcp_config::McpConfiguration},
    credentials::CredentialStore,
    diagnostics::Diagnostics,
    events::{self, SettingsProvider},
    mcp::McpService,
    ocr::{OcrService, OcrTrigger},
    platform::{active_window::SourceTracker, shortcut::ShortcutService, windows::WindowManager},
    storage::Storage,
    sync::SyncService,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub struct RuntimeHolder(pub Arc<OnceLock<Arc<Runtime>>>);

impl RuntimeHolder {
    pub fn get(&self) -> Result<Arc<Runtime>, String> {
        self.0
            .get()
            .cloned()
            .ok_or_else(|| "应用服务尚未就绪".into())
    }
}

#[derive(Default)]
struct PowerState {
    generation: u64,
    suspended: bool,
}

pub struct Runtime {
    pub app: tauri::AppHandle,
    pub data_dir: PathBuf,
    pub storage: Arc<Storage>,
    pub config: Arc<ConfigService>,
    pub clipboard: Arc<ClipboardService>,
    pub source: Arc<SourceTracker>,
    pub ocr: Arc<OcrService>,
    pub sync: Arc<SyncService>,
    pub mcp_config: McpConfiguration,
    pub windows: Arc<WindowManager>,
    pub shortcuts: Arc<ShortcutService>,
    pub diagnostics: Diagnostics,
    pub stop: CancellationToken,
    quitting: AtomicBool,
    listener_gate: tokio::sync::Mutex<()>,
    power_state: Mutex<PowerState>,
    power_lifecycle_gate: tokio::sync::Mutex<()>,
    power: Mutex<Option<crate::platform::power::PowerLifecycleAdapter>>,
}

impl Runtime {
    pub async fn initialize(
        app: tauri::AppHandle,
        holder: RuntimeHolder,
        diagnostics: Diagnostics,
    ) -> Result<Arc<Self>, String> {
        let data_dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
        let storage = Storage::open(&data_dir).await?;
        let credentials = Arc::new(CredentialStore::new());
        let link = Arc::new(OnceLock::new());
        let config = Arc::new(ConfigService::new(
            storage.clone(),
            credentials.clone(),
            events::sink(app.clone()),
            Arc::new(EffectHooks {
                runtime: link.clone(),
            }),
        ));
        let safe_config = config.clone();
        let settings: SettingsProvider = Arc::new(move || {
            let config = safe_config.clone();
            Box::pin(async move { config.runtime_settings().await })
        });
        let ocr_config = config.clone();
        let ocr_settings: SettingsProvider = Arc::new(move || {
            let config = ocr_config.clone();
            Box::pin(async move { config.runtime_ocr_settings().await })
        });
        let ai_config = config.clone();
        let ai_provider: crate::ocr::AIConfigProvider = Arc::new(move |provider: &str| {
            let config = ai_config.clone();
            let provider = provider.to_owned();
            Box::pin(async move { config.runtime_ai_config(&provider).await })
        });
        let source = SourceTracker::start(app.clone())?;
        let clipboard = ClipboardService::new(
            storage.clone(),
            settings.clone(),
            events::sink(app.clone()),
            source.clone(),
        );
        let disable_config = config.clone();
        let disable = Arc::new(move || {
            let config = disable_config.clone();
            Box::pin(async move {
                let snapshot = config.snapshot().await?;
                let metadata = StoreMutationMetadata {
                    client_id: "host:model-maintenance".into(),
                    mutation_id: uuid::Uuid::new_v4().to_string(),
                    base_versions: snapshot.versions,
                    credential_changes: BTreeMap::new(),
                };
                let result = config
                    .patch(
                        "settings",
                        json!({"enableLocalOCR":false}),
                        metadata,
                        WindowAccess::Main,
                    )
                    .await;
                if result.success {
                    Ok(())
                } else {
                    Err(result.error.unwrap_or_else(|| "无法关闭自动 OCR".into()))
                }
            }) as crate::ocr::OcrFuture
        });
        let ocr = Arc::new(OcrService::new(
            data_dir.join(data_format::OCR_MODELS_DIRECTORY),
            crate::platform::ort_runtime::resolve_ort_runtime(&app),
            storage.clone(),
            ocr_settings,
            ai_provider,
            disable,
            events::sink(app.clone()),
        )?);
        let ocr_hook = ocr.clone();
        clipboard.attach_ocr(Arc::new(move |id, automatic| {
            let ocr = ocr_hook.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = ocr
                    .enqueue_record(
                        &id,
                        if automatic {
                            OcrTrigger::Automatic
                        } else {
                            OcrTrigger::Manual
                        },
                    )
                    .await
                {
                    tracing::error!(%error,"automatic OCR request failed");
                }
            });
        }))?;
        let removed_ocr = ocr.clone();
        clipboard.attach_removed(Arc::new(move |ids| {
            let ocr = removed_ocr.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = ocr.cancel_records(&ids).await {
                    tracing::error!(%error,"removed record OCR cancellation failed");
                }
            });
        }))?;
        let backend = Arc::new(crate::core::mcp_backend::Backend {
            clipboard: clipboard.clone(),
            storage: storage.clone(),
            ocr: ocr.clone(),
        });
        let mcp = Arc::new(McpService::new(
            backend,
            app.package_info().version.to_string(),
            events::sink(app.clone()),
        ));
        let observer = mcp.clone();
        clipboard.attach_observer(Arc::new(move |item| {
            let Some(timestamp) = item["timestamp"].as_f64() else {
                tracing::error!("clipboard change has no capture timestamp");
                return;
            };
            let change = crate::mcp::McpClipboardChange {
                timestamp,
                content: item["content"].clone(),
            };
            observer.record_clipboard_change(change);
        }))?;
        let windows = WindowManager::new(app.clone(), settings, source.clone());
        let toggle = windows.clone();
        let shortcuts = ShortcutService::new(
            app.clone(),
            Arc::new(move |token| {
                let windows = toggle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = windows.toggle(token).await {
                        tracing::error!(%error,"shortcut overlay activation failed");
                    }
                });
            }),
        );
        let sync = SyncService::new(
            config.clone(),
            storage.clone(),
            events::sink(app.clone()),
            data_dir.clone(),
        );
        let mcp_config = McpConfiguration::new(storage.clone(), credentials, mcp.clone());
        let runtime = Arc::new(Self {
            app: app.clone(),
            data_dir,
            storage,
            config,
            clipboard,
            source,
            ocr,
            sync,
            mcp_config,
            windows,
            shortcuts,
            diagnostics,
            stop: CancellationToken::new(),
            quitting: AtomicBool::new(false),
            listener_gate: tokio::sync::Mutex::new(()),
            power_state: Mutex::new(PowerState::default()),
            power_lifecycle_gate: tokio::sync::Mutex::new(()),
            power: Mutex::new(None),
        });
        link.set(Arc::downgrade(&runtime))
            .map_err(|_| "配置服务重复装配")?;
        holder
            .0
            .set(runtime.clone())
            .map_err(|_| "应用服务重复初始化")?;
        runtime.windows.create(&runtime.data_dir).await?;
        crate::tray::create(&runtime).await?;
        runtime.apply_initial().await;
        if let Err(error) = runtime.start_power().await {
            tracing::error!(%error,"system sleep monitoring unavailable");
        }
        runtime.start_maintenance();
        if std::env::args().any(|arg| arg == "--toggle-overlay") {
            runtime.windows.show(None).await?;
        } else if !std::env::args().any(|arg| arg == "--background") {
            let settings = runtime.config.runtime_settings().await?;
            let silent_start = settings["silentStart"].as_bool().ok_or("静默启动配置无效")?;
            if !silent_start {
                runtime.windows.open_main().await?;
            }
        }
        runtime.initialize_background_services();
        tracing::info!("desktop runtime ready");
        Ok(runtime)
    }

    async fn apply_initial(self: &Arc<Self>) {
        let settings = match self.config.runtime_settings().await {
            Ok(settings) => settings,
            Err(error) => {
                tracing::error!(%error,"initial configuration read failed");
                return;
            }
        };
        let autostart = match settings["autoStart"].as_bool() {
            Some(enabled) => self.apply_autostart(enabled).await,
            None => Err("自启配置无效".into()),
        };
        if let Err(error) = autostart {
            tracing::error!(%error,"autostart initialization failed");
        }
        let shortcut = match settings["globalShortcut"].as_str() {
            Some(accelerator) => self.shortcuts.apply(accelerator).await,
            None => Err("快捷键配置无效".into()),
        };
        if let Err(error) = shortcut {
            tracing::error!(%error,"shortcut initialization failed");
        }
        if let Err(error) = self.apply_theme(&settings["colorMode"]) {
            tracing::error!(%error,"native theme initialization failed");
        }
        if let Err(error) = self.reconcile_listening(false).await {
            tracing::error!(%error,"clipboard initialization failed");
        }
        if let Err(error) = self.ocr.initialize().await {
            tracing::error!(%error,"automatic OCR preparation failed");
        }
        if let Err(error) = self.sync.start().await {
            tracing::error!(%error,"scheduled sync initialization failed");
        }
    }

    fn initialize_background_services(self: &Arc<Self>) {
        let app = self.app.clone();
        let storage = self.storage.clone();
        let stop = self.stop.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) =
                crate::platform::source_icon::refresh_saved(app, storage, stop).await
            {
                tracing::warn!(%error, "saved source icon refresh failed");
            }
        });
        let weak = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            if runtime.is_quitting() {
                return;
            }
            if let Err(error) = runtime.mcp_config.initialize().await {
                tracing::error!(%error,"MCP initialization failed");
            }
            if runtime.is_quitting() {
                return;
            }
            if let Err(error) = runtime.mcp_config.retry_cleanup().await {
                tracing::warn!(%error,"pending MCP credential cleanup failed");
            }
            if let Err(error) = runtime.config.retry_credential_cleanup().await {
                tracing::warn!(%error,"pending credential cleanup failed");
            }
        });
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Acquire)
    }

    async fn native_listening(&self, enabled: bool, force_restart: bool) -> Result<(), String> {
        let clipboard = self.clipboard.clone();
        tokio::task::spawn_blocking(move || {
            if force_restart {
                clipboard.stop()?;
            }
            if enabled {
                clipboard.start()
            } else {
                clipboard.stop()
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn set_listening(&self, enabled: bool) -> Result<(), String> {
        let _gate = self.listener_gate.lock().await;
        if self.is_quitting() {
            return Err("应用正在退出".into());
        }
        if enabled
            && self
                .power_state
                .lock()
                .map_err(|_| "power state lock poisoned")?
                .suspended
        {
            return Err("系统尚未完成恢复".into());
        }
        self.native_listening(enabled, false).await
    }

    pub async fn reconcile_listening(&self, force_restart: bool) -> Result<(), String> {
        let _gate = self.listener_gate.lock().await;
        if self.is_quitting() {
            return Err("应用正在退出".into());
        }
        let settings = self.config.runtime_settings().await?;
        let suspended = self
            .power_state
            .lock()
            .map_err(|_| "power state lock poisoned")?
            .suspended;
        let enabled = settings["clipboardListening"]
            .as_bool()
            .ok_or("监听配置无效")?
            && !suspended;
        self.native_listening(enabled, force_restart).await
    }

    async fn start_power(self: &Arc<Self>) -> Result<(), String> {
        let weak = Arc::downgrade(self);
        let adapter = tokio::task::spawn_blocking(move || {
            crate::platform::power::PowerLifecycleAdapter::start(Arc::new(move |sleeping| {
                if let Some(runtime) = weak.upgrade() {
                    runtime.power_event(sleeping);
                }
            }))
        })
        .await
        .map_err(|e| e.to_string())??;
        *self
            .power
            .lock()
            .map_err(|_| "power lifecycle lock poisoned")? = Some(adapter);
        Ok(())
    }

    fn power_event(self: Arc<Self>, sleeping: bool) {
        let generation = match self.power_state.lock() {
            Ok(mut state) => {
                state.generation = state.generation.wrapping_add(1);
                state.suspended = true;
                state.generation
            }
            Err(_) => {
                tracing::error!("power state lock poisoned");
                return;
            }
        };
        tauri::async_runtime::spawn(async move {
            if !sleeping {
                tokio::select! {_=self.stop.cancelled()=>return,_=tokio::time::sleep(std::time::Duration::from_millis(2000))=>{}}
            }
            let _lifecycle = self.power_lifecycle_gate.lock().await;
            if self.is_quitting() {
                return;
            }
            match self.power_state.lock() {
                Ok(mut state) => {
                    if state.generation != generation {
                        return;
                    }
                    if !sleeping {
                        state.suspended = false;
                    }
                }
                Err(_) => {
                    tracing::error!("power state lock poisoned");
                    return;
                }
            }
            if sleeping {
                if let Err(error) = self.set_listening(false).await {
                    tracing::error!(%error,"clipboard suspend failed");
                }
                if let Err(error) = self.ocr.suspend().await {
                    tracing::error!(%error,"OCR suspend failed");
                }
            } else {
                if let Err(error) = self.ocr.resume().await {
                    tracing::error!(%error,"OCR resume failed");
                }
                if let Err(error) = self.reconcile_listening(true).await {
                    tracing::error!(%error,"clipboard resume failed");
                }
            }
        });
    }

    pub async fn set_setting(&self, key: &str, value: Value, origin: &str) -> Result<(), String> {
        let snapshot = self.config.snapshot().await?;
        let metadata = StoreMutationMetadata {
            client_id: format!("host:{origin}"),
            mutation_id: uuid::Uuid::new_v4().to_string(),
            base_versions: snapshot.versions,
            credential_changes: BTreeMap::new(),
        };
        let result = self
            .config
            .patch(
                "settings",
                json!({(key):value}),
                metadata,
                WindowAccess::Main,
            )
            .await;
        if result.success {
            Ok(())
        } else {
            Err(result.error.unwrap_or_else(|| "配置保存失败".into()))
        }
    }

    async fn apply_autostart(&self, enabled: bool) -> Result<(), String> {
        let app = self.app.clone();
        tokio::task::spawn_blocking(move || {
            if enabled {
                #[cfg(target_os = "linux")]
                {
                    crate::platform::autostart::enable(&app)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    app.autolaunch().enable().map_err(|e| e.to_string())
                }
            } else {
                app.autolaunch().disable().map_err(|e| e.to_string())
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }

    fn apply_theme(&self, mode: &Value) -> Result<(), String> {
        let theme = match mode.as_str() {
            Some("dark") => Some(tauri::Theme::Dark),
            Some("light") => Some(tauri::Theme::Light),
            Some("system") => None,
            _ => return Err("颜色模式无效".into()),
        };
        for window in self.app.windows().values() {
            window.set_theme(theme).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub async fn cleanup_history(&self) -> Result<(), String> {
        let settings = self.config.runtime_settings().await?;
        let ids = self
            .storage
            .cleanup_history(
                settings["maxHistorySize"]
                    .as_i64()
                    .ok_or("历史数量限制无效")?,
                settings["maxKeepTime"].as_i64().ok_or("历史保留时间无效")?,
            )
            .await?;
        if !ids.is_empty() {
            self.ocr.cancel_records(&ids).await?;
            (events::sink(self.app.clone()))("clipboard-history-updated", Value::Null);
        }
        Ok(())
    }

    pub fn configuration_committed(self: Arc<Self>, key: String) {
        tauri::async_runtime::spawn(async move {
            if self.is_quitting() {
                return;
            }
            // Data retention runs only after the configuration transaction has committed.
            if ["settings.maxHistorySize", "settings.maxKeepTime"].contains(&key.as_str())
                && let Err(error) = self.cleanup_history().await
            {
                tracing::error!(%error,"committed retention update failed");
            }
            if let Err(error) = crate::tray::refresh(&self).await {
                tracing::error!(%error,"committed tray update failed");
            }
        });
    }

    fn start_maintenance(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let stop = self.stop.clone();
        tauri::async_runtime::spawn(async move {
            let mut retention = tokio::time::interval(std::time::Duration::from_secs(60));
            let mut displays = tokio::time::interval(std::time::Duration::from_secs(2));
            retention.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            displays.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _=stop.cancelled()=>break,
                    _=retention.tick()=>{let Some(runtime)=weak.upgrade()else{break};if let Err(error)=runtime.cleanup_history().await{tracing::error!(%error,"history cleanup failed");}},
                    _=displays.tick()=>{let Some(runtime)=weak.upgrade()else{break};if let Err(error)=runtime.windows.reconcile_monitors().await{tracing::error!(%error,"monitor reconciliation failed");}},
                }
            }
        });
    }

    pub async fn system_access(&self) -> Result<Value, String> {
        let clipboard = self.clipboard.clone();
        let available = tokio::task::spawn_blocking(move || {
            clipboard_core::check_available().map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?;
        Ok(
            json!({"wayland":clipboard_core::is_wayland_session(),"shortcut":self.shortcuts.state()?,"clipboard":{"available":available.is_ok(),"listening":clipboard.is_listening(),"error":available.err().or_else(||clipboard.error())},"sourceTracking":self.source.available(),"executable":tauri::process::current_binary(&self.app.env()).map_err(|e|e.to_string())?.to_string_lossy()}),
        )
    }

    pub async fn retry_access(self: &Arc<Self>) -> Result<Value, String> {
        let settings = self.config.runtime_settings().await?;
        let shortcut = self
            .shortcuts
            .apply(
                settings["globalShortcut"]
                    .as_str()
                    .ok_or("快捷键配置无效")?,
            )
            .await;
        if let Err(error) = shortcut {
            tracing::error!(%error,"shortcut access retry failed");
        }
        if let Err(error) = self.reconcile_listening(true).await {
            tracing::error!(%error,"clipboard access retry failed");
        }
        self.system_access().await
    }

    pub async fn shutdown(self: &Arc<Self>, restart: bool) {
        if self.quitting.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stop.cancel();
        self.source.stop();
        let power = match self.power.lock() {
            Ok(mut power) => power.take(),
            Err(poisoned) => {
                tracing::error!("power lifecycle lock poisoned during shutdown");
                poisoned.into_inner().take()
            }
        };
        let cleanup = async {
            let (clipboard, ocr, (), mcp, shortcut, power) = tokio::join!(
                self.clipboard.shutdown(),
                self.ocr.shutdown(),
                self.sync.shutdown(),
                self.mcp_config.shutdown(),
                self.shortcuts.shutdown(),
                tokio::task::spawn_blocking(move || {
                    if let Some(power) = power {
                        power.shutdown();
                    }
                })
            );
            match clipboard {
                Ok(()) => {}
                Err(error) => tracing::error!(%error,"clipboard shutdown failed"),
            }
            if let Err(error) = ocr {
                tracing::error!(%error,"OCR shutdown failed");
            }
            if let Err(error) = mcp {
                tracing::error!(%error,"MCP shutdown failed");
            }
            if let Err(error) = shortcut {
                tracing::error!(%error,"shortcut shutdown failed");
            }
            if let Err(error) = power {
                tracing::error!(%error,"system power monitor shutdown task failed");
            }
            self.config.wait_idle().await;
            if let Err(error) = self.storage.flush().await {
                tracing::error!(%error,"database flush failed");
            }
        };
        if tokio::time::timeout(std::time::Duration::from_secs(8), cleanup)
            .await
            .is_err()
        {
            tracing::error!("shutdown grace period expired");
        }
        if restart {
            self.app.restart();
        } else {
            self.app.exit(0);
        }
    }
}
