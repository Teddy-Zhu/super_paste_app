use std::{rc::Rc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Easing, Transition, transition, slider::{SliderEvent, SliderState, SliderValue}};
use gpui_kit::component::select::{Select, SelectEvent};
use super_paste_format::{ColorMode, ConfigChange};
use uuid::Uuid;
use super::PaneContext;
use crate::ui::{dialogs, forms::{self, Choice, ChoiceState, IntegerChanged, IntegerInput, IntegerSpec}, palette::Palette, widgets::{PositionSelector, SliderMark, TextButton, Toggle, ValueSlider}};

const PRESETS: [(&str, i64); 6] = [("不限制", -1), ("周", 7), ("月", 30), ("季度", 90), ("半年", 180), ("一年", 365)];
const MARKS: [(&str, i64); 5] = [("7天", 7), ("30天", 30), ("90天", 90), ("180天", 180), ("365天", 365)];
pub struct GeneralPane {
    context: PaneContext, client_id: Uuid,
    color_mode: Entity<ChoiceState<ColorMode>>, count: Entity<IntegerInput>, keep_time: Entity<IntegerInput>, slider: Entity<SliderState>,
    animated_keep_target: Option<i64>, clearing: bool,
    subscriptions: Vec<Subscription>,
}
impl GeneralPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let configuration = context.configuration.clone();
        let preferences = configuration.read(cx).projected.clone();
        let preferences = &preferences.settings;
        let p = Palette::for_window(preferences.color_mode, window);
        let color_mode = forms::choices(vec![Choice::new("跟随系统", ColorMode::System), Choice::new("亮色模式", ColorMode::Light), Choice::new("暗黑模式", ColorMode::Dark)], &preferences.color_mode, window, cx);
        let count = cx.new(|cx| IntegerInput::new("最大历史记录数量", preferences.max_history_size, IntegerSpec { min: 10, max: 1000, step: 10, empty: Some(100), clamp: true }, p, window, cx));
        let keep_time = cx.new(|cx| IntegerInput::new("历史记录保存时间", preferences.max_keep_time, IntegerSpec { min: 1, max: 365, step: 1, empty: Some(-1), clamp: true }, p, window, cx));
        keep_time.update(cx, |input, cx| input.set_value(preferences.max_keep_time, preferences.max_keep_time == -1, p, window, cx));
        let slider_value = preferences.max_keep_time.max(1) as f32;
        let slider = cx.new(|_| SliderState::new().min(1.0).max(365.0).step(1.0).default_value(slider_value));
        let subscriptions = vec![
            cx.observe_in(&configuration, window, |this, _, window, cx| this.synchronize(window, cx)),
            cx.subscribe_in(&color_mode, window, |this, _, event: &SelectEvent<Vec<Choice<ColorMode>>>, window, cx| {
                if let SelectEvent::Confirm(Some(mode)) = event { this.submit(ConfigChange::ColorMode(*mode), window, cx); }
            }),
            cx.subscribe_in(&count, window, |this, _, value: &IntegerChanged, window, cx| this.submit(ConfigChange::MaxHistorySize(value.0), window, cx)),
            cx.subscribe_in(&keep_time, window, |this, _, value: &IntegerChanged, window, cx| this.set_keep_time(value.0, false, window, cx)),
            cx.subscribe_in(&slider, window, |this, _, event: &SliderEvent, window, cx| {
                if let SliderEvent::Change(SliderValue::Single(value)) = event {
                    this.set_keep_time(value.round() as i64, false, window, cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| this.synchronize(window, cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && this.slider.read(cx).is_interacting() {
                    this.slider.update(cx, |state, cx| state.cancel_interaction(cx));
                }
            }),
            cx.observe_window_visibility(window, |this, visibility, _, cx| {
                if visibility == WindowVisibility::Hidden && this.slider.read(cx).is_interacting() {
                    this.slider.update(cx, |state, cx| state.cancel_interaction(cx));
                }
            }),
        ];
        Self { context, client_id: Uuid::new_v4(), color_mode, count, keep_time, slider, animated_keep_target: None, clearing: false, subscriptions }
    }
    fn synchronize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let configuration = self.context.configuration.read(cx).projected.clone();
        let preferences = &configuration.settings;
        let p = Palette::for_window(preferences.color_mode, window);
        self.count.update(cx, |input, cx| input.set_value(preferences.max_history_size, false, p, window, cx));
        self.keep_time.update(cx, |input, cx| input.set_value(preferences.max_keep_time, preferences.max_keep_time == -1, p, window, cx));
        if self.color_mode.read(cx).selected_value() != Some(&preferences.color_mode) {
            self.color_mode.update(cx, |state, cx| state.set_selected_value(&preferences.color_mode, window, cx));
        }
        if self.animated_keep_target != Some(preferences.max_keep_time) { self.animated_keep_target = None; }
        cx.notify();
    }
    fn submit(&mut self, change: ConfigChange, _window: &mut Window, cx: &mut Context<Self>) {
        if !change.differs(&self.context.configuration.read(cx).projected) { return; }
        let client_id = self.client_id;
        if let Err(error) = self.context.configuration.update(cx, |model, cx| model.submit(client_id, vec![change], vec![], None, cx)) {
            tracing::warn!(%error, "保存即时设置失败");
        }
    }
    fn set_keep_time(&mut self, value: i64, animate: bool, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.context.configuration.read(cx).projected.settings.max_keep_time;
        self.animated_keep_target = (animate && old > 0 && value > 0).then_some(value);
        self.submit(ConfigChange::MaxKeepTime(value), window, cx);
        cx.notify();
    }
    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clearing || self.context.services.is_shutting_down() { return; }
        self.clearing = true;
        cx.notify();
        let services = self.context.services.clone();
        let task = services.runtime.spawn({ let clipboard = services.clipboard.clone(); async move { clipboard.clear_history().await } });
        let owner = window.window_handle();
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(super_paste_core::Error::from).and_then(|result| result);
            let _ = weak.update(cx, |this, cx| {
                this.clearing = false;
                cx.notify();
            });
            if services.is_shutting_down() { return; }
            // The accepted operation outlives its page. Report exactly once to
            // the originating settings window, even after switching sections.
            let message = match result { Ok(_) => "历史记录已清除".to_owned(), Err(error) => format!("清除历史记录失败：{error}") };
            let _ = owner.update(cx, |_, window, cx| dialogs::message(message, window, cx));
        }).detach();
    }
}
impl Render for GeneralPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let configuration = self.context.configuration.read(cx).projected.clone();
        let preferences = &configuration.settings;
        let p = Palette::for_window(preferences.color_mode, window);
        let compact = window.viewport_size().width <= px(900.0);
        let weak = cx.entity().downgrade();
        let position_weak = weak.clone(); let start_weak = weak.clone(); let silent_weak = weak.clone();
        let keep = preferences.max_keep_time;
        if keep == -1 && self.slider.read(cx).is_interacting() {
            self.slider.update(cx, |state, cx| state.cancel_interaction(cx));
        }
        let duration = if self.animated_keep_target == Some(keep) { 200 } else { 0 };
        let displayed = transition((ElementId::View(cx.entity_id()), "retention-display"), keep.max(1) as f32,
            Transition::new(Duration::from_millis(duration)).easing(Easing::Custom(Rc::new(gpui_kit::base::animation::ease_out_cubic))), window, cx).round();
        if self.slider.read(cx).value().end() != displayed { self.slider.update(cx, |state, cx| state.set_value(displayed, window, cx)); }
        let presets = PRESETS.into_iter().enumerate().map(|(index, (label, value))| {
            let weak = weak.clone();
            TextButton::new(("retention-preset", index), label, p).preset(keep == value)
                .on_click(move |_, window, cx| { let _ = weak.update(cx, |this, cx| this.set_keep_time(value, true, window, cx)); })
        });
        let marks = MARKS.into_iter().enumerate().map(|(index, (label, value))| {
            let weak = weak.clone();
            SliderMark::new(("retention-mark", index), label, (value - 1) as f32 / 364.0, keep == value, p)
                .on_click(move |_, window, cx| { let _ = weak.update(cx, |this, cx| this.set_keep_time(value, true, window, cx)); })
        });
        forms::page("general-settings", window)
            .child(forms::heading("通用设置", "配置应用的基本行为", p))
            .child(forms::settings_group(p)
                .child(forms::setting_row(p, false).child(forms::setting_info("颜色模式", "选择应用外观，默认跟随系统", p))
                    .child(Select::new(&self.color_mode).intrinsic_size(true).w(px(160.0)).h(px(28.0))))
                .child(forms::setting_row(p, true).child(forms::setting_info("开机自启", "系统启动时自动运行 SuperPaste", p))
                    .child(Toggle::new("auto-start", "开机自启", preferences.auto_start, p).on_change(move |value, window, cx| { let _ = start_weak.update(cx, |this, cx| this.submit(ConfigChange::AutoStart(value), window, cx)); })))
                .child(forms::setting_row(p, true).child(forms::setting_info("静默启动", "启动时不自动打开设置窗口，下次启动生效", p))
                    .child(Toggle::new("silent-start", "静默启动", preferences.silent_start, p).on_change(move |value, window, cx| { let _ = silent_weak.update(cx, |this, cx| this.submit(ConfigChange::SilentStart(value), window, cx)); })))
                .child(forms::setting_row(p, true).when(compact, |row| row.flex_col().items_stretch().gap(px(16.0)))
                    .child(forms::setting_info("粘贴面板出现位置", "设置快捷面板从屏幕的哪个方向出现，默认底部", p))
                    .child(PositionSelector::new(("overlay-position", cx.entity_id()), preferences.overlay_position, p, compact, move |value, window, cx| { let _ = position_weak.update(cx, |this, cx| this.submit(ConfigChange::OverlayPosition(value), window, cx)); })))
                .child(forms::setting_row(p, true).child(forms::setting_info("最大历史记录数量", "设置剪贴板历史记录的最大保存数量 (10-1000)", p))
                    .child(div().flex().items_center().gap(px(8.0)).child(self.count.clone()).child(div().text_size(px(12.0)).text_color(p.text_secondary).child("条"))))
                .child(forms::setting_row(p, true).flex_col().items_stretch().gap(px(16.0))
                    .child(forms::setting_info("历史记录保存时间", "设置历史记录的最大保存时间", p))
                    .child(div().flex().items_center().justify_between().flex_wrap().gap(px(16.0))
                        .child(div().flex().flex_1().min_w_0().flex_wrap().gap(px(4.0)).children(presets))
                        .child(div().flex().items_center().gap(px(8.0)).child(self.keep_time.clone()).child(div().text_size(px(12.0)).text_color(p.text_secondary).child(if keep == -1 { "不限制" } else { "天" }))))
                    .when(keep != -1, |row| row.child(div().flex().flex_col().gap(px(0.0)).mt(px(-4.0))
                        .child(ValueSlider::new(("keep-time-slider", cx.entity_id()), &self.slider, "历史记录保存时间", p).accessibility_value(keep as f32))
                        .child(div().relative().h(px(32.0)).mx(px(16.0)).children(marks)))))
                .child(forms::setting_row(p, true).child(forms::setting_info("清除历史记录", "清除所有剪贴板历史记录，此操作不可撤销", p))
                    .child(TextButton::new("clear-history", "清除历史", p).danger().disabled(self.clearing).on_click(move |_, window, cx| {
                        let weak = weak.clone();
                        dialogs::confirm("确定要清除所有剪贴板历史记录吗？此操作不可撤销。", window, cx, move |window, cx| { let _ = weak.update(cx, |this, cx| this.clear(window, cx)); });
                    }))))
    }
}
