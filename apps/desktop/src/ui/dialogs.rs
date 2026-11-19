use std::rc::Rc;
use gpui_kit::{prelude::*, *};
use gpui_kit::component::{WindowExt, dialog::DialogButtonProps};

pub fn message(text: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
    let text = text.into();
    window.open_alert_dialog(cx, move |dialog, _, _| {
        dialog.title("Super Paste").child(text.clone())
            .button_props(DialogButtonProps::default().ok_text("确定"))
    });
}

pub fn confirm(text: impl Into<SharedString>, window: &mut Window, cx: &mut App,
    accept: impl Fn(&mut Window, &mut App) + 'static)
{
    let text = text.into();
    let accept = Rc::new(accept);
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let accept = accept.clone();
        dialog.title("Super Paste").child(text.clone())
            .button_props(DialogButtonProps::default().ok_text("确定").cancel_text("取消").show_cancel(true)
                .on_ok(move |_, window, cx| { accept(window, cx); true }))
    });
}
