// Native counterpart of the reference DatePicker (MIT).
use std::{cell::Cell, rc::Rc};
use chrono::{Datelike, Days, Months, NaiveDate};
use gpui_kit::{prelude::*, *};
use gpui::accesskit;
use gpui_kit::base::{Button, ElementExt as _, PopoverState, Positioner, Scrollbar, ScrollbarMode};
use gpui_kit::component::{Disableable, input::{Input, InputEvent, InputState}};
use super::color_motion;
use crate::ui::{forms, icon, palette::Palette};

const INVALID_DATE: &str = "请输入有效日期（YYYY-MM-DD）";

pub struct DateInputChanged;

pub struct DateInputState {
    label: &'static str,
    palette: Option<Palette>,
    trigger_label: SharedString,
    input: Entity<InputState>,
    value: Option<NaiveDate>,
    month: NaiveDate,
    focused_day: NaiveDate,
    today: NaiveDate,
    group_focus: FocusHandle,
    trigger_focus: FocusHandle,
    day_focus: FocusHandle,
    popover: Entity<PopoverState>,
    open: bool,
    invalid: bool,
    disabled: bool,
    panel_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scroll: ScrollHandle,
    reveal: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DateInputChanged> for DateInputState {}

impl DateInputState {
    pub fn new(label: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).max_utf16_length(10).placeholder("YYYY-MM-DD"));
        let group_focus = cx.focus_handle();
        let day_focus = cx.focus_handle();
        let popover = cx.new(|cx| PopoverState::new(false, cx));
        let owner = cx.entity().downgrade();
        popover.update(cx, |state, _| {
            state.track_focus(Some(day_focus.clone()));
            state.set_on_open_change(Some(Rc::new(move |open, _, cx| {
                // Dismissal already owns the popover update. Do not reborrow it.
                if !open { let _ = owner.update(cx, |this, cx| this.finish_close(cx)); }
            })));
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::Change => {
                        if this.invalid { this.invalid = false; cx.notify(); }
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => this.commit(window, cx),
                    InputEvent::Focus => cx.notify(),
                }
            }),
            cx.on_focus_in(&group_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus_out(&group_focus, window, |this, _, window, cx| {
                if this.open && !this.group_focus.contains_focused(window, cx) && !this.day_focus.is_focused(window) { this.close(false, window, cx); }
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() { this.close(false, window, cx); }
            }),
            cx.observe_window_visibility(window, |this, visibility, window, cx| {
                if visibility == WindowVisibility::Hidden { this.close(false, window, cx); }
            }),
        ];
        let today = chrono::Local::now().date_naive();
        Self { label, palette: None, trigger_label: format!("{label}，选择日期").into(), input, value: None,
            month: today.with_day(1).expect("calendar first day"), focused_day: today, today,
            group_focus, trigger_focus: cx.focus_handle(), day_focus, popover,
            open: false, invalid: false, disabled: false, panel_bounds: Rc::new(Cell::new(None)),
            scroll: ScrollHandle::new(), reveal: Rc::new(Cell::new(false)), _subscriptions: subscriptions }
    }

    pub fn value(&self) -> Option<NaiveDate> { self.value }

    // Matches the reference's exact ten ASCII characters, including year bounds.
    fn parse(text: &str) -> Option<NaiveDate> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-'
            || !bytes.iter().enumerate().all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit()) {
            return None;
        }
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().filter(Self::in_range)
    }

    fn in_range(date: &NaiveDate) -> bool { (1..=9999).contains(&date.year()) }

    fn write_draft(&mut self, date: Option<NaiveDate>, window: &mut Window, cx: &mut Context<Self>) {
        self.invalid = false;
        let text = date.map(|date| date.format("%Y-%m-%d").to_string()).unwrap_or_default();
        if self.input.read(cx).value().as_ref() != text.as_str() {
            self.input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        cx.notify();
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.input.read(cx).is_composing() { return; }
        let draft = self.input.read(cx).value();
        let text = draft.trim();
        let date = if text.is_empty() { None } else { Self::parse(text) };
        if !text.is_empty() && date.is_none() {
            self.invalid = true;
            cx.notify();
            return;
        }
        self.write_draft(date, window, cx);
        if self.value != date { self.value = date; cx.emit(DateInputChanged); }
    }

    fn choose(&mut self, date: Option<NaiveDate>, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled { return; }
        self.write_draft(date, window, cx);
        if self.value != date { self.value = date; cx.emit(DateInputChanged); }
        self.close(true, window, cx);
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.open { return; }
        self.commit(window, cx);
        self.today = chrono::Local::now().date_naive();
        // Commit is the only draft-to-date boundary. Invalid drafts keep the
        // previous value, so opening never reparses or promotes an invalid edit.
        let base = self.value.unwrap_or(self.today);
        self.month = base.with_day(1).expect("calendar first day");
        self.focused_day = base;
        self.scroll.set_offset(point(px(0.0), px(0.0)));
        self.open = true;
        self.popover.update(cx, |state, cx| state.sync_open(true, window, cx));
        self.reveal_day();
        cx.notify();
    }

    fn finish_close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.panel_bounds.set(None);
        self.reveal.set(false);
        cx.notify();
    }

    fn close(&mut self, return_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Establish the explicit target first, so native dismissal does not
        // briefly restore the trigger before returning to the editable input.
        if return_focus && !self.disabled { self.input.read(cx).focus_handle(cx).focus(window, cx); }
        if self.open {
            self.finish_close(cx);
            self.popover.update(cx, |state, cx| state.sync_open(false, window, cx));
        }
    }

    /// Removing a custom-range field also drops its uncommitted draft, as the
    /// reference unmount does. The last committed range remains in the pane.
    pub fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open && !self.invalid {
            let value = self.value.map(|date| date.format("%Y-%m-%d").to_string()).unwrap_or_default();
            if self.input.read(cx).value().as_ref() == value.as_str() { return; }
        }
        self.close(false, window, cx);
        self.write_draft(self.value, window, cx);
    }

    fn shifted_month(&self, amount: i32) -> Option<NaiveDate> {
        let months = Months::new(amount.unsigned_abs());
        (if amount < 0 { self.month.checked_sub_months(months) } else { self.month.checked_add_months(months) }).filter(Self::in_range)
    }

    fn shift_month(&mut self, amount: i32, focus_day: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(month) = self.shifted_month(amount) else { return; };
        self.month = month;
        // Clamp the remembered day without changing selection.
        self.focused_day = (1..=self.focused_day.day()).rev().find_map(|day| month.with_day(day))
            .expect("calendar month includes its first day");
        if focus_day { self.day_focus.focus(window, cx); self.reveal_day(); }
        cx.notify();
    }

    fn reveal_day(&self) { self.reveal.set(true); }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.input.read(cx).is_composing() { return; }
        let input_focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let key = event.keystroke.key.as_str();
        let handled = if key == "escape" {
            if self.open { self.close(true, window, cx); }
            else if input_focused || self.trigger_focus.is_focused(window) { self.write_draft(self.value, window, cx); }
            true
        } else if input_focused || self.trigger_focus.is_focused(window) {
            if key == "down" { self.show(window, cx); true } else { false }
        } else if self.open && self.day_focus.is_focused(window) {
            let offset = match key {
                "left" => Some(-1), "right" => Some(1), "up" => Some(-7), "down" => Some(7),
                "home" => Some(-(self.focused_day.weekday().num_days_from_monday() as i64)),
                "end" => Some(6 - self.focused_day.weekday().num_days_from_monday() as i64),
                _ => None,
            };
            if let Some(offset) = offset {
                let days = Days::new(offset.unsigned_abs());
                let next = if offset < 0 { self.focused_day.checked_sub_days(days) } else { self.focused_day.checked_add_days(days) };
                if let Some(next) = next.filter(Self::in_range) {
                    self.focused_day = next;
                    self.month = next.with_day(1).expect("calendar first day");
                    self.reveal_day();
                    cx.notify();
                }
                true
            } else if key == "pageup" || key == "pagedown" {
                let amount = (if event.keystroke.modifiers.shift { 12 } else { 1 }) * (if key == "pageup" { -1 } else { 1 });
                self.shift_month(amount, true, window, cx);
                true
            } else { false }
        } else { false };
        if handled { window.prevent_default(); cx.stop_propagation(); }
    }
}

