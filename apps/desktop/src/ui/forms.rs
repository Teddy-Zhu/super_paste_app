use gpui_kit::{prelude::*, *};
use gpui_kit::base::{NumberInput, StepAction};
use gpui_kit::component::{Disableable, input::{Input, InputEvent, InputState, MaskPattern}, scroll::ScrollableElement,
    searchable_list::SearchableListItem, select::{Select, SelectMenuStyle, SelectState}};
use super::{icon, palette::Palette};

pub fn local_timestamp(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|time| time.with_timezone(&chrono::Local).format("%Y/%-m/%-d %H:%M:%S").to_string())
        .unwrap_or_else(|| "无效日期".to_owned())
}

pub fn tabular() -> FontFeatures {
    static FEATURES: std::sync::LazyLock<FontFeatures> = std::sync::LazyLock::new(|| FontFeatures(std::sync::Arc::new(vec![("tnum".to_owned(), 1)])));
    FEATURES.clone()
}

#[derive(Clone)]
pub struct Choice<T> { pub title: SharedString, pub value: T }
impl<T> Choice<T> {
    pub fn new(title: impl Into<SharedString>, value: T) -> Self { Self { title: title.into(), value } }
}
impl<T: Clone + PartialEq> SearchableListItem for Choice<T> {
    type Value = T;
    fn title(&self) -> SharedString { self.title.clone() }
    fn value(&self) -> &T { &self.value }
}
pub type ChoiceState<T> = SelectState<Vec<Choice<T>>>;

pub fn choices<T: Clone + PartialEq + 'static>(items: Vec<Choice<T>>, selected: &T, window: &mut Window, cx: &mut App) -> Entity<ChoiceState<T>> {
    let index = items.iter().position(|item| &item.value == selected).map(|index| gpui_kit::component::IndexPath::default().row(index));
    cx.new(|cx| SelectState::new(items, index, window, cx))
}

pub fn choice_control<T: Clone + PartialEq + 'static>(state: &Entity<ChoiceState<T>>, label: &'static str, p: Palette) -> Select<Vec<Choice<T>>> {
    Select::new(state).accessibility_label(label).intrinsic_size(true).appearance(false)
        .menu_style(choice_menu(p)).scroll_bounce(false)
        .w_auto().h_auto().min_w(px(112.0)).max_w(px(220.0)).min_h(px(30.0))
        .px(px(9.0)).py(px(5.0)).gap(px(12.0)).rounded(px(4.0))
        .text_size(px(13.0)).line_height(relative(1.3)).text_color(p.text_primary)
        .icon(gpui_kit::component::Icon::new(gpui_kit::component::IconName::ChevronDown).size(px(16.0)).text_color(p.text_secondary))
        .trigger(move |root, trigger, window, cx| {
            let active = trigger.hovered || trigger.focused;
            let id: ElementId = ("choice-control", trigger.id).into();
            let border = crate::ui::widgets::color_motion(&id, "border", if active { p.primary } else { p.control_border }, window, cx);
            let background = crate::ui::widgets::color_motion(&id, "background", if active { p.bg_tertiary } else { p.bg_primary }, window, cx);
            root.border_color(border).bg(background).opacity(if trigger.disabled { 0.5 } else { 1.0 })
                .when(trigger.disabled, |root| root.cursor_not_allowed())
                .when(!trigger.disabled, |root| root.cursor_pointer())
                .child(div().flex_1().min_w_0().whitespace_nowrap().truncate().child(trigger.title))
                .child(div().flex().items_center().justify_center().flex_shrink_0().size(px(16.0)).child(trigger.trailing))
        })
}

