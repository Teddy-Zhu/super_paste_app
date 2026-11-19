//! One native tray, one latest-state worker, and typed actions back to the host.
use std::{panic::AssertUnwindSafe, sync::Arc};
use futures_util::FutureExt;
use super_paste_core::{Error, Result};
use super_paste_format::{ColorMode, Preferences};
use tokio::{runtime::Handle, sync::watch};
use tokio_util::sync::CancellationToken;
use crate::main_thread::MainThread;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::Backend;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod native;
#[cfg(any(target_os = "windows", target_os = "macos"))]
use native::Backend;

const TITLE: &str = "Super Paste";
const TOOLTIP: &str = "Super Paste - 剪贴板管理工具";
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TrayState { pub color_mode: ColorMode, pub auto_start: bool, pub enable_local_ocr: bool }
impl From<&Preferences> for TrayState {
    fn from(value: &Preferences) -> Self {
        Self { color_mode: value.color_mode, auto_start: value.auto_start, enable_local_ocr: value.enable_local_ocr }
    }
}
#[derive(Clone)]
pub enum TrayAction {
    ShowSettings { activation_token: Option<String> },
    ToggleOverlay { activation_token: Option<String> },
    SetColorMode(ColorMode), ToggleAutoStart, ToggleLocalOcr, Restart, Quit,
}
pub type ActionHandler = Arc<dyn Fn(TrayAction) + Send + Sync>;
struct Icon { rgba: Vec<u8>, width: u32, height: u32 }

pub struct TrayService {
    state: watch::Sender<TrayState>,
    stop: CancellationToken,
    completion: watch::Receiver<Option<Result<()>>>,
}
impl TrayService {
    pub async fn start(runtime: &Handle, main: MainThread, initial: TrayState, action: ActionHandler) -> Result<Arc<Self>> {
        // This immutable application asset is decoded once, outside both UI and menu callbacks.
        let icon = runtime.spawn_blocking(|| -> Result<Icon> {
            let image = image::load_from_memory_with_format(include_bytes!("../../../../resources/app-icon.png"), image::ImageFormat::Png)?
                .resize_exact(32, 32, image::imageops::FilterType::Lanczos3).into_rgba8();
            Ok(Icon { width: image.width(), height: image.height(), rgba: image.into_raw() })
        }).await??;
        let mut backend = Backend::create(main, initial, icon, action).await?;
        let (state, mut updates) = watch::channel(initial);
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let (finished, completion) = watch::channel(None);
        let service = Arc::new(Self { state, stop, completion });
        runtime.spawn(async move {
            let outcome = AssertUnwindSafe(async {
                loop {
                    tokio::select! {
                        biased;
                        _ = cancelled.cancelled() => break,
                        changed = updates.changed() => {
                            if changed.is_err() { break; }
                            let state = *updates.borrow_and_update();
                            backend.refresh(state).await?;
                        }
                    }
                }
                Ok(())
            }).catch_unwind().await.unwrap_or_else(|_| Err(Error::Task("托盘后台任务发生异常".into())));
            // Even a failed update must release the native icon and its callbacks.
            let released = backend.shutdown().await;
            let result = match (outcome, released) {
                (Err(first), Err(second)) => Err(Error::Platform(format!("{first}；释放托盘失败：{second}").into())),
                (Err(error), _) | (_, Err(error)) => Err(error),
                _ => Ok(()),
            };
            if let Err(error) = &result { tracing::error!(%error, "托盘服务停止"); }
            finished.send_replace(Some(result));
        });
        Ok(service)
    }
    /// Ordinary projection changes coalesce and ignore unrelated configuration fields.
    pub fn update(&self, state: TrayState) -> Result<()> {
        if self.state.is_closed() { return Err(Error::Platform("托盘服务已停止".into())); }
        self.state.send_if_modified(|current| { if *current == state { false } else { *current = state; true } });
        Ok(())
    }
    /// Native check items may toggle themselves before a failed/no-op application mutation.
    pub fn refresh(&self, state: TrayState) -> Result<()> {
        self.state.send(state).map_err(|_| Error::Platform("托盘服务已停止".into()))
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.stop.cancel();
        let mut completion = self.completion.clone();
        loop {
            if let Some(result) = completion.borrow().clone() { return result; }
            completion.changed().await.map_err(|_| Error::Task("托盘结束状态丢失".into()))?;
        }
    }
}
impl Drop for TrayService { fn drop(&mut self) { self.stop.cancel(); } }