#[derive(IntoElement)]
pub struct DateInput { state: Entity<DateInputState>, palette: Palette, disabled: bool }

impl DateInput {
    pub fn new(state: &Entity<DateInputState>, palette: Palette) -> Self { Self { state: state.clone(), palette, disabled: false } }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
}

impl RenderOnce for DateInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { state, palette, disabled } = self;
        state.update(cx, |state, cx| {
            let changed = state.palette != Some(palette) || state.disabled != disabled;
            state.palette = Some(palette);
            state.disabled = disabled;
            if disabled { state.close(false, window, cx); }
            if changed { cx.notify(); }
        });
        state
    }
}

impl Render for DateInputState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Props are installed by DateInput before this entity joins the tree.
        let palette = self.palette.expect("date input render props");
        render_input(self, cx.entity(), palette, window, cx)
    }
}

fn render_input(state: &DateInputState, entity: Entity<DateInputState>, p: Palette, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let id: ElementId = ("date-input", entity.entity_id()).into();
    let focused = state.input.read(cx).focus_handle(cx).is_focused(window) || state.trigger_focus.is_focused(window);
    let border = color_motion(&id, "border", if state.invalid { p.danger } else if focused { p.primary } else { p.control_border }, window, cx);
    let toggle = entity.downgrade();
    let keys = entity.downgrade();
    let owner = entity.downgrade();
    let scroll_owner = owner.clone();
    let label = state.label;
    let popup = accesskit::HasPopup::Dialog;
    div().id(id).relative().track_focus(&state.group_focus).tab_stop(false)
        .flex().flex_wrap().items_center().w(px(156.0)).min_w_0().max_w_full().min_h(px(30.0))
        .overflow_hidden().border_1().border_color(border).rounded(px(4.0)).bg(p.bg_primary)
        .text_color(p.text_primary).line_height(relative(1.4)).opacity(if state.disabled { 0.5 } else { 1.0 })
        .capture_key_down(move |event, window, cx| { let _ = keys.update(cx, |state, cx| state.key_down(event, window, cx)); })
        .child(Input::new(&state.input).text_direction(gpui::TextDirection::Ltr).role(Role::ComboBox).aria_label(label).aria_expanded(state.open).aria_has_popup(popup)
            .aria_invalid(state.invalid).when(state.invalid, |input| input.aria_description(INVALID_DATE))
            .appearance(false).focus_bordered(false).disabled(state.disabled)
            .flex_1().w(px(124.0)).min_w(px(92.0)).max_w_full().h_auto().rounded_none()
            .px(px(8.0)).py(px(5.0)).text_size(px(12.0)).line_height(relative(1.4)).text_color(p.text_primary).font_features(forms::tabular()))
        .child(Button::new("calendar-trigger").track_focus(&state.trigger_focus).disabled(state.disabled)
            .accessibility_label(state.trigger_label.clone()).aria_expanded(state.open).aria_has_popup(popup)
            .flex_shrink_0().w(px(30.0)).h(px(28.0)).border_l_1().border_color(p.border_light)
            .text_color(p.text_secondary).opacity(if state.disabled { 0.5 } else { 1.0 })
            .when(!state.disabled, |button| button.cursor_pointer().hover(move |style| style.bg(p.selection).text_color(p.primary)))
            .when(state.disabled, |button| button.cursor_not_allowed())
            .child(icon("calendar-month-outline", 16.0))
            .on_click(move |_, window, cx| { let _ = toggle.update(cx, |state, cx| {
                if state.open { state.close(false, window, cx); } else { state.show(window, cx); }
            }); }))
        .when(state.invalid, |root| root.child(div().id("date-error").role(Role::Alert).w_full().max_w(px(180.0))
            .px(px(8.0)).py(px(4.0)).text_size(px(11.0)).text_color(p.danger).child(INVALID_DATE)))
        .when(state.open, |root| root.child(canvas(move |bounds, window, cx| {
            let bounds = window.to_window_bounds(bounds);
            let _ = owner.update(cx, |state, cx| {
                let viewport = window.viewport_size();
                if !state.open { return; }
                if bounds.bottom() <= px(0.0) || bounds.top() >= viewport.height || bounds.right() <= px(0.0) || bounds.left() >= viewport.width {
                    state.close(false, window, cx); return;
                }
                let below = (viewport.height - bounds.bottom() - px(12.0)).max(px(0.0));
                let above = (bounds.top() - px(12.0)).max(px(0.0));
                let upwards = below < px(180.0) && above > below;
                let available = if upwards { above } else { below };
                if available <= px(0.0) { state.close(false, window, cx); return; }
                let width = px(286.0).min((viewport.width - px(16.0)).max(px(0.0)));
                let measured = state.panel_bounds.clone();
                let panel = render_panel(cx.entity().clone(), state, bounds, width, available, p, window, cx);
                let mut popup = Positioner::corner(if upwards { Anchor::BottomLeft } else { Anchor::TopLeft },
                    point(bounds.left(), if upwards { bounds.top() - px(4.0) } else { bounds.bottom() + px(4.0) }))
                    .margin(px(8.0)).occlude().on_position(move |position| measured.set(Some(position.bounds))).child(panel).into_any_element();
                popup.layout_as_root(size(AvailableSpace::MaxContent, AvailableSpace::MaxContent), window, cx);
                window.defer_draw(popup, point(px(0.0), px(0.0)), gpui_kit::base::POPUP_PRIORITY, None);
            });
        }, move |_, _, window, _| {
            window.on_mouse_event::<ScrollEvent>(move |event, phase, window, cx| {
                if phase.capture() { let _ = scroll_owner.update(cx, |state, cx| {
                    if state.open && !state.panel_bounds.get().is_some_and(|bounds| bounds.contains(&event.position)) { state.close(false, window, cx); }
                }); }
            });
        }).absolute().top_0().left_0().size_full()))
}

