use serde::Deserialize;
use super_paste_core::{Error, Result};
use tokio::runtime::Handle;

/// Desktop coordinates, including negative origins on secondary displays.
/// X11 snapshots explicitly mark physical root coordinates with `x11_root`;
/// all other snapshots use logical coordinates.
#[derive(Clone, Debug, Deserialize)]
pub struct Area {
    pub display_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Deserialize)]
pub struct WorkAreaSnapshot {
    pub cursor_area: Area,
    pub areas: Vec<Area>,
    /// X11 reports physical root coordinates. The GPUI host converts them
    /// using this root's current logical size, rather than assuming 1x DPI.
    #[serde(skip)]
    pub x11_root: Option<X11Root>,
}
#[derive(Clone, Debug)]
pub struct X11Root { pub index: u32, pub width: u16, pub height: u16 }
impl Area {
    pub fn validate(&self) -> Result<()> {
        if self.display_id.is_empty() || self.width <= 0.0 || self.height <= 0.0
            || [self.x, self.y, self.width, self.height].into_iter()
                .any(|value| !value.is_finite() || value.abs() > f32::MAX as f64)
        {
            return Err(Error::Platform("显示器工作区无效".into()));
        }
        Ok(())
    }
}

/// KWin and X11 provide panel-aware work areas without blocking the UI thread.
/// Windows/macOS obtain the cursor display and usable bounds from GPUI.
pub async fn work_area_snapshot(runtime: &Handle) -> Result<Option<WorkAreaSnapshot>> {
    #[cfg(target_os = "linux")]
    if clipboard_core::is_wayland_session()
        && std::env::var("KDE_SESSION_VERSION").as_deref() == Ok("6")
    {
        let snapshot = runtime.spawn_blocking(crate::kwin::work_area_snapshot).await?
            .map_err(|error| Error::Platform(error.to_string().into()))?;
        snapshot.cursor_area.validate()?;
        for area in &snapshot.areas { area.validate()?; }
        if !snapshot.areas.iter().any(|area| area.display_id == snapshot.cursor_area.display_id) {
            return Err(Error::Platform("光标所在显示器已不可用".into()));
        }
        return Ok(Some(snapshot));
    }
    #[cfg(target_os = "linux")]
    if !clipboard_core::is_wayland_session() {
        return Ok(Some(runtime.spawn_blocking(crate::geometry_x11::snapshot).await??));
    }
    let _ = runtime;
    Ok(None)
}
