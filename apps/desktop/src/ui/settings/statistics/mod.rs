mod copy;
mod usage;
mod live;
mod chart;
mod table_borders;

use gpui_kit::{prelude::*, *};
use gpui::{OverflowWrap, TableCellAlignment};
use gpui_kit::base::Button;
use crate::ui::{forms::{self, tabular}, palette::Palette, text::normal_flow_text as table_text};
use super::PaneContext;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab { Copy, Token }
pub struct StatisticsPane { context: PaneContext, tab: Tab, body: AnyView, _appearance: Subscription }
impl StatisticsPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let owner = cx.entity().downgrade();
        let body = cx.new(|cx| copy::CopyPane::new(context.clone(), owner, window, cx)).into();
        let appearance = cx.observe_window_appearance(window, |_, _, cx| cx.notify());
        Self { context, tab: Tab::Copy, body, _appearance: appearance }
    }
    fn select(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == tab { return; }
        self.tab = tab;
        let owner = cx.entity().downgrade();
        self.body = match tab {
            Tab::Copy => cx.new(|cx| copy::CopyPane::new(self.context.clone(), owner, window, cx)).into(),
            Tab::Token => cx.new(|cx| usage::UsagePane::new(self.context.clone(), owner, window, cx)).into(),
        };
        cx.notify();
    }
}
impl Render for StatisticsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let weak = cx.entity().downgrade();
        forms::page("statistics", window).child(forms::heading("统计", "复制数据与 AI 用量", p))
            .child(div().flex().gap(px(8.0)).mb(px(20.0)).border_b_1().border_color(p.border_light)
                .children([(Tab::Copy, "复制统计"), (Tab::Token, "Token 用量")].into_iter().enumerate().map(|(index, (tab, title))| {
                    let weak = weak.clone();
                    Button::new(("statistics-tab", index)).px(px(18.0)).py(px(10.0)).border_b_2()
                        .border_color(if self.tab == tab { p.primary } else { transparent_black() })
                        .text_size(px(14.0)).text_color(if self.tab == tab { p.primary } else { p.text_secondary })
                        .child(title).on_click(move |_, window, cx| { let _ = weak.update(cx, |this, cx| this.select(tab, window, cx)); })
                })))
            .child(self.body.clone())
    }
}

fn card(p: Palette) -> Div { div().min_w_0().w_full().border_1().border_color(p.border_medium).rounded(px(8.0)).px(px(16.0)).py(px(14.0)).bg(p.bg_primary).line_height(relative(1.4)) }
fn note(text: impl Into<SharedString>, p: Palette) -> Div { div().text_size(px(12.0)).line_height(px(20.4)).text_color(p.text_secondary).child(text.into()) }
fn heading(text: &'static str, p: Palette) -> Div { div().mt(px(18.0)).mb(px(10.0)).text_size(px(14.0)).text_color(p.text_primary).child(text) }
fn number(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 { result.push(','); }
        result.push(digit as char);
    }
    result
}
fn bytes(value: u64) -> String { format_bytes(value as f64) }
fn format_bytes(value: f64) -> String {
    let mut value = value; let mut unit = 0;
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    while value >= 1024.0 && unit + 1 < units.len() { value /= 1024.0; unit += 1; }
    let digits = if unit == 0 { format!("{value:.0}") } else { format!("{value:.2}").trim_end_matches('0').trim_end_matches('.').to_owned() };
    let (whole, fraction) = digits.split_once('.').unwrap_or((&digits, ""));
    let mut grouped = String::with_capacity(digits.len() + whole.len() / 3);
    for (index, digit) in whole.bytes().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 { grouped.push(','); }
        grouped.push(digit as char);
    }
    if !fraction.is_empty() { grouped.push('.'); grouped.push_str(fraction); }
    format!("{grouped} {}", units[unit])
}
fn time(value: Option<i64>) -> String { value.map(forms::local_timestamp).unwrap_or_else(|| "—".to_owned()) }
fn type_name(kind: super_paste_format::PrimaryType) -> &'static str {
    use super_paste_format::PrimaryType::*;
    match kind { Text => "文本", Image => "图片", Html => "HTML", Url => "链接", Files => "文件", Email => "邮箱", Phone => "电话", Code => "代码", Rtf => "RTF", Color => "颜色", Empty => "其他" }
}
fn metric_grid(minimum: f32, gap: f32) -> Div {
    // The card's actual content box owns repetition, including empty-track
    // collapse. No viewport/sidebar/padding estimate or resize state is needed.
    div().w_full().min_w_0().grid().grid_cols_auto_fit(px(minimum)).gap(px(gap))
}
fn metric(label: &'static str, value: String, size: f32, p: Palette) -> Div {
    div().min_w_0().child(div().text_size(px(12.0)).text_color(p.text_secondary).child(label))
        .child(div().mt(px(6.0)).text_size(px(size)).line_height(relative(1.4)).font_features(tabular()).text_color(p.text_primary).child(value))
}
#[derive(Clone, Copy)]
enum PaneKind { Copy, Usage }
fn table_value(id: &'static str, value: impl Into<SharedString>) -> gpui::Text {
    gpui::Text::new(id.into(), table_text(value))
}

