use gpui_kit::{prelude::*, *};
use super_paste_core::Error;
use super_paste_format::{ConfigChange, ConfigField};
use uuid::Uuid;
use super::PaneContext;
use crate::{host::DesktopGlobal, ui::{dialogs, forms, palette::Palette, widgets::{TextButton, Toggle}}};

pub struct DebugPane { context: PaneContext, client: Uuid, exporting: bool, _subscriptions: Vec<Subscription> }
impl DebugPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let configuration = context.configuration.clone();
        let subscriptions = vec![cx.observe(&configuration, |_, _, cx| cx.notify()), cx.observe_window_appearance(window, |_, _, cx| cx.notify())];
        Self { context, client: Uuid::new_v4(), exporting: false, _subscriptions: subscriptions }
    }
    fn toggle_listening(&mut self, value: bool, _window: &mut Window, cx: &mut Context<Self>) {
        if self.context.configuration.read(cx).is_pending(ConfigField::ClipboardListening) { return; }
        if let Err(error) = self.context.configuration.update(cx, |model, cx| model.submit(self.client, vec![ConfigChange::ClipboardListening(value)], vec![], None, cx)) {
            tracing::warn!(%error, "切换剪贴板监听状态失败");
        }
    }
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exporting || self.context.services.is_shutting_down() { return; }
        let directory = dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| self.context.services.paths.data.clone());
        let name = format!("super-paste-diagnostics-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let prompt = cx.prompt_for_new_path(&directory, Some(&name));
        let compositor = cx.compositor_name().to_owned(); let services = self.context.services.clone();
        let owner = window.window_handle();
        self.exporting = true; cx.notify();
        cx.spawn_in(window, async move |weak, cx| {
            let result = async {
                let path = prompt.await.map_err(|error| Error::Platform(format!("保存文件对话框已中断：{error}").into()))?
                    .map_err(|error| Error::Platform(format!("无法选择保存路径：{error}").into()))?;
                let Some(path) = path else { return Ok(None); };
                let task = services.export_diagnostics(path, compositor)?;
                task.await.map_err(Error::from)?.map(Some)
            }.await;
            let _ = weak.update(cx, |this, cx| {
                this.exporting = false;
                cx.notify();
            });
            if services.is_shutting_down() { return; }
            let message = match result {
                Ok(Some(path)) => Some(path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()),
                Ok(None) => None,
                Err(error) => Some(format!("导出失败: {error}")),
            };
            if let Some(message) = message { let _ = owner.update(cx, |_, window, cx| dialogs::message(message, window, cx)); }
        }).detach();
    }
}
fn open_overlay(cx: &mut App) { let desktop = cx.global::<DesktopGlobal>().0.clone(); desktop.update(cx, |desktop, cx| desktop.show_overlay(None, cx)); }
impl Render for DebugPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.context.configuration.read(cx);
        let settings = model.state.snapshot.value.settings.clone(); let pending = model.is_pending(ConfigField::ClipboardListening);
        let p = Palette::for_window(model.projected.settings.color_mode, window);
        let toggle = cx.entity().downgrade();
        // Capture runs before the button transfers focus and commits a blurred
        // input. The pointer's later click must not activate the overlay twice.
        let open = div().id("debug-overlay-pointer").capture_any_mouse_down(|event, _, cx| { if event.button == MouseButton::Left { open_overlay(cx); } })
            .child(TextButton::new("debug-open-overlay", "打开测试", p).on_click(|event, _, cx| { if !matches!(event, ClickEvent::Mouse(_)) { open_overlay(cx); } }));
        forms::page("debug-settings", window).child(forms::heading("调试设置", "用于开发和调试的额外选项", p))
            .child(forms::settings_group(p)
                .child(forms::setting_row(p, false).child(forms::setting_info("剪贴板监听", "控制是否监听剪贴板内容变化并保存历史记录", p))
                    .child(Toggle::new("debug-clipboard-listening", "剪贴板监听", settings.clipboard_listening, p).disabled(pending).on_change(move |value, window, cx| { let _ = toggle.update(cx, |this, cx| this.toggle_listening(value, window, cx)); })))
                .child(forms::setting_row(p, true).child(forms::setting_info("测试覆盖层", "打开覆盖层窗口进行测试", p)).child(open))
                .child(forms::setting_row(p, true).child(forms::setting_info("导出诊断信息", "导出应用运行状态和脱敏日志", p))
                    .child(TextButton::new("debug-export", if self.exporting { "导出中..." } else { "导出诊断信息" }, p).disabled(self.exporting).on_click(cx.listener(|this, _, window, cx| this.export(window, cx))))))
    }
}