fn choice_menu(p: Palette) -> SelectMenuStyle {
    let mut surface = div().p_0().border_1().border_color(p.control_border).rounded(px(6.0))
        .bg(p.bg_elevated).text_color(p.text_primary).text_size(px(13.0)).line_height(relative(1.4))
        .overflow_wrap(gpui::OverflowWrap::Normal)
        .shadow(vec![BoxShadow { color: black().opacity(if p.dark { 0.36 } else { 0.12 }),
            offset: point(px(0.0), px(2.0)), blur_radius: px(8.0), spread_radius: px(0.0), inset: false },
            BoxShadow { color: black().opacity(if p.dark { 0.24 } else { 0.08 }),
            offset: point(px(0.0), px(1.0)), blur_radius: px(3.0), spread_radius: px(0.0), inset: false }]);
    SelectMenuStyle {
        surface: surface.style().clone(), variable_rows: true, animated: false, offset: px(4.0), trigger_navigation: true, viewport_fit: true,
        scrollbar: Some(super::widgets::thin_scrollbar_styles(p)), scrollbar_gutter: Some(px(6.0)),
        row_renderer: Some(std::rc::Rc::new(move |root, row, _, _| {
            root.w_full().min_h(px(30.0)).px(px(9.0)).py(px(6.0)).rounded(px(4.0))
                .items_center().justify_between().gap(px(12.0)).text_size(px(13.0)).line_height(relative(1.4))
                .text_color(if row.disabled { p.text_tertiary } else if row.checked { p.primary } else { p.text_primary })
                .font_weight(if row.checked { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .bg(if row.active && !row.disabled { p.selection } else { transparent_black() })
                .opacity(if row.disabled { 0.5 } else { 1.0 })
                .when(row.disabled, |root| root.cursor_not_allowed())
                .when(!row.disabled, |root| root.cursor_pointer())
                .children(row.children)
                .when(row.checked, |root| root.child(div().id("check").aria_hidden(true).flex_shrink_0().text_color(p.primary).child("✓")))
        })),
    }
}

pub fn page(id: &'static str, window: &Window) -> gpui_kit::component::scroll::Scrollable<Stateful<Div>> {
    let compact = window.viewport_size().width < px(900.0);
    div().id(id).size_full().min_h_0().min_w_0().max_w(px(1120.0)).mx_auto()
        .pt(px(if compact { 20.0 } else { 24.0 })).pl(px(if compact { 18.0 } else { 28.0 }))
        // Match scrollbar-gutter: stable even when this page does not overflow.
        .pr(px(if compact { 26.0 } else { 36.0 })).pb(px(if compact { 28.0 } else { 32.0 }))
        .overflow_y_scrollbar()
}

pub fn heading(title: impl Into<SharedString>, subtitle: impl Into<SharedString>, p: Palette) -> Div {
    let title = title.into();
    div().mb(px(20.0))
        .child(div().id(title.clone()).role(Role::Heading).aria_level(2).text_size(px(20.0)).line_height(px(26.0)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_primary).mb(px(6.0)).child(title))
        .child(div().text_size(px(12.0)).line_height(px(18.0)).text_color(p.text_secondary).child(subtitle.into()))
}
pub fn setting_info(title: impl Into<SharedString>, description: impl Into<SharedString>, p: Palette) -> Div {
    div().flex_1().min_w_0()
        .child(div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary).mb(px(4.0)).child(title.into()))
        .child(div().text_size(px(12.0)).line_height(px(18.0)).text_color(p.text_secondary).child(description.into()))
}
pub fn setting_row(p: Palette, divided: bool) -> Div {
    div().flex().w_full().min_w_0().flex_shrink_0().justify_between().items_center().gap(px(20.0)).min_h(px(66.0)).py(px(14.0)).px(px(16.0))
        .when(divided, |row| row.border_t_1().border_color(p.border_light))
}
pub fn settings_group(p: Palette) -> Div { div().w_full().flex().flex_col().items_stretch().border_1().border_color(p.border_medium).rounded(px(8.0)).bg(p.bg_primary) }

#[derive(Clone, Copy)]
pub struct IntegerSpec { pub min: i64, pub max: i64, pub step: i64, pub empty: Option<i64>, pub clamp: bool }
impl IntegerSpec {
    fn commit(self, text: &str) -> Result<i64, &'static str> {
        let text = trim_number(text);
        if text.is_empty() { return self.empty.ok_or("请输入有效整数"); }
        let digits = text.strip_prefix('+').or_else(|| text.strip_prefix('-')).unwrap_or(text);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) { return Err("请输入有效整数"); }
        // The reference checks a finite Number before clamping. A decimal
        // outside i64 still clamps normally; overflow to infinity is invalid.
        let number = draft_number(text).ok_or("请输入有效整数")?;
        if !self.clamp && (number < self.min as f64 || number > self.max as f64) { return Err("数值超出允许范围"); }
        Ok(number.clamp(self.min as f64, self.max as f64) as i64)
    }
}

