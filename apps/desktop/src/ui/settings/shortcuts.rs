use std::{rc::Rc, sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::{WindowExt, input::{Input, InputState}};
use super_paste_core::{Error, Result};
use super_paste_format::{ConfigChange, ShortcutState, ShortcutStatus};
use super_paste_platform::{access::AccessStatus, shortcut::ActivationSuppression};
use uuid::Uuid;
use super::PaneContext;
use crate::ui::{configuration::Feedback, dialogs, forms, palette::Palette, widgets::TextButton};

struct Detected { physical: NativeKeyCode, accelerator: String }
struct Pending { mutation: Uuid, session: u64, edit: u64 }
pub struct ShortcutsPane {
    context: PaneContext, client: Uuid, _client_lifetime: Rc<()>, field: Entity<InputState>,
    recording: bool, suppression: Option<ActivationSuppression>, modifiers: Modifiers,
    detected: Option<Detected>, preview: String, session: u64, edit: u64, pending: Option<Pending>,
    error: Option<SharedString>, error_timer: Option<Task<()>>,
    access: Result<AccessStatus>, shortcut: Arc<ShortcutStatus>, refresh_error: Option<SharedString>, refreshing: bool,
    poll: Option<Task<()>>, _shortcut_watch: Task<()>, _subscriptions: Vec<Subscription>,
}
impl ShortcutsPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let client = Uuid::new_v4();
        let configuration = context.configuration.clone();
        let lifetime = configuration.update(cx, |model, _| model.register_inline_client(client));
        let saved = display(&configuration.read(cx).projected.settings.global_shortcut);
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("点击设置快捷键").default_value(saved));
        let mut updates = context.services.shortcuts.subscribe();
        let shortcut = updates.borrow_and_update().status.clone();
        let watcher = cx.spawn(async move |weak, cx| {
            while updates.changed().await.is_ok() {
                let status = updates.borrow_and_update().status.clone();
                if weak.update(cx, |this, cx| { this.shortcut = status; cx.notify(); }).is_err() { break; }
            }
        });
        let subscriptions = vec![
            cx.observe_in(&configuration, window, |this, _, window, cx| this.synchronize(window, cx)),
            cx.subscribe_in(&configuration, window, |this, _, event: &Feedback, window, cx| this.acknowledge(event, window, cx)),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            cx.observe_window_visibility(window, |this, _, window, cx| {
                if !window.is_visible() { this.stop_recording(window, cx); this.poll = None; }
                else { this.access = this.context.services.access.status(); this.schedule_poll(window, cx); }
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && this.recording { this.stop_recording(window, cx); }
            }),
        ];
        let access = context.services.access.status();
        cx.defer_in(window, |this, window, cx| this.schedule_poll(window, cx));
        Self { context, client, _client_lifetime: lifetime, field, recording: false, suppression: None,
            modifiers: Modifiers::default(), detected: None, preview: String::new(), session: 0, edit: 0, pending: None,
            error: None, error_timer: None, access, shortcut, refresh_error: None, refreshing: false,
            poll: None, _shortcut_watch: watcher, _subscriptions: subscriptions }
    }
    fn synchronize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = if self.recording { display(&self.preview) } else { display(&self.context.configuration.read(cx).projected.settings.global_shortcut) };
        let placeholder = if self.recording { "请按下快捷键组合..." } else { "点击设置快捷键" };
        self.field.update(cx, |field, cx| {
            if field.value().as_ref() != value { field.set_value(value, window, cx); }
            field.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }
    fn start_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.context.services.shortcuts.suppress_activation() {
            Ok(guard) => self.suppression = Some(guard),
            Err(error) => { self.show_error(error.to_string(), false, cx); return; }
        }
        self.session = self.session.wrapping_add(1); self.edit = self.edit.wrapping_add(1);
        self.recording = true; self.detected = None; self.preview.clear(); self.error = None; self.error_timer = None;
        self.pending = None; self.modifiers = Modifiers::default();
        self.synchronize(window, cx); self.field.update(cx, |field, cx| field.focus(window, cx));
    }
    fn stop_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = false; self.suppression = None; self.detected = None; self.preview.clear();
        self.session = self.session.wrapping_add(1); self.pending = None; self.error = None; self.error_timer = None;
        self.synchronize(window, cx);
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.recording || window.has_active_dialog(cx) { return; }
        window.prevent_default(); cx.stop_propagation();
        if event.is_held || event.physical_key.is_none() { return; }
        self.edit = self.edit.wrapping_add(1); self.error = None; self.error_timer = None;
        self.modifiers = event.keystroke.modifiers;
        if event.keystroke.key == "backspace" { self.detected = None; self.preview.clear(); }
        else if let Some(physical) = event.physical_key {
            match accelerator(physical, self.modifiers, &event.keystroke) {
                Ok(value) => { self.preview = value.clone(); self.detected = Some(Detected { physical, accelerator: value }); }
                Err(error) => { self.detected = None; self.preview.clear(); self.show_error(error.to_string(), true, cx); }
            }
        }
        // IME text and binding replays carry no physical identity and never
        // become a substitute binding derived from the current text layout.
        self.synchronize(window, cx);
    }
    fn key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.recording || window.has_active_dialog(cx) { return; }
        window.prevent_default(); cx.stop_propagation();
        if !self.detected.as_ref().is_some_and(|detected| Some(detected.physical) == event.physical_key) { return; }
        let Some(detected) = self.detected.take() else { return; };
        if let Err(message) = validate(&detected.accelerator) {
            self.preview.clear(); self.show_error(message, true, cx); self.synchronize(window, cx); return;
        }
        self.submit(detected.accelerator, window, cx);
    }
    fn submit(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        match self.context.configuration.update(cx, |model, cx| model.submit(self.client, vec![ConfigChange::GlobalShortcut(value)], vec![], None, cx)) {
            Ok(mutation) => self.pending = Some(Pending { mutation, session: self.session, edit: self.edit }),
            Err(error) => self.show_error(error.to_string(), false, cx),
        }
        self.synchronize(window, cx);
    }
    fn acknowledge(&mut self, event: &Feedback, window: &mut Window, cx: &mut Context<Self>) {
        if event.client_id != self.client || !self.pending.as_ref().is_some_and(|pending| pending.mutation == event.mutation_id) { return; }
        let pending = self.pending.take().expect("matched pending shortcut mutation");
        // A response belongs to the submitted edit, including its error. New
        // physical input can supersede that edit before another key is released.
        match &event.result {
            Ok(_) if pending.session == self.session && pending.edit == self.edit => self.stop_recording(window, cx),
            Err(error) if pending.session == self.session && pending.edit == self.edit => self.show_error(error.to_string(), false, cx),
            _ => {},
        }
        self.access = self.context.services.access.status(); self.synchronize(window, cx);
    }
    fn show_error(&mut self, message: impl Into<SharedString>, temporary: bool, cx: &mut Context<Self>) {
        self.error = Some(message.into()); self.error_timer = None;
        if temporary {
            let timer = cx.background_executor().timer(Duration::from_secs(3));
            self.error_timer = Some(cx.spawn(async move |weak, cx| { timer.await; let _ = weak.update(cx, |this, cx| { this.error_timer = None; this.error = None; cx.notify(); }); }));
        }
        cx.notify();
    }
    fn schedule_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.poll.is_some() || !window.is_visible() { return; }
        let timer = cx.background_executor().timer(Duration::from_secs(3));
        self.poll = Some(cx.spawn_in(window, async move |weak, cx| {
            timer.await;
            let _ = weak.update_in(cx, |this, window, cx| { this.poll = None; this.access = this.context.services.access.status(); this.schedule_poll(window, cx); cx.notify(); });
        }));
    }
    fn refresh(&mut self, reconnect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.refreshing { return; }
        self.refresh_error = None;
        if !reconnect { self.access = self.context.services.access.status(); cx.notify(); return; }
        self.refreshing = true;
        let access = self.context.services.access.clone();
        let task = self.context.services.runtime.spawn(async move { access.retry().await });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, _, cx| { this.refreshing = false; this.refresh_error = result.err().map(|error| error.to_string().into()); this.access = this.context.services.access.status(); cx.notify(); });
        }).detach();
        cx.notify();
    }
}

