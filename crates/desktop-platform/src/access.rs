//! Owns reconnectable native observers independently of settings-page lifetime.
use std::sync::{Arc, Mutex};
use super_paste_core::{Error, Result, clipboard::ClipboardService, configuration::ConfigurationService, events::{AppEvent, EventBus}, images::ImageService};
use super_paste_format::{ConfigField, SourceApp};
use tokio::{runtime::Handle, sync::Semaphore};
use tokio_util::task::TaskTracker;
use crate::{clipboard::{ClipboardWatch, NativeClipboard, SourceReader}, main_thread::MainThread, shortcut::ShortcutService, source::{SourceDiagnostics, SourceTracker}, source_icon::SourceIcons};

#[derive(Clone)]
pub struct AccessStatus {
    pub clipboard_listening: bool,
    pub suspended: bool,
    pub clipboard_error: Option<Error>,
    pub source: Option<SourceDiagnostics>,
    pub source_error: Option<Error>,
}
#[derive(Default)]
struct State { source: Option<Arc<SourceTracker>>, source_error: Option<Error>, clipboard_error: Option<Error>, source_freezes: usize, suspended: bool,
    clipboard_startup: Option<clipboard_core::ReadCancellation> }

/// Each pending activation or visible overlay owns its own freeze. Releasing
/// an older owner cannot unfreeze tracking for a newer activation or window.
#[must_use = "keep this handle alive while source tracking must remain frozen"]
pub struct SourceFreeze(Option<Arc<NativeAccess>>);
impl SourceFreeze {
    pub fn release(mut self) -> Result<()> {
        match self.0.take() { Some(access) => access.release_source(), None => Ok(()) }
    }
}
impl Drop for SourceFreeze {
    fn drop(&mut self) {
        if let Some(access) = self.0.take() {
            if let Err(error) = access.release_source() { tracing::error!(%error, "释放来源冻结失败"); }
        }
    }
}

