use std::{collections::BTreeMap, sync::Arc};
use chrono::NaiveDate;
use gpui_kit::{prelude::*, *};
use gpui::OverflowWrap;
use gpui_kit::base::Button;
use gpui_kit::component::{Disableable, select::SelectEvent};
use super_paste_core::{Error, Result, statistics::{StatisticsKind, StatisticsService}};
use super_paste_format::{AiPurpose, AiUsageQuery, AiUsageStats, StatisticsInterval};
use crate::ui::{forms::{self, Choice, ChoiceState}, icon, palette::Palette, widgets::TextButton};
use super::{PaneContext, Period, chart::{Chart, ChartPoint, ValueFormat}, live::{self, Live}};

pub struct UsagePane {
    context: PaneContext, period: Entity<ChoiceState<Period>>, live: Entity<Live<Period, AiUsageStats>>,
    chart: Entity<Chart>, chart_source: Option<Arc<AiUsageStats>>, chart_period: Period, chart_error: Option<SharedString>,
    days_open: bool, recent_open: bool, _subscriptions: Vec<Subscription>,
}
impl UsagePane {
    pub fn new(context: PaneContext, owner: WeakEntity<super::StatisticsPane>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let period = forms::choices(vec![Choice::new("全部时间", Period::All), Choice::new("最近 7 天", Period::Week), Choice::new("最近 30 天", Period::Month)], &Period::All, window, cx);
        let origin = cx.entity_id();
        let live = cx.new(|cx| Live::new(context.services.clone(), owner, origin, StatisticsKind::Ai, Ok(Period::All), true, fetch, window, cx));
        let chart = cx.new(|_| Chart::new("token-trend"));
        let subscriptions = vec![
            cx.observe_in(&live, window, |this, _, _, cx| { this.update_chart(cx); cx.notify(); }),
            cx.subscribe_in(&period, window, |this, _, _: &SelectEvent<Vec<Choice<Period>>>, window, cx| {
                let period = this.period.read(cx).selected_value().copied().unwrap_or(Period::All);
                this.live.update(cx, |live, cx| live.set_query(Ok(period), window, cx));
                this.update_chart(cx);
            }),
        ];
        Self { context, period, live, chart, chart_source: None, chart_period: Period::All, chart_error: None, days_open: false, recent_open: false, _subscriptions: subscriptions }
    }
    fn update_chart(&mut self, cx: &mut Context<Self>) {
        let value = self.live.read(cx).value.clone();
        let period = self.period.read(cx).selected_value().copied().unwrap_or(Period::All);
        let unchanged = match (&self.chart_source, &value) { (Some(previous), Some(next)) => Arc::ptr_eq(previous, next), (None, None) => true, _ => false };
        if unchanged && self.chart_period == period { return; }
        self.chart_source = value.clone(); self.chart_period = period;
        let trend = value.as_ref().map(|stats| trend(stats, period)).transpose();
        match trend {
            Ok(trend) => {
                self.chart_error = None;
                let (points, interval) = trend.unwrap_or((Vec::new(), StatisticsInterval::Day));
                self.chart.update(cx, |chart, cx| chart.set(points, "总 Token", interval, ValueFormat::Number, cx));
            }
            Err(error) => { self.chart_error = Some(error.to_string().into()); }
        }
    }
}
fn fetch(service: Arc<StatisticsService>, period: Period, cancel: tokio_util::sync::CancellationToken) -> live::FetchResult<AiUsageStats> {
    Box::pin(async move {
        let (from, _) = super::range(period, None)?;
        service.ai(AiUsageQuery { from, to: None }, &cancel).await
    })
}
impl Render for UsagePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let live = self.live.read(cx);
        let loading = live.loading; let clearing = live.clearing; let confirming = live.confirming;
        let error = live.operation_error.clone().or_else(|| live.error.clone()); let stats = live.value.clone();
        let refresh = self.live.downgrade(); let clear = self.live.downgrade(); let days = cx.entity().downgrade(); let recent = days.clone();
        super::card(p).child(div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary).mb(px(4.0)).child("AI OCR 用量"))
            .child(note("按接口实际返回的 Token 统计，包含连通性测试。用量按小时汇总，保留最近 50 条请求明细；缓存 Token 已包含在输入用量中。", p).line_height(px(18.0)))
            .child(div().flex().flex_wrap().gap(px(8.0)).mt(px(12.0))
                .child(forms::choice_control(&self.period, "时间范围", p).disabled(clearing))
                .child(TextButton::new("refresh-token-statistics", "刷新", p).neutral().disabled(loading || clearing)
                    .on_click(move |_, window, cx| { let _ = refresh.update(cx, |live, cx| live.refresh(window, cx)); }))
                .child(TextButton::new("clear-token-statistics", "清空统计", p).neutral().disabled(clearing)
                    .on_click(move |_, _, cx| { let _ = clear.update(cx, |live, cx| live.request_clear(cx)); })))
            .when(confirming, |card| card.child(super::clear_confirmation(&self.live, "确定清空全部用量记录？清空前已发起的请求不再计入统计。", clearing, super::PaneKind::Usage, p)))
            .when_some(error, |card, error| card.child(div().id("usage-statistics-error").role(Role::Alert).my(px(12.0)).text_size(px(12.0)).text_color(p.danger).child(error)))
            .when_some(stats, |card, stats| {
                let totals = &stats.totals;
                let metrics = [("总 Token", super::number(totals.total_tokens)), ("输入 Token", super::number(totals.input_tokens)), ("输出 Token", super::number(totals.output_tokens)),
                    ("请求数", super::number(totals.requests)), ("失败数", super::number(totals.failures)), ("未返回用量", super::number(totals.unknown_usage)),
                    ("缓存读取", super::number(totals.cached_input_tokens)), ("缓存写入", super::number(totals.cache_write_tokens)), ("累计耗时", seconds(totals.duration_ms))];
                card.child(super::metric_grid(115.0, 14.0).my(px(18.0)).children(metrics.into_iter().map(|(label, value)| super::metric(label, value, 18.0, p))))
                    .child(note(format!("首次使用: {} · 最近使用: {}", super::time(totals.first_used_at), super::time(totals.last_used_at)), p).mt(px(8.0)))
                    .when_some(stats.cleared_at, |card, timestamp| card.child(note(format!("统计清空时间: {}", forms::local_timestamp(timestamp)), p).mt(px(8.0))))
                    .when(totals.requests == 0, |card| card.child(note("暂无用量记录", p).mt(px(8.0))))
                    .child(super::heading("Token 用量趋势", p))
                    .child(if let Some(error) = &self.chart_error { div().text_size(px(13.0)).text_color(p.danger).child(error.clone()).into_any_element() } else { self.chart.clone().into_any_element() })
                    .when(totals.requests != 0, |card| card
                        .child(super::heading("按模型", p))
                        .child(super::table("按模型", &["模型 / 接口", "请求数", "输入 Token", "输出 Token", "总 Token", "最近使用"], stats.models.iter().map(|model| vec![
                            model_cell(&model.model, model.provider.as_str(), &model.base_url, p),
                            div().child(super::table_value("count", super::number(model.totals.requests))).child(small(format!("失败数: {} · 未返回用量: {}", model.totals.failures, model.totals.unknown_usage), p)).into_any_element(),
                            cell(super::number(model.totals.input_tokens)), cell(super::number(model.totals.output_tokens)), cell(super::number(model.totals.total_tokens)), cell(super::time(model.totals.last_used_at)),
                        ]), super::PaneKind::Usage, p))
                        .child(disclosure("token-days", "按日期", self.days_open, p).on_click(move |_, _, cx| { let _ = days.update(cx, |this, cx| { this.days_open = !this.days_open; cx.notify(); }); }))
                        .when(self.days_open, |card| card.child(super::table("按日期", &["日期", "请求数", "输入 Token", "输出 Token", "总 Token"], stats.days.iter().map(|day| vec![
                            cell(day.day.clone()), cell(super::number(day.totals.requests)), cell(super::number(day.totals.input_tokens)), cell(super::number(day.totals.output_tokens)), cell(super::number(day.totals.total_tokens)),
                        ]), super::PaneKind::Usage, p)))
                        .child(disclosure("token-recent", "最近 50 次请求", self.recent_open, p).on_click(move |_, _, cx| { let _ = recent.update(cx, |this, cx| { this.recent_open = !this.recent_open; cx.notify(); }); }))
                        .when(self.recent_open, |card| card.child(super::table("最近 50 次请求", &["时间", "模型 / 接口", "用途", "状态", "输入 Token", "输出 Token", "总 Token", "累计耗时"], stats.recent.iter().map(|entry| vec![
                            cell(forms::local_timestamp(entry.started_at)), model_cell(&entry.model, entry.provider.as_str(), &entry.base_url, p),
                            cell(if entry.purpose == AiPurpose::Ocr { "OCR" } else { "连通性测试" }.to_owned()), cell(if entry.success { "成功" } else { "失败" }.to_owned()),
                            cell(optional_number(entry.tokens.input_tokens)), cell(optional_number(entry.tokens.output_tokens)), cell(optional_number(entry.tokens.total_tokens)), cell(seconds(entry.duration_ms)),
                        ]), super::PaneKind::Usage, p))))
            })
    }
}
fn seconds(value: u64) -> String { format!("{:.2} s", value as f64 / 1000.0) }
fn optional_number(value: Option<u64>) -> String { value.map(super::number).unwrap_or_else(|| "—".to_owned()) }
fn note(text: impl Into<SharedString>, p: Palette) -> Div { super::note(text, p).line_height(relative(1.4)) }
fn cell(text: String) -> AnyElement { div().child(super::table_value("value", text)).into_any_element() }
fn small(text: String, p: Palette) -> Div { div().mt(px(4.0)).text_size(px(12.0)).text_color(p.text_secondary).overflow_wrap(OverflowWrap::Anywhere).child(super::table_value("detail", text)) }
fn model_cell(model: &str, provider: &str, url: &str, p: Palette) -> AnyElement {
    div().min_w_0().child(super::table_value("model", model.to_owned())).child(small(format!("{provider} · {url}"), p)).into_any_element()
}
fn disclosure(id: &'static str, title: &'static str, open: bool, p: Palette) -> Button {
    Button::new(id).aria_expanded(open).flex().items_center().gap(px(5.0)).mt(px(18.0)).text_size(px(13.0)).text_color(p.text_primary)
        .child(icon("chevron-right", 12.0).with_transformation(Transformation::rotate(percentage(if open { 0.25 } else { 0.0 }))))
        .child(title)
}
fn trend(stats: &AiUsageStats, period: Period) -> Result<(Vec<ChartPoint>, StatisticsInterval)> {
    let invalid = || Error::Invalid("用量统计日期无效".into());
    let from = super::range(period, None)?.0;
    if stats.days.is_empty() && from.is_none() { return Ok((Vec::new(), StatisticsInterval::Day)); }
    let first = stats.days.iter().map(|day| NaiveDate::parse_from_str(&day.day, "%Y-%m-%d").map_err(|_| invalid())).collect::<Result<Vec<_>>>()?.into_iter().min();
    let start = match from { Some(from) => chrono::DateTime::from_timestamp_millis(from).ok_or_else(invalid)?.with_timezone(&chrono::Local).date_naive(), None => first.ok_or_else(invalid)? };
    let now = chrono::Local::now();
    let interval = if now.timestamp_millis().saturating_sub(super::midnight(start)?) > 366 * 86_400_000 { StatisticsInterval::Month } else { StatisticsInterval::Day };
    let mut values = BTreeMap::<String, u64>::new();
    for day in &stats.days {
        let key = if interval == StatisticsInterval::Month { day.day[..7].to_owned() } else { day.day.clone() };
        let total = values.entry(key).or_default();
        *total = total.checked_add(day.totals.total_tokens).ok_or_else(|| Error::Invalid("Token 用量汇总溢出".into()))?;
    }
    let mut cursor = super_paste_core::calendar::period_start(super::midnight(start)?, interval)?;
    let mut points = Vec::new();
    while cursor <= now.timestamp_millis() {
        if points.len() >= 2000 { return Err(Error::ResourceLimit("用量趋势图日期范围过大".into())); }
        let local = chrono::DateTime::from_timestamp_millis(cursor).ok_or_else(invalid)?.with_timezone(&chrono::Local);
        let key = local.format(if interval == StatisticsInterval::Month { "%Y-%m" } else { "%Y-%m-%d" }).to_string();
        points.push(ChartPoint { timestamp: cursor, value: values.get(&key).copied().unwrap_or(0) as f64 });
        cursor = super_paste_core::calendar::period_next(cursor, interval)?;
    }
    Ok((points, interval))
}