impl Render for ShortcutsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let compact = window.viewport_size().width < px(900.0);
        let has_shortcut = !self.context.configuration.read(cx).projected.settings.global_shortcut.trim().is_empty();
        let status = self.shortcut.error.clone().unwrap_or_else(|| match self.shortcut.state {
            ShortcutState::PortalRequested => "已向桌面提交快捷键请求，请在系统授权窗口或系统快捷键设置中确认。",
            ShortcutState::Registered => "快捷键已注册",
            ShortcutState::Disabled => "尚未设置全局快捷键",
            ShortcutState::Failed => "全局快捷键不可用",
        }.into());
        let mut info = div().flex_1().min_w_0().text_size(px(12.0)).line_height(px(18.0)).text_color(p.text_secondary)
            .child(div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary).mb(px(4.0)).child("系统访问"))
            .child(div().text_color(if self.shortcut.error.is_some() { p.danger } else { p.text_secondary }).child(SharedString::from(status)));
        match &self.access {
            Ok(access) => {
                info = info.child(div().text_color(if access.clipboard_error.is_some() { p.danger } else { p.text_secondary })
                    .child(access.clipboard_error.as_ref().map(ToString::to_string).unwrap_or_else(|| if access.clipboard_listening { "剪贴板监听已启动" } else { "剪贴板监听未启动" }.to_owned())));
                let source_error = access.source_error.as_ref().map(ToString::to_string).or_else(|| access.source.as_ref().and_then(|source| source.error.clone()));
                info = info.child(div().text_color(if source_error.is_some() { p.danger } else { p.text_secondary }).child(source_error.unwrap_or_else(||
                    if access.source.as_ref().is_some_and(|source| source.available) { "活动窗口来源识别已启用" } else { "活动窗口来源识别不可用" }.to_owned())));
                if let Some(error) = access.source.as_ref().and_then(|source| source.listener_error.as_ref()) { info = info.child(div().text_color(p.danger).child(error.clone())); }
            }
            Err(error) => info = info.child(div().id("access-error").role(Role::Alert).text_color(p.danger).child(error.to_string())),
        }
        if self.context.services.shortcuts.uses_portal() {
            info = info.child(div().mt(px(4.0)).child("Wayland 的全局快捷键由桌面授权；后台剪贴板监听需要桌面提供相应协议。活动窗口来源识别支持 KDE Plasma 6，其他桌面暂不支持。"))
                .child(div().mt(px(4.0)).child("若桌面不支持全局快捷键授权，可在系统自定义快捷键中设置以下启动命令："));
            match std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).map(Ok).unwrap_or_else(std::env::current_exe) {
                Ok(path) => info = info.child(div().mt(px(4.0)).font_family("monospace").child(format!("\"{}\" --toggle-overlay", path.display()))),
                Err(error) => info = info.child(div().text_color(p.danger).child(format!("读取启动路径失败：{error}"))),
            }
        }
        if let Some(error) = &self.refresh_error { info = info.child(div().id("shortcut-refresh-error").role(Role::Alert).text_color(p.danger).child(error.clone())); }
        let weak = cx.entity().downgrade(); let retry = weak.clone(); let record = weak.clone(); let cancel = weak.clone(); let clear = weak;
        let mut buttons = div().flex().items_center().flex_wrap().gap(px(8.0));
        if self.recording {
            buttons = buttons.child(TextButton::new("shortcut-cancel", "取消", p).neutral().on_click(move |_, window, cx| { let _ = cancel.update(cx, |this, cx| this.stop_recording(window, cx)); }))
                .child(div().text_size(px(11.0)).text_color(p.text_secondary).child("按下快捷键组合，释放最后一个键完成设置"));
        } else {
            buttons = buttons.child(TextButton::new("shortcut-record", if has_shortcut { "重新设置" } else { "设置快捷键" }, p).primary().on_click(move |_, window, cx| { let _ = record.update(cx, |this, cx| this.start_recording(window, cx)); }));
            if has_shortcut { buttons = buttons.child(TextButton::new("shortcut-clear", "清除", p).danger().on_click(move |_, window, cx| {
                let clear = clear.clone(); dialogs::confirm("确定要清除当前快捷键设置吗？", window, cx, move |window, cx| { let _ = clear.update(cx, |this, cx| this.submit(String::new(), window, cx)); });
            })); }
        }
        let border = crate::ui::widgets::motion(&ElementId::Name("shortcut-recorder".into()), "border", if self.recording { p.primary } else { p.control_border }, 150, window, cx);
        let control = div().w(px(320.0)).max_w_full().flex().flex_col().gap(px(8.0))
            .child(div().border_1().border_color(border).rounded(px(6.0)).bg(if self.recording { p.selection } else { p.bg_primary })
                .child(Input::new(&self.field).text_direction(gpui::TextDirection::Ltr).readonly(true).appearance(false).bordered(false).aria_label("启动覆盖层快捷键").text_color(if self.recording { p.primary } else { p.text_primary })))
            .child(buttons)
            .when_some(self.error.clone(), |body, error| body.child(div().id("shortcut-error").role(Role::Alert).text_size(px(12.0)).text_color(p.danger).child(error)))
            .child(div().text_size(px(11.0)).line_height(px(18.0)).text_color(p.text_secondary).child("快捷键要求：")
                .child("• 最少2个键，最多3个键").child("• 必须包含至少一个修饰键（Ctrl、Cmd、Alt、Shift）").child("• 至少包含一个字母或数字键"));
        forms::page("shortcut-settings", window)
            .capture_key_down(cx.listener(Self::key_down)).capture_key_up(cx.listener(Self::key_up))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                this.modifiers = event.modifiers;
                if this.recording && this.detected.is_none() { this.preview = modifier_parts(event.modifiers).join("+"); this.synchronize(window, cx); }
            }))
            .child(forms::heading("快捷键设置", "自定义全局快捷键", p))
            .child(forms::settings_group(p)
                .child(forms::setting_row(p, false).items_start().when(compact, |row| row.flex_col().items_stretch()).child(info)
                    .child(div().flex().gap(px(8.0))
                        .child(TextButton::new("access-refresh", "刷新状态", p).neutral().disabled(self.refreshing).on_click(move |_, window, cx| { let _ = retry.update(cx, |this, cx| this.refresh(false, window, cx)); }))
                        .child(TextButton::new("access-reconnect", if self.refreshing { "连接中..." } else { "重新连接" }, p).primary().disabled(self.refreshing).on_click(cx.listener(|this, _, window, cx| this.refresh(true, window, cx))))))
                .child(forms::setting_row(p, true).items_start().when(compact, |row| row.flex_col().items_stretch())
                    .child(forms::setting_info("启动覆盖层快捷键", "设置打开覆盖层窗口的全局快捷键", p)).child(control)))
    }
}