pub struct NativeAccess {
    runtime: Handle, main_thread: MainThread, images: Arc<ImageService>, native: Arc<NativeClipboard>,
    clipboard: Arc<ClipboardService>, configuration: Arc<ConfigurationService>, shortcuts: Arc<ShortcutService>, events: EventBus,
    state: Mutex<State>, watch: tokio::sync::Mutex<Option<ClipboardWatch>>,
    admission: Mutex<bool>, slots: Arc<Semaphore>, tasks: TaskTracker,
}
impl NativeAccess {
    pub fn new(runtime: Handle, main_thread: MainThread, images: Arc<ImageService>, native: Arc<NativeClipboard>, clipboard: Arc<ClipboardService>, configuration: Arc<ConfigurationService>, shortcuts: Arc<ShortcutService>, events: EventBus) -> Arc<Self> {
        Arc::new(Self { runtime, main_thread, images, native, clipboard, configuration, shortcuts, events,
            state: Mutex::new(State::default()), watch: tokio::sync::Mutex::new(None),
            admission: Mutex::new(false), slots: Arc::new(Semaphore::new(1)), tasks: TaskTracker::new() })
    }
    pub fn status(&self) -> Result<AccessStatus> {
        let state = self.state.lock().map_err(|_| broken())?;
        Ok(AccessStatus { clipboard_listening: !state.suspended && self.clipboard.is_listening(), suspended: state.suspended, clipboard_error: state.clipboard_error.clone(),
            source: state.source.as_ref().map(|source| source.diagnostics().map_err(platform)).transpose()?, source_error: state.source_error.clone() })
    }
    pub fn freeze_source(self: &Arc<Self>) -> Result<SourceFreeze> {
        let mut state = self.state.lock().map_err(|_| broken())?;
        state.source_freezes = state.source_freezes.checked_add(1).ok_or_else(|| Error::ResourceLimit("来源冻结数量超出范围".into()))?;
        if state.source_freezes == 1 { if let Some(source) = &state.source { source.freeze(true); } }
        Ok(SourceFreeze(Some(self.clone())))
    }
    fn release_source(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| broken())?;
        state.source_freezes = state.source_freezes.checked_sub(1).ok_or_else(|| Error::Task("来源冻结计数不一致".into()))?;
        if state.source_freezes == 0 { if let Some(source) = &state.source { source.freeze(false); } }
        Ok(())
    }
    fn read_source(&self, has_image: bool) -> Result<Option<Arc<SourceApp>>> {
        let source = self.state.lock().map_err(|_| broken())?.source.clone();
        source.map(|source| source.snapshot(has_image).map_err(platform)).transpose().map(Option::flatten)
    }
    /// Record the target on the event thread before asynchronous teardown.
    /// Explicit retries may refresh other adapters, but cannot restart capture
    /// while sleep or the wake grace period still owns this suspension.
    pub fn set_suspended(&self, suspended: bool) -> Result<()> {
        let initialization = {
            let mut state = self.state.lock().map_err(|_| broken())?;
            state.suspended = suspended;
            if suspended { state.clipboard_startup.take() } else { None }
        };
        if let Some(initialization) = initialization { initialization.cancel(); }
        Ok(())
    }
    /// Power changes share refresh admission and ownership with explicit retry.
    /// One lifecycle worker waits for a current refresh instead of dropping a
    /// required stop/restart when the user is already refreshing permissions.
    pub async fn reconcile_power(self: &Arc<Self>) -> Result<()> {
        let task = {
            let closing = self.admission.lock().map_err(|_| broken())?;
            if *closing { return Err(Error::Cancelled); }
            let service = self.clone();
            self.tasks.spawn_on(async move {
                let _permit = service.slots.clone().acquire_owned().await.map_err(|_| Error::Cancelled)?;
                if *service.admission.lock().map_err(|_| broken())? { return Err(Error::Cancelled); }
                service.reconcile_clipboard(true).await
            }, &self.runtime)
        };
        task.await.map_err(Error::from)?
    }
    /// Only one explicit refresh is accepted. Closing the page cannot abandon
    /// native unregister/register work or the configuration-version check.
    pub async fn retry(self: &Arc<Self>) -> Result<()> {
        let task = {
            let closing = self.admission.lock().map_err(|_| broken())?;
            if *closing { return Err(Error::Cancelled); }
            let permit = self.slots.clone().try_acquire_owned().map_err(|_| Error::ResourceLimit("系统接入刷新正在执行".into()))?;
            let service = self.clone();
            self.tasks.spawn_on(async move { let _permit = permit; service.reconnect().await }, &self.runtime)
        };
        task.await.map_err(Error::from)?
    }
    async fn reconnect(self: &Arc<Self>) -> Result<()> {
        let mut errors = Vec::new();
        let previous = self.state.lock().map_err(|_| broken())?.source.take();
        let stopped = match previous.as_ref() {
            Some(source) => source.shutdown(&self.runtime).await.map_err(platform),
            None => Ok(()),
        };
        {
            let mut state = self.state.lock().map_err(|_| broken())?;
            // Publish the replacement under the same lock as freeze ownership.
            // Its first sample must already observe the current freeze state.
            let source = match stopped {
                Ok(()) => SourceTracker::start(self.main_thread.clone(), SourceIcons::new(self.images.clone(), self.runtime.clone()),
                    state.source_freezes != 0, previous.as_deref()).map_err(platform),
                Err(error) => Err(error),
            };
            match source {
                Ok(source) => {
                    state.source = Some(source); state.source_error = None;
                }
                Err(error) => { state.source_error = Some(error.clone()); errors.push(error); }
            }
        }
        if let Err(error) = self.reconcile_clipboard(false).await { errors.push(error); }
        let expected = self.configuration.snapshot();
        let accelerator = expected.snapshot.value.settings.global_shortcut.clone();
        let configuration = self.configuration.clone();
        if let Err(error) = self.shortcuts.reconcile(accelerator, move || {
            if configuration.snapshot().fields.get(&ConfigField::GlobalShortcut) != expected.fields.get(&ConfigField::GlobalShortcut) {
                return Err(Error::Conflict("全局快捷键已更新".into()));
            }
            Ok(())
        }).await { errors.push(error); }
        if errors.is_empty() { Ok(()) } else { Err(Error::Platform(errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("；").into())) }
    }
    async fn reconcile_clipboard(self: &Arc<Self>, force: bool) -> Result<()> {
        let suspended = self.state.lock().map_err(|_| broken())?.suspended;
        if force || suspended || !self.clipboard.is_native_listening() {
            let mut slot = self.watch.lock().await;
            let stopped = match slot.take() { Some(watch) => watch.stop().await, None => Ok(()) };
            let result = match stopped {
                Err(error) => Err(error),
                Ok(()) => {
                    // Re-read after waiting for callbacks and queued captures.
                    if self.state.lock().map_err(|_| broken())?.suspended { return Ok(()); }
                    let weak = Arc::downgrade(self); let events = self.events.clone();
                    let reader: SourceReader = Arc::new(move |has_image| {
                        let Some(access) = weak.upgrade() else { return Ok(None); };
                        match access.read_source(has_image) {
                            Ok(source) => Ok(source),
                            Err(error) => { events.publish(AppEvent::SourceTrackingFailed(error)); Ok(None) }
                        }
                    });
                    let initialization = match clipboard_core::ReadCancellation::new() {
                        Ok(control) => control,
                        Err(error) => {
                            let error = Error::Platform(error.to_string().into());
                            self.state.lock().map_err(|_| broken())?.clipboard_error = Some(error.clone());
                            return Err(error);
                        },
                    };
                    {
                        // Publish before the first native operation. Power and
                        // shutdown can cancel without waiting for this slot.
                        let closing = self.admission.lock().map_err(|_| broken())?;
                        let mut state = self.state.lock().map_err(|_| broken())?;
                        if *closing { return Err(Error::Cancelled); }
                        if state.suspended { return Ok(()); }
                        state.clipboard_startup = Some(initialization.clone());
                    }
                    let result = self.native.start_watch(self.clipboard.clone(), reader, self.events.clone(), initialization).await;
                    self.state.lock().map_err(|_| broken())?.clipboard_startup = None;
                    result
                }
            };
            match result {
                Ok(watch) => {
                    let suspended = {
                        let mut state = self.state.lock().map_err(|_| broken())?;
                        state.clipboard_error = None;
                        state.suspended
                    };
                    if suspended {
                        if let Err(error) = watch.stop().await {
                            self.state.lock().map_err(|_| broken())?.clipboard_error = Some(error.clone());
                            return Err(error);
                        }
                    } else { *slot = Some(watch); }
                }
                Err(error) => {
                    let mut state = self.state.lock().map_err(|_| broken())?;
                    // A sleep cancellation is expected lifecycle work, rather
                    // than a new clipboard-permission failure.
                    if state.suspended && matches!(error, Error::Cancelled) { return Ok(()); }
                    state.clipboard_error = Some(error.clone()); return Err(error);
                }
            }
        }
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<()> {
        { let mut closing = self.admission.lock().map_err(|_| broken())?; *closing = true; self.tasks.close(); }
        let initialization = self.state.lock().map_err(|_| broken())?.clipboard_startup.take();
        if let Some(initialization) = initialization { initialization.cancel(); }
        self.tasks.wait().await;
        let source = self.state.lock().map_err(|_| broken())?.source.take();
        let capture = async { match self.watch.lock().await.take() { Some(watch) => watch.stop().await, None => Ok(()) } };
        let source = async { match source { Some(source) => source.shutdown(&self.runtime).await.map_err(platform), None => Ok(()) } };
        let (capture, source) = tokio::join!(capture, source);
        let errors = [capture, source].into_iter().filter_map(Result::err).map(|error| error.to_string()).collect::<Vec<_>>();
        if errors.is_empty() { Ok(()) } else { Err(Error::Platform(errors.join("；").into())) }
    }
}
fn broken() -> Error { Error::Task("系统接入状态锁损坏".into()) }
fn platform(error: String) -> Error { Error::Platform(error.into()) }