fn render_panel(owner: Entity<DateInputState>, state: &DateInputState, bounds: Bounds<Pixels>, width: Pixels, available: Pixels, p: Palette, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let outside = owner.downgrade();
    let today = owner.downgrade();
    let clear = owner.downgrade();
    let month = state.month;
    let parent_view = window.current_view();
    let offset = month.weekday().num_days_from_monday() as i32;
    let navigation = |amount: i32, label: &'static str| {
        let owner = owner.downgrade();
        panel_button(("month", (amount + 12) as usize), label, state.shifted_month(amount).is_none(), p).size(px(28.0))
            .child(icon(if amount < 0 { "chevron-left" } else { "chevron-right" }, 16.0).opacity(if amount.abs() == 12 { 0.78 } else { 1.0 }))
            .on_click(move |_, window, cx| { let _ = owner.update(cx, |state, cx| state.shift_month(amount, false, window, cx)); })
    };
    div().id("calendar-panel").role(Role::Dialog).aria_label(state.label).tab_group().track_focus(&state.popover.focus_handle(cx))
        .on_mouse_down_out(move |event, window, cx| {
            if !bounds.contains(&event.position) { let _ = outside.update(cx, |state, cx| state.close(false, window, cx)); }
        })
        .relative().w(width).max_h(available).overflow_x_scroll().overflow_y_scroll().track_scroll(&state.scroll).p(px(10.0))
        .border_1().border_color(p.control_border).rounded(px(6.0)).bg(p.bg_elevated).text_color(p.text_primary).line_height(relative(1.4))
        .shadow(vec![BoxShadow { color: black().opacity(if p.dark { 0.36 } else { 0.12 }), offset: point(px(0.0), px(2.0)), blur_radius: px(8.0), spread_radius: px(0.0), inset: false },
            BoxShadow { color: black().opacity(if p.dark { 0.24 } else { 0.08 }), offset: point(px(0.0), px(1.0)), blur_radius: px(3.0), spread_radius: px(0.0), inset: false }])
        .child(div().flex().items_center().gap(px(3.0)).mb(px(8.0))
            .child(navigation(-12, "上一年")).child(navigation(-1, "上一月"))
            .child(div().flex_1().min_w_0().text_center().text_size(px(13.0)).font_weight(FontWeight::BOLD).child(format!("{}年{}月", month.year(), month.month())))
            .child(navigation(1, "下一月")).child(navigation(12, "下一年")))
        .child(div().id("date-calendar-grid").role(Role::Grid).aria_label(format!("{}年{}月", month.year(), month.month())).flex().flex_col().gap(px(2.0))
        .child(div().id("date-calendar-weekdays").role(Role::Row).grid().grid_cols(7).gap(px(2.0))
            .children(["一", "二", "三", "四", "五", "六", "日"].into_iter().enumerate().map(|(index, day)| div().id(("date-weekday", index)).role(Role::ColumnHeader).min_h(px(30.0)).flex().items_center().justify_center()
                .text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_secondary).child(day))))
        .children((0..6).map(|row| div().id(("date-calendar-row", row as usize)).role(Role::Row).grid().grid_cols(7).gap(px(2.0)).children((0..7).map(|column| {
            let day = row * 7 + column - offset + 1;
            let date = (day > 0).then(|| month.with_day(day as u32)).flatten();
            let mut cell = div().id(("date-calendar-cell", (row * 7 + column) as usize)).role(Role::GridCell).aria_selected(date.is_some() && date == state.value).min_w_0().min_h(px(30.0)).flex().items_center().justify_center();
            if let Some(date) = date {
                let focused = date == state.focused_day;
                let selected = Some(date) == state.value;
                let is_today = date == state.today;
                let owner = owner.downgrade();
                let reveal = state.reveal.clone();
                let scroll = state.scroll.clone();
                cell = cell.when(focused, |cell| cell.on_prepaint(move |bounds, _, cx| {
                    if !reveal.replace(false) { return; }
                    // Use actual descendant geometry, including the current
                    // scroll offset. The calendar needs no estimated row heights.
                    let viewport = scroll.bounds();
                    let delta = if bounds.top() < viewport.top() + px(10.0) { viewport.top() + px(10.0) - bounds.top() }
                        else if bounds.bottom() > viewport.bottom() - px(10.0) { viewport.bottom() - px(10.0) - bounds.bottom() }
                        else { px(0.0) };
                    if delta != px(0.0) {
                        let offset = scroll.offset();
                        let next = point(offset.x, (offset.y + delta).clamp(-scroll.max_offset().y, px(0.0)));
                        if next != offset { scroll.set_offset(next); cx.notify(parent_view); }
                    }
                }));
                cell = cell.child(Button::new(("day", date.num_days_from_ce() as usize)).tab_stop(focused)
                    .when(focused, |button| button.track_focus(&state.day_focus))
                    .accessibility_label(if is_today { format!("今天，{}", date.format("%Y-%m-%d")) } else { date.format("%Y-%m-%d").to_string() })
                    .size(px(30.0)).border_1().border_color(if is_today && !selected { p.primary } else { transparent_black() }).rounded(px(4.0))
                    .text_size(px(12.0)).cursor_pointer().bg(if selected { p.primary } else { transparent_black() })
                    .text_color(if selected { white() } else if is_today { p.primary } else { p.text_primary })
                    .when(is_today && !selected, |button| button.font_weight(FontWeight::SEMIBOLD))
                    .hover(move |style| style.bg(if selected { p.primary } else { p.selection }).border_color(p.primary))
                    .focus_visible(move |style| style.bg(if selected { p.primary } else { p.selection }).border_color(p.primary))
                    .child(day.to_string()).on_click(move |_, window, cx| { let _ = owner.update(cx, |state, cx| state.choose(Some(date), window, cx)); }));
            }
            cell
        })))))
        .child(div().flex().justify_between().mt(px(8.0)).border_t_1().border_color(p.border_light).pt(px(6.0))
            .child(panel_button("today", "今天", false, p).px(px(8.0)).text_color(p.primary).child("今天")
                .on_click(move |_, window, cx| { let _ = today.update(cx, |state, cx| state.choose(Some(state.today), window, cx)); }))
            .child(panel_button("clear", "清除", state.value.is_none(), p).px(px(8.0)).child("清除")
                .on_click(move |_, window, cx| { let _ = clear.update(cx, |state, cx| state.choose(None, window, cx)); })))
        .child(Scrollbar::new(&state.scroll).mode(ScrollbarMode::Always).styles(move |_| super::thin_scrollbar_styles(p)))
}

fn panel_button(id: impl Into<ElementId>, label: &'static str, disabled: bool, p: Palette) -> Button {
    Button::new(id).accessibility_label(label).disabled(disabled).min_w(px(28.0)).min_h(px(28.0)).rounded(px(4.0))
        .text_size(px(12.0)).text_color(p.text_secondary).opacity(if disabled { 0.5 } else { 1.0 })
        .when(disabled, |button| button.cursor_not_allowed())
        .when(!disabled, |button| button.cursor_pointer()
            .hover(move |style| style.bg(p.selection).text_color(p.primary))
            .focus_visible(move |style| style.bg(p.selection).text_color(p.primary)))
}
