pub mod geometry;
pub mod configuration;
pub mod draft;
pub mod dialogs;
pub mod forms;
pub mod palette;
pub(crate) mod rasters;
pub(crate) mod overlay;
pub mod settings;
pub mod theme;
pub mod widgets;
pub(crate) mod text;

use gpui_kit::{prelude::*, *};
use gpui_kit::component::{WindowExt, dialog::DialogButtonProps};

pub fn icon(name: &'static str, size: f32) -> Svg {
    svg().path(format!("superpaste/icons/mdi/{name}.svg")).size(px(size))
}

pub struct ErrorView;
impl ErrorView {
    pub fn new(title: &'static str, message: SharedString, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("Super Paste");
        if cfg!(target_os = "macos") {
            cx.set_activation_policy(ActivationPolicy::Regular);
            cx.activate(true);
        }
        window.on_window_should_close(cx, |_, cx| { Self::quit(cx); false });
        // Kit installs the Root after building this host. Open the alert only
        // after that Root is available, using its normal modal and focus flow.
        cx.defer_in(window, move |_, window, cx| {
            window.open_alert_dialog(cx, move |dialog, _, _| {
                dialog.title(title).child(message.clone())
                    .button_props(DialogButtonProps::default().ok_text("退出"))
                    .on_close(|_, _, cx| Self::quit(cx))
            });
        });
        Self
    }

    fn quit(cx: &mut App) {
        let desktop = cx.global::<crate::host::DesktopGlobal>().0.clone();
        desktop.update(cx, |desktop, cx| desktop.quit(cx));
    }
}
impl Render for ErrorView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}
