use gpui_kit::{prelude::*, *};
use gpui_kit::base::{PremultipliedColor, input::Enter};
use gpui_kit::component::{WindowExt, dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState, Textarea, TextareaState}, DialogHandle};
use super_paste_core::Error;
use super_paste_format::{Tag, TagId};
use crate::ui::{palette::Palette, settings::PaneContext, widgets::{ColorPicker, ColorPickerState, ColorSwatch, motion}};

const COLORS: [u32; 10] = [0x3b82f6, 0x10b981, 0x8b5cf6, 0xf59e0b, 0xef4444, 0x06b6d4, 0x84cc16, 0xf97316, 0xec4899, 0x6366f1];
struct TagEditor {
    context: PaneContext, owner: WeakEntity<super::TagsPane>, id: Option<TagId>, name: Entity<InputState>, description: Entity<TextareaState>,
    picker: Entity<ColorPickerState>, saving: bool, open: bool, error: Option<SharedString>, invalid_name: bool,
    dialog: Option<DialogHandle>, subscriptions: Vec<Subscription>,
}
pub fn open(context: PaneContext, tag: Option<Tag>, owner: WeakEntity<super::TagsPane>, window: &mut Window, cx: &mut App) {
    let editing = tag.is_some();
    let editor = cx.new(|cx| TagEditor::new(context, tag, owner, window, cx));
    let builder_editor = editor.clone();
    let handle = window.open_owned_alert_dialog(cx, move |dialog, _, cx| {
        let editor = builder_editor.clone(); let close = editor.downgrade();
        let saving = editor.read(cx).saving;
        dialog.title(if editing { "编辑标签" } else { "新建标签" }).child(editor.clone())
            .button_props(DialogButtonProps::default().show_cancel(true).cancel_text("取消")
                .ok_text(if saving { "正在保存…" } else if editing { "保存" } else { "创建" })
                .on_ok(move |_, window, cx| { editor.update(cx, |this, cx| this.save(window, cx)); false }))
            .on_close(move |_, window, cx| { let _ = close.update(cx, |this, cx| {
                this.open = false;
                this.picker.update(cx, |picker, cx| picker.close(false, window, cx));
            }); })
    });
    editor.update(cx, |editor, _| editor.dialog = Some(handle));
}
impl TagEditor {
    fn new(context: PaneContext, tag: Option<Tag>, owner: WeakEntity<super::TagsPane>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let color = tag.as_ref().map(|tag| tag.color.to_ascii_lowercase()).unwrap_or_else(|| "#3b82f6".into());
        let name = cx.new(|cx| InputState::new(window, cx).max_utf16_length(20).placeholder("标签名称").default_value(tag.as_ref().map(|tag| tag.name.to_string()).unwrap_or_default()));
        let description = cx.new(|cx| TextareaState::new(window, cx).normalize_line_endings().max_utf16_length(100).rows(2).placeholder("添加简短描述").default_value(tag.as_ref().and_then(|tag| tag.description.as_deref()).unwrap_or("").to_owned()));
        let picker = cx.new(|cx| ColorPickerState::new(&color, window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::Change => { this.invalid_name = false; cx.notify(); }
                    InputEvent::PressEnter { .. } => this.save(window, cx),
                    InputEvent::Focus | InputEvent::Blur => cx.notify(),
                }
            }),
            cx.subscribe_in(&description, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Focus | InputEvent::Blur) { cx.notify(); }
            }),
            cx.observe(&picker, |_, _, cx| cx.notify()),
            cx.observe(&context.configuration, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        ];
        Self { context, owner, id: tag.as_ref().map(|tag| tag.id), name, description, picker, saving: false, open: true, error: None, invalid_name: false, dialog: None, subscriptions }
    }
    fn select_color(&mut self, color: u32, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving { return; }
        self.picker.update(cx, |picker, cx| picker.set_value(color, window, cx));
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || !self.open || self.is_composing(cx) || self.context.services.is_shutting_down() { return; }
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.invalid_name = true; self.error = Some("请输入标签名称".into());
            self.name.read(cx).focus_handle(cx).focus(window, cx); cx.notify(); return;
        }
        let description = self.description.read(cx).value().trim().to_owned();
        let description = (!description.is_empty()).then_some(description);
        self.picker.update(cx, |picker, cx| picker.close(false, window, cx));
        let color = self.picker.read(cx).hex().to_string(); let id = self.id;
        self.saving = true; self.error = None; cx.notify(); window.refresh();
        let services = self.context.services.clone(); let tags = services.tags.clone();
        let task = services.runtime.spawn(async move {
            match id { Some(id) => tags.update(id, name, color, description).await, None => tags.create(name, color, description).await.map(|_| ()) }
        });
        let owner_window = window.window_handle();
        let tag_owner = self.owner.clone();
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            if result.is_ok() {
                let _ = tag_owner.update(cx, |owner, cx| { owner.operation_error = None; cx.notify(); });
            }
            let failure = result.as_ref().err().cloned();
            let handled = weak.update_in(cx, |this, window, cx| {
                if !this.open { return false; }
                this.saving = false;
                match result {
                    Ok(()) => {
                        this.open = false; if let Some(dialog) = this.dialog.take() { dialog.close(window, cx); }
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                window.refresh(); cx.notify();
                true
            }).unwrap_or(false);
            if !handled && !services.is_shutting_down() { if let Some(error) = failure {
                let _ = owner_window.update(cx, |_, window, cx| crate::ui::dialogs::message(format!("保存标签失败：{error}"), window, cx));
            } }
        }).detach();
    }
    fn is_composing(&self, cx: &App) -> bool {
        self.name.read(cx).is_composing() || self.description.read(cx).is_composing() || self.picker.read(cx).is_composing(cx)
    }
    fn save_shortcut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_composing(cx) { return; }
        window.prevent_default(); cx.stop_propagation();
        self.save(window, cx);
    }
}
impl Render for TagEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let id: ElementId = ("tag-editor", cx.entity_id()).into();
        let name_focused = self.name.read(cx).focus_handle(cx).is_focused(window);
        let description_focused = self.description.read(cx).focus_handle(cx).is_focused(window);
        let name_border = motion(&id, "name-border", PremultipliedColor((if self.invalid_name { p.danger } else if name_focused { p.primary } else { p.control_border }).into()), 100, window, cx).0;
        let description_border = motion(&id, "description-border", PremultipliedColor((if description_focused { p.primary } else { p.control_border }).into()), 100, window, cx).0;
        let background = motion(&id, "field-background", PremultipliedColor((if self.saving { p.bg_secondary } else { p.bg_primary }).into()), 100, window, cx).0;
        // Input::h configures multi-line input only. Set the native frame's
        // height through Styled so the single-line control is 18.2 + 14 + 2px.
        let name = Styled::h(field_frame(Input::new(&self.name).text_direction(gpui::TextDirection::Ltr).aria_label("名称").aria_invalid(self.invalid_name)
            .when_some(self.error.clone(), |input, error| input.aria_description(error))
            .appearance(false).focus_bordered(false).disabled(self.saving).dim_disabled_content(false),
            name_border, background, p, self.saving), px(34.2));
        let description = field_frame(Textarea::new(&self.description).text_direction(gpui::TextDirection::Ltr).aria_label("描述").appearance(false)
            .disabled(self.saving).dim_disabled_content(false).content_padding(Edges::default()).h(px(52.4)),
            description_border, background, p, self.saving);
        let selected_color = self.picker.read(cx).value();
        let colors = COLORS.into_iter().enumerate().map(|(index, color)| {
            let selected = selected_color == Some(color);
            let weak = cx.entity().downgrade();
            ColorSwatch::new(("tag-preset-color", index), color, selected, p).disabled(self.saving)
                .on_click(move |_, window, cx| { let _ = weak.update(cx, |this, cx| this.select_color(color, window, cx)); })
        }).collect::<Vec<_>>();
        div().id("tag-editor-fields").role(Role::Group).aria_busy(self.saving)
            .flex().flex_col().gap(px(18.0)).text_size(px(12.0)).line_height(relative(1.4)).font_weight(FontWeight::NORMAL)
            // Input actions run before raw key listeners. Capture the native
            // secondary Enter so a textarea does not insert a newline first.
            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                if action.secondary { this.save_shortcut(window, cx); }
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" && (event.keystroke.modifiers.control || event.keystroke.modifiers.platform) {
                    this.save_shortcut(window, cx);
                }
            }))
            .child(div().flex().flex_col().gap(px(7.0)).font_weight(FontWeight::MEDIUM)
                .child(div().id("name-label").on_click(cx.listener(|this, _, window, cx| {
                    if !this.saving { this.name.read(cx).focus_handle(cx).focus(window, cx); }
                })).child("名称"))
                .child(name))
            .child(div().flex().flex_col().gap(px(7.0)).font_weight(FontWeight::MEDIUM).child("颜色")
                .child(div().id("tag-color-options").role(Role::Group).aria_label("颜色").flex().flex_wrap().items_center().gap(px(8.0)).children(colors)
                    .child(ColorPicker::new(&self.picker, p).disabled(self.saving))))
            .child(div().flex().flex_col().gap(px(7.0)).font_weight(FontWeight::MEDIUM)
                .child(div().id("description-label").flex().items_baseline().gap(px(4.0))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if !this.saving { this.description.read(cx).focus_handle(cx).focus(window, cx); }
                    })).child("描述").child(div().text_size(px(11.0)).font_weight(FontWeight::NORMAL).text_color(p.text_tertiary).child("可选")))
                .child(description))
            .when_some(self.error.clone(), |form, error| form.child(div().id("tag-editor-error").role(Role::Alert).text_size(px(12.0)).text_color(p.danger).child(error)))
    }
}

fn field_frame<T: Styled>(field: T, border: Rgba, background: Rgba, p: Palette, disabled: bool) -> T {
    field.w_full().min_w_0().px(px(9.0)).py(px(7.0)).border_1().border_color(border).rounded(px(4.0)).bg(background)
        .text_size(px(13.0)).line_height(relative(1.4)).font_weight(FontWeight::MEDIUM)
        .text_color(if disabled { p.text_tertiary } else { p.text_primary }).opacity(if disabled { 0.5 } else { 1.0 })
}