fn trim_number(text: &str) -> &str {
    text.trim_matches(|ch| matches!(ch, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'))
}

/// Steppers use the finite draft's numeric value, including decimals, radix
/// prefixes and exponents. Commit independently requires signed decimal digits.
fn draft_number(text: &str) -> Option<f64> {
    let text = trim_number(text);
    if text.is_empty() { return None; }
    let radix = match text.as_bytes() {
        [b'0', b'x' | b'X', ..] => Some(16u32),
        [b'0', b'o' | b'O', ..] => Some(8),
        [b'0', b'b' | b'B', ..] => Some(2),
        _ => None,
    };
    let number = if let Some(radix) = radix {
        let digits = text.get(2..)?;
        if digits.is_empty() { return None; }
        digits.chars().try_fold(0.0, |value, ch| ch.to_digit(radix).map(|digit| value * f64::from(radix) + f64::from(digit)))?
    } else {
        if !text.bytes().all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E')) { return None; }
        text.parse::<f64>().ok()?
    };
    number.is_finite().then_some(number)
}

pub struct IntegerChanged(pub i64);
pub struct IntegerInput {
    state: Entity<InputState>, label: SharedString, value: i64, spec: IntegerSpec,
    focused: bool, invalid: Option<&'static str>, disabled: bool, hovered: bool, palette: Palette,
    subscriptions: Vec<Subscription>,
}
impl EventEmitter<IntegerChanged> for IntegerInput {}
impl IntegerInput {
    pub fn new(label: impl Into<SharedString>, value: i64, spec: IntegerSpec, palette: Palette, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Preserve the editable draft, including invalid input, until commit.
        // NumberInput respects an explicitly selected unmasked editor.
        let state = cx.new(|cx| InputState::new(window, cx).mask_pattern(MaskPattern::None).default_value(value.to_string()));
        let subscriptions = vec![cx.subscribe_in(&state, window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::Focus => this.focused = true,
                InputEvent::Blur => { this.focused = false; this.commit(window, cx); }
                InputEvent::PressEnter { .. } => this.commit(window, cx),
                InputEvent::Change => this.invalid = None,
            }
            cx.notify();
        })];
        Self { state, label: label.into(), value, spec, focused: false, invalid: None, disabled: false, hovered: false, palette, subscriptions }
    }
    pub fn set_value(&mut self, value: i64, disabled: bool, palette: Palette, window: &mut Window, cx: &mut Context<Self>) {
        if self.value == value && self.disabled == disabled && self.palette == palette { return; }
        self.value = value;
        self.disabled = disabled;
        self.palette = palette;
        if !self.focused && self.invalid.is_none() {
            let text = if disabled && Some(value) == self.spec.empty && value < self.spec.min { String::new() } else { value.to_string() };
            if self.state.read(cx).value().as_ref() != text { self.state.update(cx, |state, cx| state.set_value(text, window, cx)); }
            let placeholder = if disabled && value < self.spec.min { "不限制" } else if self.spec.empty.is_some_and(|empty| empty < self.spec.min) { "输入天数" } else { "" };
            if self.state.read(cx).placeholder_text() != placeholder { self.state.update(cx, |state, cx| state.set_placeholder(placeholder, window, cx)); }
        }
        cx.notify();
    }
    fn current(&self, cx: &App) -> f64 { draft_number(self.state.read(cx).value().as_ref()).unwrap_or(self.value as f64) }
    pub fn valid_value(&self, cx: &App) -> Option<i64> { self.spec.commit(self.state.read(cx).value().as_ref()).ok() }
    fn emit(&mut self, value: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.invalid = None;
        self.state.update(cx, |state, cx| state.set_value(value.to_string(), window, cx));
        if value != self.value { cx.emit(IntegerChanged(value)); }
        cx.notify();
    }
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.state.read(cx).is_composing() { return; }
        let text = self.state.read(cx).value();
        match self.spec.commit(text.as_ref()) {
            Ok(value) => self.emit(value, window, cx),
            Err(error) => { self.invalid = Some(error); cx.notify(); }
        }
    }
    fn step(&mut self, action: StepAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.state.read(cx).is_composing() { return; }
        let current = self.current(cx);
        let floor = current.floor();
        let current = if current - floor >= 0.5 { floor + 1.0 } else { floor };
        let delta = if action == StepAction::Increment { self.spec.step } else { -self.spec.step };
        self.emit((current + delta as f64).clamp(self.spec.min as f64, self.spec.max as f64) as i64, window, cx);
        self.state.read(cx).focus_handle(cx).focus(window, cx);
    }
}
impl Render for IntegerInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let current = self.current(cx);
        let decrement_disabled = self.disabled || current <= self.spec.min as f64;
        let increment_disabled = self.disabled || current >= self.spec.max as f64;
        let weak = cx.entity().downgrade();
        let step = move |action, window: &mut Window, cx: &mut App| { let _ = weak.update(cx, |this, cx| this.step(action, window, cx)); };
        let hovered = self.hovered && !self.disabled;
        let border = if self.invalid.is_some() { p.danger } else if self.focused || hovered { p.primary } else { p.control_border };
        let background = if self.disabled { p.bg_secondary } else if hovered && !self.focused { p.bg_tertiary } else { p.bg_primary };
        let id: ElementId = ("integer-input", cx.entity_id()).into();
        let border = crate::ui::widgets::color_motion(&id, "border", border, window, cx);
        let background = crate::ui::widgets::color_motion(&id, "background", background, window, cx);
        div().id(id).w(px(80.0)).h(px(28.0)).rounded(px(4.0)).border_1().overflow_hidden()
            .border_color(border).bg(background)
            .opacity(if self.disabled { 0.5 } else { 1.0 })
            .on_hover(cx.listener(|this, hovered, _, cx| { this.hovered = *hovered; cx.notify(); }))
            .child(NumberInput::new(&self.state).controls_right().disabled(self.disabled).accessibility_label(self.label.clone())
                .size_full().on_step(step)
                .input(Input::new(&self.state).text_direction(gpui::TextDirection::Ltr).appearance(false).disabled(self.disabled)
                    .aria_label(self.label.clone()).aria_invalid(self.invalid.is_some())
                    .when_some(self.invalid, |input, error| input.aria_description(error))
                    .h_full().flex_1().min_w_0().rounded_none().text_size(px(12.0)).line_height(px(16.8)).px(px(8.0)).py(px(4.0)))
                .increment_button(move |button| button.disabled(increment_disabled).accessibility_label("增加")
                    .w(px(20.0)).h_full().flex().items_center().justify_center().border_l_1().border_b_1().border_color(p.border_light)
                    .text_color(p.text_secondary).hover(move |style| style.bg(p.control_hover)).child(icon("chevron-up", 12.0)))
                .decrement_button(move |button| button.disabled(decrement_disabled).accessibility_label("减少")
                    .w(px(20.0)).h_full().flex().items_center().justify_center().border_l_1().border_color(p.border_light)
                    .text_color(p.text_secondary).hover(move |style| style.bg(p.control_hover)).child(icon("chevron-down", 12.0))))
    }
}
