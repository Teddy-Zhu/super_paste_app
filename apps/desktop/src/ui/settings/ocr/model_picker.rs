use std::{cell::Cell, rc::Rc, sync::Arc};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Align, Button, Combobox, Placement, Positioner};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use super_paste_format::AiModelOption;
use crate::ui::{icon, palette::Palette};

pub struct ModelEdited(pub String);
/// The editable value and the unfiltered server collection have separate owners.
/// Only visible options are laid out; typing never copies or filters that collection.
pub struct ModelPicker {
    input: Entity<InputState>, options: Arc<Vec<AiModelOption>>, list: ListState,
    open: bool, active: Option<usize>, disabled: bool, palette: Palette, popup_height: Pixels,
    bounds: Rc<Cell<Bounds<Pixels>>>, _subscription: Subscription, _blur: Subscription,
}
impl EventEmitter<ModelEdited> for ModelPicker {}
impl ModelPicker {
    pub fn new(value: String, palette: Palette, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder("选择或输入模型名称、模型 ID"));
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            match event {
                InputEvent::Change => { cx.emit(ModelEdited(input.read(cx).value().to_string())); this.active = this.selected(cx); }
                InputEvent::Blur => { if !input.read(cx).focus_handle(cx).contains_focused(window, cx) { this.open = false; } }
                _ => {},
            }
            cx.notify();
        });
        let focus = input.read(cx).focus_handle(cx);
        let blur = cx.on_focus_out(&focus, window, |this, _, _, cx| { this.open = false; cx.notify(); });
        Self { input, options: Arc::new(Vec::new()), list: ListState::new(0, ListAlignment::Top, px(64.0)), open: false, active: None, disabled: false, popup_height: px(8.0),
            palette, bounds: Rc::new(Cell::new(Bounds::default())), _subscription: subscription, _blur: blur }
    }
    pub fn value(&self, cx: &App) -> SharedString { self.input.read(cx).value() }
    pub fn set_value(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).value().as_ref() != value { self.input.update(cx, |input, cx| input.set_value(value.to_owned(), window, cx)); }
    }
    pub fn present(&mut self, disabled: bool, palette: Palette) { self.disabled = disabled; self.palette = palette; if disabled { self.open = false; } }
    pub fn set_options(&mut self, options: Arc<Vec<AiModelOption>>, reveal: bool, cx: &mut Context<Self>) {
        let mut height: f32 = 10.0;
        for item in options.iter().take(8) { height += if item.name == item.id { 32.2 } else { 51.0 }; if height >= 240.0 { break; } }
        self.popup_height = px(height.min(240.0));
        self.options = options;
        // At least eight rows already fill the capped popup. Only a small
        // collection needs complete measurements to determine its natural height.
        self.list = ListState::new(self.options.len(), ListAlignment::Top, px(64.0));
        if self.options.len() < 8 { self.list = self.list.clone().measure_all(); }
        self.active = self.selected(cx);
        self.open = reveal && !self.options.is_empty();
        if self.open {
            self.active = self.active.or(Some(0));
            if let Some(active) = self.active { self.list.scroll_to_reveal_item(active); }
        }
        // A server reply reveals options without moving focus away from the
        // request button or another control the user is currently editing.
        cx.notify();
    }
    fn selected(&self, cx: &App) -> Option<usize> { let value = self.input.read(cx).value(); self.options.iter().position(|item| item.id == value.as_ref()) }
    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled { self.open = false; cx.notify(); return; }
        self.open = open && !self.disabled && !self.options.is_empty();
        if self.open {
            self.active = self.selected(cx).or(Some(0));
            if let Some(active) = self.active { self.list.scroll_to_reveal_item(active); }
        }
        self.input.read(cx).focus_handle(cx).focus(window, cx); cx.notify();
    }
    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled { return; }
        let Some(value) = self.options.get(index).map(|item| item.id.clone()) else { return; };
        self.open = false; self.active = Some(index);
        self.input.update(cx, |input, cx| input.set_value(value, window, cx));
        self.input.read(cx).focus_handle(cx).focus(window, cx); cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.input.read(cx).is_composing() { return; }
        let key = event.keystroke.key.as_str();
        if key == "tab" { self.open = false; cx.notify(); return; }
        if key == "escape" && self.open { self.open = false; cx.stop_propagation(); cx.notify(); return; }
        if matches!(key, "up" | "down") && !self.options.is_empty() {
            if !self.open { self.set_open(true, window, cx); }
            else {
                let count = self.options.len(); let current = self.active.unwrap_or(0);
                self.active = Some(if key == "down" { (current + 1) % count } else { (current + count - 1) % count });
                if let Some(active) = self.active { self.list.scroll_to_reveal_item(active); }
                cx.notify();
            }
            cx.stop_propagation();
        } else if key == "enter" && self.open {
            if let Some(active) = self.active { self.choose(active, window, cx); }
            cx.stop_propagation();
        }
    }
}
impl Render for ModelPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette; let bounds = self.bounds.clone(); let weak = cx.entity().downgrade(); let toggle = weak.clone();
        let resize = weak.clone();
        let input_focus = self.input.read(cx).focus_handle(cx);
        let focused = input_focus.contains_focused(window, cx);
        let mut root = super::render::field_frame(self.input.entity_id(), self.disabled, focused, p, window, cx).relative()
            .capture_key_down(cx.listener(Self::key))
            .on_click(cx.listener(|this, _, window, cx| { if !this.open { this.set_open(true, window, cx); } }))
            .child(canvas(move |rect, _, cx| {
                let previous = bounds.replace(rect);
                if previous.size.width != rect.size.width {
                    let owner = resize.clone();
                    cx.defer(move |cx| { let _ = owner.update(cx, |this, cx| { if this.open && this.bounds.get().size.width == rect.size.width { cx.notify(); } }); });
                }
            }, |_, _, _, _| {}).absolute().inset_0())
            .child(Input::new(&self.input).text_direction(gpui::TextDirection::Ltr).aria_label("模型名称").disabled(self.disabled).appearance(false).focus_bordered(false)
                .w_full().h_auto().min_h(px(28.0)).pl(px(9.0)).pr(px(36.0)).py(px(5.0))
                .text_size(px(13.0)).line_height(relative(1.4)).bg(transparent_black()))
            .child(Button::new("ocr-model-combo-toggle").absolute().right(px(1.0)).top(px(1.0)).bottom(px(1.0)).w(px(30.0))
                .disabled(self.disabled || self.options.is_empty()).accessibility_label(if self.open { "收起模型列表" } else { "展开模型列表" })
                .text_color(p.text_secondary).child(icon("chevron-down", 16.0))
                .on_click(move |_, window, cx| { cx.stop_propagation(); let _ = toggle.update(cx, |this, cx| this.set_open(!this.open, window, cx)); }));
        if self.open {
            let popup_weak = weak.clone(); let row_weak = weak.clone(); let rows = self.options.clone(); let active = self.active;
            let measurement_owner = weak.clone(); let measurement_rows = self.options.clone(); let measured = self.list.clone(); let previous_height = self.popup_height;
            let selected = self.input.read(cx).value(); let rect = self.bounds.get();
            let popup = div().id("ocr-model-options").role(Role::ListBox).occlude().relative().w(rect.size.width).h(self.popup_height).p(px(4.0))
                .bg(p.bg_elevated).border_1().border_color(p.control_border).rounded(px(4.0))
                .shadow(vec![BoxShadow { color: black().opacity(if p.dark { 0.35 } else { 0.12 }), offset: point(px(0.0), px(8.0)), blur_radius: px(24.0), spread_radius: px(0.0), inset: false }])
                .child(list(self.list.clone(), move |index, _, _| {
                    let item = &rows[index]; let choose = row_weak.clone(); let hover = row_weak.clone();
                    div().id(("ocr-model-option", index)).role(Role::ListBoxOption).aria_selected(item.id == selected.as_ref())
                        .w_full().py(px(7.0)).px(px(9.0)).rounded(px(4.0)).cursor_pointer().flex().flex_col().gap(px(2.0))
                        .bg(if active == Some(index) { p.bg_secondary } else { p.bg_elevated })
                        .text_size(px(13.0)).line_height(relative(1.4)).text_color(if item.id == selected.as_ref() { p.primary } else { p.text_primary })
                        .child(item.name.clone()).when(item.name != item.id, |row| row.child(div().text_size(px(12.0)).line_height(relative(1.4)).text_color(p.text_secondary).child(item.id.clone())))
                        .on_hover(move |over, _, cx| { if *over { let _ = hover.update(cx, |this, cx| { this.active = Some(index); cx.notify(); }); } })
                        .on_click(move |_, window, cx| { cx.stop_propagation(); let _ = choose.update(cx, |this, cx| this.choose(index, window, cx)); })
                        .into_any_element()
                }).size_full())
                .child(canvas(|_, _, _| (), move |_, _, _, cx| {
                    let Some(height) = measured.measured_content_height() else { return; };
                    let height = (height + px(10.0)).min(px(240.0));
                    if height == previous_height || measurement_rows.len() >= 8 { return; }
                    let owner = measurement_owner.clone(); let rows = measurement_rows.clone();
                    cx.defer(move |cx| { let _ = owner.update(cx, |this, cx| {
                        if this.open && Arc::ptr_eq(&this.options, &rows) && this.bounds.get().size.width == rect.size.width && this.popup_height != height {
                            this.popup_height = height; cx.notify();
                        }
                    }); });
                }).absolute().inset_0())
                .on_mouse_down_out(move |event, _, cx| { if !rect.contains(&event.position) { let _ = popup_weak.update(cx, |this, cx| { this.open = false; cx.notify(); }); } });
            root = root.child(deferred(Positioner::side(rect).placement(Placement::Bottom).align(Align::Start).offset(px(4.0)).margin(px(8.0)).child(popup)).with_priority(gpui_kit::base::POPUP_PRIORITY));
        }
        Combobox::new("ocr-model-combo").open(self.open).disabled(self.disabled).focus_handle(&input_focus)
            .on_open_change(move |open, window, cx| { let _ = weak.update(cx, |this, cx| this.set_open(open, window, cx)); })
            .child(root).w_full()
    }
}