fn table(label: &'static str, headers: &[&'static str], rows: impl IntoIterator<Item = Vec<AnyElement>>, kind: PaneKind, p: Palette) -> gpui_kit::component::scroll::Scrollable<Stateful<Div>> {
    use gpui_kit::{component::scroll::ScrollableElement, base::text::editor_host::{auto_table_block, native_table_cell}};
    let columns = u16::try_from(headers.len()).expect("statistics table column count");
    assert!(columns > 0, "statistics table requires columns");
    let column_count = usize::from(columns);
    let copy = matches!(kind, PaneKind::Copy);
    let border = if copy { p.border_light } else { p.control_border };
    let alignment = if copy { TableCellAlignment::Middle } else { TableCellAlignment::Top };
    let mut grid = div().id("statistics-table").role(Role::Table).aria_label(label)
        .aria_column_count(column_count).w_full().min_w_0().grid().grid_cols_auto(columns)
        .text_size(px(12.0)).text_color(p.text_primary).overflow_wrap(OverflowWrap::Normal)
        .when(copy, |table| table.font_features(tabular()))
        .children(headers.iter().enumerate().map(|(column, label)| native_table_cell(
            div().id(("column", column)).role(Role::ColumnHeader).aria_row_index(0).aria_column_index(column)
                .min_w_0().px(px(8.0)).py(px(10.0))
                .flex().flex_col().font_weight(FontWeight::BOLD).child(div().child(table_value("value", *label))).into_any_element(), alignment)));
    let mut row_count = 1;
    for row in rows {
        assert_eq!(row.len(), column_count, "statistics table row must match its headers");
        let row_index = row_count;
        row_count += 1;
        grid = grid.children(row.into_iter().enumerate().map(move |(column, cell)| {
            let content = div().min_w_0()
                .when(column == 0 && copy, |cell| cell.overflow_wrap(OverflowWrap::Anywhere))
                .child(cell);
            native_table_cell(div().id(("cell", row_index * column_count + column)).role(Role::Cell)
                .aria_row_index(row_index).aria_column_index(column).min_w_0().px(px(8.0)).py(px(10.0))
                .flex().flex_col()
                .when(copy && column == 0, |cell| cell.max_w(px(240.0)))
                .when(!copy && column == 0, |cell| cell.min_w(px(130.0)))
                .child(content).into_any_element(), alignment)
        }));
    }
    let grid = grid.aria_row_count(row_count).a11y_synthetic_children(move |builder| {
        // Reparent the real cells; a11y-only rows add no layout boxes or
        // duplicated content. This runs only when accessibility is active.
        let cells = builder.parent_node().children().to_vec();
        assert_eq!(cells.len(), row_count * column_count, "statistics a11y cells must match the native grid");
        builder.parent_node().clear_children();
        for (index, cells) in cells.chunks_exact(column_count).enumerate() {
            let id = builder.synthetic_node_id(("statistics-row", index));
            let mut row = gpui::accesskit::Node::new(Role::Row);
            row.set_row_index(index);
            row.set_children(cells.to_vec());
            assert!(builder.push_child(id, row), "statistics a11y row ID must be unique");
        }
    });
    // All rows share native automatic tracks, with the same intrinsic text
    // contributions used for final wrapping. The scroll host owns no widths.
    div().id(headers[0]).w_full().min_w_0()
        .when(!copy, |wrap| wrap.mt(px(10.0)))
        .child(table_borders::horizontal(auto_table_block(grid.into_any_element()), column_count, border)).overflow_x_scrollbar()
}
fn clear_confirmation<Q: Clone + Send + 'static, T: Send + Sync + 'static>(live: &Entity<live::Live<Q, T>>, text: &'static str, clearing: bool, kind: PaneKind, p: Palette) -> Stateful<Div> {
    use crate::ui::widgets::TextButton;
    let confirm = live.downgrade(); let cancel = live.downgrade();
    let copy = matches!(kind, PaneKind::Copy);
    let text_size = if copy { 13.0 } else { 12.0 };
    div().id("statistics-clear-confirmation").role(Role::Alert)
        .mt(px(if copy { 16.0 } else { 12.0 })).text_size(px(text_size))
        .when(copy, |card| card.p(px(12.0)).rounded(px(8.0)).border_1().border_color(p.warning_border).bg(p.warning_bg))
        .child(div().my(px(text_size)).child(text))
        .child(div().flex().flex_wrap().gap(px(if copy { 10.0 } else { 8.0 })).mt(px(if copy { 16.0 } else { 12.0 }))
            .when(copy, |actions| actions.items_end())
            .child(TextButton::new("confirm-clear-statistics", "确认清空", p).neutral().disabled(clearing)
                .on_click(move |_, window, cx| { let _ = confirm.update(cx, |live, cx| live.confirm_clear(window, cx)); }))
            .child(TextButton::new("cancel-clear-statistics", "取消", p).neutral().disabled(clearing)
                .on_click(move |_, _, cx| { let _ = cancel.update(cx, |live, cx| live.cancel_clear(cx)); })))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Period { Today, Week, Month, All, Custom }
fn midnight(date: chrono::NaiveDate) -> super_paste_core::Result<i64> {
    super_paste_core::calendar::day_start(date)
}
fn range(period: Period, custom: Option<(chrono::NaiveDate, chrono::NaiveDate)>) -> super_paste_core::Result<(Option<i64>, Option<i64>)> {
    use chrono::Days;
    if period == Period::All { return Ok((None, None)); }
    let invalid = || super_paste_core::Error::Invalid("请选择有效的起止日期".into());
    let today = chrono::Local::now().date_naive();
    let (start, end) = match period {
        Period::Custom => custom.filter(|(start, end)| start <= end).ok_or_else(invalid)?,
        Period::Week => (today.checked_sub_days(Days::new(6)).ok_or_else(invalid)?, today),
        Period::Month => (today.checked_sub_days(Days::new(29)).ok_or_else(invalid)?, today),
        _ => (today, today),
    };
    let end = end.succ_opt().ok_or_else(invalid)?;
    let from = midnight(start)?;
    let local_start = chrono::DateTime::from_timestamp_millis(from).ok_or_else(invalid)?.with_timezone(&chrono::Local);
    if local_start.date_naive() != start { return Err(super_paste_core::Error::Invalid("所选日期在本地时区不存在".into())); }
    Ok((Some(from), Some(midnight(end)?)))
}
