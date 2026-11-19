use std::sync::Arc;
use gpui_kit::{prelude::*, *};
use gpui_kit::component::{Disableable, select::SelectEvent};
use super_paste_core::{statistics::{StatisticsKind, StatisticsService}};
use super_paste_format::{CopyGroup, CopyStats, CopyStatsQuery, CopyTotals, PrimaryType, StatisticsInterval};
use crate::ui::{forms::{self, Choice, ChoiceState}, palette::Palette, widgets::{DateInput, DateInputChanged, DateInputState, TextButton}};
use super::{PaneContext, Period, chart::{Chart, ChartPoint, ValueFormat}, live::{self, Live}};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Metric { Copies, Bytes, Files }
impl Metric {
    fn label(self) -> &'static str { match self { Self::Copies => "复制次数", Self::Bytes => "复制数据量", Self::Files => "复制文件数" } }
    fn value(self, totals: &CopyTotals) -> u64 { match self { Self::Copies => totals.copies, Self::Bytes => totals.bytes, Self::Files => totals.files } }
}
pub struct CopyPane {
    context: PaneContext, live: Entity<Live<Request, CopyStats>>,
    period: Entity<ChoiceState<Period>>, kind: Entity<ChoiceState<Option<PrimaryType>>>, source: Entity<ChoiceState<Option<String>>>,
    interval: Entity<ChoiceState<StatisticsInterval>>, metric: Entity<ChoiceState<Metric>>, start: Entity<DateInputState>, end: Entity<DateInputState>,
    source_options: Vec<String>, chart: Entity<Chart>, chart_source: Option<Arc<CopyStats>>, chart_metric: Metric, _subscriptions: Vec<Subscription>,
}
#[derive(Clone)]
struct Request { period: Period, dates: Option<(chrono::NaiveDate, chrono::NaiveDate)>, kind: Option<PrimaryType>, source: Option<String>, interval: StatisticsInterval }
fn fetch(service: Arc<StatisticsService>, query: Request, cancel: tokio_util::sync::CancellationToken) -> live::FetchResult<CopyStats> {
    Box::pin(async move {
        let (from, to) = super::range(query.period, query.dates)?;
        service.copy(CopyStatsQuery { from, to, kind: query.kind, source: query.source, interval: query.interval }, &cancel).await
    })
}
impl CopyPane {
    pub fn new(context: PaneContext, owner: WeakEntity<super::StatisticsPane>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let period = forms::choices(vec![Choice::new("今天", Period::Today), Choice::new("最近 7 天", Period::Week), Choice::new("最近 30 天", Period::Month), Choice::new("全部时间", Period::All), Choice::new("自定义", Period::Custom)], &Period::Week, window, cx);
        let mut types = vec![Choice::new("全部类型", None)];
        types.extend([PrimaryType::Text, PrimaryType::Image, PrimaryType::Html, PrimaryType::Url, PrimaryType::Files, PrimaryType::Email, PrimaryType::Phone, PrimaryType::Code, PrimaryType::Rtf, PrimaryType::Color].into_iter().map(|kind| Choice::new(super::type_name(kind), Some(kind))));
        let kind = forms::choices(types, &None, window, cx);
        let source = forms::choices(vec![Choice::new("全部应用", None::<String>)], &None, window, cx);
        let interval = forms::choices(vec![Choice::new("按小时", StatisticsInterval::Hour), Choice::new("按天", StatisticsInterval::Day), Choice::new("按月", StatisticsInterval::Month)], &StatisticsInterval::Day, window, cx);
        let metric = forms::choices(vec![Choice::new("复制次数", Metric::Copies), Choice::new("复制数据量", Metric::Bytes), Choice::new("复制文件数", Metric::Files)], &Metric::Copies, window, cx);
        let start = cx.new(|cx| DateInputState::new("开始日期", window, cx));
        let end = cx.new(|cx| DateInputState::new("结束日期", window, cx));
        let chart = cx.new(|_| Chart::new("copy-trend"));
        let initial_query = Request { period: Period::Week, dates: None, kind: None, source: None, interval: StatisticsInterval::Day };
        let origin = cx.entity_id();
        let live = cx.new(|cx| Live::new(context.services.clone(), owner, origin, StatisticsKind::Copy, Ok(initial_query), false, fetch, window, cx));
        let subscriptions = vec![
            cx.observe_in(&live, window, |this, _, window, cx| { this.update_sources(window, cx); this.update_chart(cx); cx.notify(); }),
            cx.subscribe_in(&period, window, |this, _, _: &SelectEvent<Vec<Choice<Period>>>, window, cx| this.query(window, cx)),
            cx.subscribe_in(&kind, window, |this, _, _: &SelectEvent<Vec<Choice<Option<PrimaryType>>>>, window, cx| this.query(window, cx)),
            cx.subscribe_in(&source, window, |this, _, _: &SelectEvent<Vec<Choice<Option<String>>>>, window, cx| this.query(window, cx)),
            cx.subscribe_in(&interval, window, |this, _, _: &SelectEvent<Vec<Choice<StatisticsInterval>>>, window, cx| this.query(window, cx)),
            cx.subscribe_in(&metric, window, |this, _, _: &SelectEvent<Vec<Choice<Metric>>>, _, cx| { this.update_chart(cx); cx.notify(); }),
            cx.subscribe_in(&start, window, |this, _, _: &DateInputChanged, window, cx| this.query(window, cx)),
            cx.subscribe_in(&end, window, |this, _, _: &DateInputChanged, window, cx| this.query(window, cx)),
        ];
        Self { context, live, period, kind, source, interval, metric, start, end, source_options: Vec::new(), chart, chart_source: None, chart_metric: Metric::Copies, _subscriptions: subscriptions }
    }
    fn query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let period = self.period.read(cx).selected_value().copied().unwrap_or(Period::Week);
        if period != Period::Custom {
            self.start.update(cx, |state, cx| state.hide(window, cx));
            self.end.update(cx, |state, cx| state.hide(window, cx));
        }
        let custom = self.start.read(cx).value().zip(self.end.read(cx).value());
        let query = super::range(period, custom).map(|_| Request {
            period, dates: custom, kind: self.kind.read(cx).selected_value().copied().flatten(), source: self.source.read(cx).selected_value().cloned().flatten(),
            interval: self.interval.read(cx).selected_value().copied().unwrap_or(StatisticsInterval::Day),
        });
        self.live.update(cx, |live, cx| live.set_query(query, window, cx));
    }
    fn update_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stats) = self.live.read(cx).value.clone() else { return; };
        let mut sources = stats.available_sources.clone();
        if let Some(Some(selected)) = self.source.read(cx).selected_value() { if !sources.contains(selected) { sources.push(selected.clone()); } }
        if sources == self.source_options { return; }
        self.source_options = sources;
        let mut choices = vec![Choice::new("全部应用", None::<String>)];
        choices.extend(self.source_options.iter().map(|source| Choice::new(if source.is_empty() { "未知应用".to_owned() } else { source.clone() }, Some(source.clone()))));
        self.source.update(cx, |source, cx| source.set_items(choices, window, cx));
    }
    fn update_chart(&mut self, cx: &mut Context<Self>) {
        let metric = self.metric.read(cx).selected_value().copied().unwrap_or(Metric::Copies);
        let value = self.live.read(cx).value.clone();
        let unchanged = match (&self.chart_source, &value) { (Some(previous), Some(next)) => Arc::ptr_eq(previous, next), (None, None) => true, _ => false };
        if unchanged && self.chart_metric == metric { return; }
        self.chart_source = value.clone(); self.chart_metric = metric;
        let points = value.as_ref().map(|stats| stats.series.iter().map(|point| ChartPoint { timestamp: point.timestamp, value: metric.value(&point.totals) as f64 }).collect()).unwrap_or_default();
        let interval = value.as_ref().map(|stats| stats.interval).unwrap_or(StatisticsInterval::Day);
        self.chart.update(cx, |chart, cx| chart.set(points, metric.label(), interval, if metric == Metric::Bytes { ValueFormat::Bytes } else { ValueFormat::Number }, cx));
    }
}
fn filter(label: &'static str, control: impl IntoElement, p: Palette) -> Div {
    div().flex().flex_col().gap(px(6.0)).text_size(px(12.0)).text_color(p.text_secondary).child(label).child(control)
}
fn breakdown(title: &'static str, column: &'static str, rows: &[CopyGroup], copies: u64, types: bool, p: Palette) -> Div {
    div().child(super::heading(title, p)).child(super::table(title, &[column, "复制次数", "次数占比", "复制数据量", "复制文件数"], rows.iter().map(|row| {
        let name = if types { PrimaryType::parse(&row.key).map(super::type_name).unwrap_or(&row.key) } else if row.key.is_empty() { "未知应用" } else { &row.key };
        vec![div().child(super::table_value("value", name.to_owned())).into_any_element(), div().child(super::table_value("value", super::number(row.totals.copies))).into_any_element(),
            div().child(super::table_value("value", format!("{:.1}%", row.totals.copies as f64 / copies as f64 * 100.0))).into_any_element(),
            div().child(super::table_value("value", super::bytes(row.totals.bytes))).into_any_element(), div().child(super::table_value("value", super::number(row.totals.files))).into_any_element()]
    }), super::PaneKind::Copy, p))
}
impl Render for CopyPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let live = self.live.read(cx);
        let loading = live.loading; let clearing = live.clearing; let confirming = live.confirming;
        let error = live.operation_error.clone().or_else(|| live.error.clone()); let value = live.value.clone();
        let refresh = self.live.downgrade(); let clear = self.live.downgrade();
        super::card(p).child(div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary).mb(px(4.0)).child("复制统计"))
            .child(super::note("累计复制活动，支持按时间、内容类型和来源应用查看。", p).line_height(px(18.0)))
            .child(div().flex().flex_wrap().items_end().gap(px(10.0)).mt(px(16.0))
                .child(filter("时间范围", forms::choice_control(&self.period, "时间范围", p).disabled(clearing), p))
                .when(self.period.read(cx).selected_value() == Some(&Period::Custom), |row| row
                    .child(filter("开始日期", DateInput::new(&self.start, p).disabled(clearing), p))
                    .child(filter("结束日期", DateInput::new(&self.end, p).disabled(clearing), p)))
                .child(filter("内容类型", forms::choice_control(&self.kind, "内容类型", p).disabled(clearing), p))
                .child(filter("来源应用", forms::choice_control(&self.source, "来源应用", p).disabled(clearing), p))
                .child(filter("时间粒度", forms::choice_control(&self.interval, "时间粒度", p).disabled(clearing), p))
                .child(TextButton::new("refresh-copy-statistics", if loading { "正在加载…" } else { "刷新" }, p).neutral().disabled(loading || clearing)
                    .on_click(move |_, window, cx| { let _ = refresh.update(cx, |live, cx| live.refresh(window, cx)); }))
                .child(TextButton::new("clear-copy-statistics", "清空复制统计", p).neutral().disabled(clearing)
                    .on_click(move |_, _, cx| { let _ = clear.update(cx, |live, cx| live.request_clear(cx)); })))
            .when(confirming, |card| card.child(super::clear_confirmation(&self.live, "确认清空全部复制统计？历史记录和 Token 用量将保留。", clearing, super::PaneKind::Copy, p)))
            .when_some(error, |card, error| card.child(div().id("copy-statistics-error").role(Role::Alert).my(px(13.0)).text_size(px(13.0)).text_color(p.danger).child(error)))
            .when_some(value, |card, stats| card
                .child(super::metric_grid(120.0, 16.0).mt(px(24.0)).mb(px(16.0)).line_height(relative(1.7))
                    .child(super::metric("复制次数", super::number(stats.totals.copies), 22.0, p))
                    .child(super::metric("复制数据量", super::bytes(stats.totals.bytes), 22.0, p))
                    .child(super::metric("复制文件数", super::number(stats.totals.files), 22.0, p))
                    .child(super::metric("活跃天数", super::number(stats.active_days as u64), 22.0, p)))
                .child(super::note("包含剪贴板文本、图片及文件路径的数据量，不包含文件本体。", p))
                .child(super::note(format!("首次复制: {} · 最近复制: {}", super::time(stats.totals.first_copied_at), super::time(stats.totals.last_copied_at)), p))
                .when_some(stats.cleared_at, |card, timestamp| card.child(super::note(format!("上次清空: {}", forms::local_timestamp(timestamp)), p)))
                .child(div().flex().items_center().justify_between().gap(px(10.0)).mt(px(24.0))
                    .child(super::heading("复制趋势", p)).child(forms::choice_control(&self.metric, "趋势指标", p)))
                .child(self.chart.clone())
                .when(stats.totals.copies == 0, |card| card.child(super::note("所选范围暂无复制数据", p)))
                .when(stats.totals.copies != 0, |card| card
                    .child(breakdown("按类型", "内容类型", &stats.types, stats.totals.copies, true, p))
                    .child(breakdown("按来源应用", "来源应用", &stats.sources, stats.totals.copies, false, p))))
            .child(div().mt(px(20.0)).child(super::note("自启用此功能后开始统计；删除历史记录不影响统计。连续相同内容的重复通知不重复计数。", p)))
    }
}
