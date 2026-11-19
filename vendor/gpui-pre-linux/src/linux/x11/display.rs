use anyhow::Context as _;
use uuid::Uuid;
use x11rb::{connection::Connection as _, xcb_ffi::XCBConnection};
use x11rb::protocol::xproto::ConnectionExt as _;

use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay, Size, px};

#[derive(Debug)]
pub(crate) struct X11Display {
    x_screen_index: usize,
    bounds: Bounds<Pixels>,
    uuid: Uuid,
}

impl X11Display {
    pub(crate) fn new(
        xcb: &XCBConnection,
        scale_factor: f32,
        x_screen_index: usize,
    ) -> anyhow::Result<Self> {
        let screen = xcb
            .setup()
            .roots
            .get(x_screen_index)
            .with_context(|| format!("No screen found with index {x_screen_index}"))?;
        // The connection setup is immutable. Query current dimensions after
        // RandR changes instead of retaining the size from application launch.
        let geometry = xcb.get_geometry(screen.root)?.reply()?;
        Ok(Self {
            x_screen_index,
            bounds: Bounds {
                origin: Default::default(),
                size: Size {
                    width: px(geometry.width as f32 / scale_factor),
                    height: px(geometry.height as f32 / scale_factor),
                },
            },
            uuid: Uuid::from_bytes([0; 16]),
        })
    }
}

impl PlatformDisplay for X11Display {
    // An X screen spans every monitor, so it has no single refresh rate.
    fn refresh_interval(&self) -> Option<std::time::Duration> {
        None
    }

    fn id(&self) -> DisplayId {
        DisplayId::new(self.x_screen_index as u64)
    }

    fn uuid(&self) -> anyhow::Result<Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }
}