fn modifier_parts(value: Modifiers) -> Vec<&'static str> {
    let mut parts = Vec::with_capacity(4);
    if value.control { parts.push("Ctrl"); } if value.shift { parts.push("Shift"); }
    if value.platform { parts.push(if cfg!(target_os = "macos") { "Command" } else { "Super" }); }
    if value.alt { parts.push("Alt"); } parts
}
fn accelerator(native: NativeKeyCode, modifiers: Modifiers, logical: &Keystroke) -> Result<String> {
    let mapping = match native {
        NativeKeyCode::Xkb(code) => keycode::KeyMapping::Xkb(u16::try_from(code).map_err(|_| Error::Invalid("物理按键代码超出范围".into()))?),
        NativeKeyCode::Windows(code) => keycode::KeyMapping::Win(code), NativeKeyCode::MacOS(code) => keycode::KeyMapping::Mac(code),
    };
    let code = keycode::KeyMap::try_from(mapping).ok().and_then(|key| key.code).ok_or_else(|| Error::Invalid("不支持该物理按键".into()))?.to_string();
    let token = if let Some(value) = code.strip_prefix("Key").or_else(|| code.strip_prefix("Digit")).or_else(|| code.strip_prefix("Arrow")).or_else(|| code.strip_prefix("Intl")) { value.to_owned() }
        else if let Some(value) = code.strip_prefix("Numpad") { if logical.key_char.as_ref().is_some_and(|text| text.chars().count() == 1) { format!("Num{value}") } else { logical.key.clone() } }
        else { match code.as_str() {
            "Backquote" => "`", "Backslash" => "\\", "BracketLeft" => "[", "BracketRight" => "]", "Comma" => ",", "Equal" => "=", "Minus" => "-", "Period" => ".", "Quote" => "'", "Semicolon" => ";", "Slash" => "/",
            "Escape" => "Esc", "Space" | "Tab" | "CapsLock" | "ContextMenu" | "Convert" | "Delete" | "End" | "Help" | "Home" | "PageDown" | "PageUp" | "PrintScreen" | "ScrollLock" | "Pause" | "Insert" | "Suspend" => &code,
            value if value.starts_with('F') && value[1..].parse::<u8>().is_ok() => &code,
            _ => return Err(Error::Invalid("不支持该物理按键".into())),
        }.to_owned() };
    let mut result = modifier_parts(modifiers).join("+"); if !result.is_empty() { result.push('+'); } result.push_str(&token); Ok(result)
}
fn validate(value: &str) -> std::result::Result<(), &'static str> {
    let parts: Vec<_> = value.split('+').filter(|part| !part.trim().is_empty()).collect();
    if parts.len() < 2 { return Err("至少需要2个键"); } if parts.len() > 3 { return Err("最多只能3个键"); }
    let modifier = |part: &&str| matches!(*part, "Ctrl" | "Command" | "Super" | "Alt" | "Shift");
    if !parts.iter().any(modifier) { return Err("必须包含至少一个修饰键（Ctrl、Cmd、Alt、Shift）"); }
    if parts.iter().all(modifier) { return Err("需要至少一个字母或数字键"); } Ok(())
}
fn display(value: &str) -> String {
    value.split('+').map(|key| match key {
        "CommandOrControl" => if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }, "Command" => "Cmd", "Super" => "Win", "Control" => "Ctrl",
        "Meta" => if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }, "Return" => "Enter", "Escape" => "Esc", "Delete" => "Del",
        "Up" => "↑", "Down" => "↓", "Left" => "←", "Right" => "→", _ => key,
    }).collect::<Vec<_>>().join("+")
}
