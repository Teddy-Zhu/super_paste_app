use std::{cell::RefCell, rc::Rc, sync::Arc};
use chrono::Datelike;
use gpui_kit::{prelude::*, *};
use gpui_kit::component::tooltip::Tooltip;
use super_paste_format::StatisticsInterval;
use crate::ui::{configuration::ConfigurationGlobal, palette::Palette};

#[derive(Clone, Copy, PartialEq)]
pub struct ChartPoint { pub timestamp: i64, pub value: f64 }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ValueFormat { Number, Bytes }
impl ValueFormat {
    fn format(self, value: f64) -> String { match self { Self::Number => super::number(value.round() as u64), Self::Bytes => super::format_bytes(value) } }
}
struct Label { line: ShapedLine, origin: Point<Pixels>, height: Pixels }
struct Geometry {
    bounds: Bounds<Pixels>, font: Font, font_generation: usize, foreground: Hsla, primary: Hsla, border: Hsla,
    paths: Vec<(Path<Pixels>, Hsla)>, labels: Vec<Label>, points: Vec<Point<Pixels>>, scale: f32, error: Option<String>,
}
pub struct Chart {
    id: ElementId, points: Arc<[ChartPoint]>, label: SharedString, interval: StatisticsInterval, format: ValueFormat,
    cache: Rc<RefCell<Option<Rc<Geometry>>>>,
}
impl Chart {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), points: Arc::from([]), label: "复制次数".into(), interval: StatisticsInterval::Day, format: ValueFormat::Number, cache: Rc::new(RefCell::new(None)) }
    }
    pub fn set(&mut self, points: Vec<ChartPoint>, label: &'static str, interval: StatisticsInterval, format: ValueFormat, cx: &mut Context<Self>) {
        if self.points.as_ref() == points.as_slice() && self.label.as_ref() == label && self.interval == interval && self.format == format { return; }
        self.points = points.into(); self.label = label.into(); self.interval = interval; self.format = format;
        *self.cache.borrow_mut() = None;
        cx.notify();
    }
}
impl Render for Chart {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = cx.global::<ConfigurationGlobal>().0.read(cx).projected.settings.color_mode;
        let p = Palette::for_window(mode, window);
        if self.points.is_empty() {
            return div().w_full().my(px(12.0)).py(px(35.0)).text_center().text_size(px(13.0)).text_color(p.text_secondary).child("暂无趋势数据").into_any_element();
        }
        let points = self.points.clone(); let cache = self.cache.clone(); let label = self.label.clone(); let interval = self.interval; let format = self.format;
        div().id(self.id.clone()).w_full().aspect_ratio(680.0 / 228.0).min_h(px(160.0)).max_h(px(320.0)).my(px(12.0))
            .child(canvas(move |bounds, window, cx| {
                let mut font = window.text_style().font();
                let mut features = font.features.0.as_ref().clone();
                features.retain(|(tag, _)| tag != "tnum"); features.push(("tnum".to_owned(), 1)); font.features = FontFeatures(Arc::new(features));
                let font_generation = window.text_system().font_generation();
                let geometry = {
                    let current = cache.borrow();
                    current.as_ref().filter(|frame| frame.bounds == bounds && frame.font == font
                        && frame.font_generation == font_generation && frame.foreground == p.text_secondary
                        && frame.primary == p.primary && frame.border == p.border_light).cloned()
                }.unwrap_or_else(|| {
                    let geometry = Rc::new(geometry(bounds, font, font_generation, &points, interval, format, p, window));
                    *cache.borrow_mut() = Some(geometry.clone()); geometry
                });
                let mut elements = Vec::with_capacity(points.len());
                if geometry.error.is_none() {
                    for (index, (point, origin)) in points.iter().zip(&geometry.points).enumerate() {
                        let scale = geometry.scale; let point = *point; let title = label.clone();
                        let mut element = div().id(("trend-point", index)).size(px(6.0 * scale)).rounded_full().bg(p.primary)
                            .hover(move |circle| circle.size(px(10.0 * scale)).relative().left(px(-2.0 * scale)).top(px(-2.0 * scale)))
                            .tooltip(move |window, cx| Tooltip::new(format!("{} · {}: {}", crate::ui::forms::local_timestamp(point.timestamp), title, format.format(point.value))).build(window, cx))
                            .into_any_element();
                        element.layout_as_root(size(AvailableSpace::MinContent, AvailableSpace::MinContent), window, cx);
                        element.prepaint_at(*origin - point2(3.0 * scale, 3.0 * scale), window, cx);
                        elements.push(element);
                    }
                }
                (geometry, elements)
            }, |_, (geometry, mut elements), window, cx| {
                for (path, color) in &geometry.paths { window.paint_path(path.clone(), *color); }
                for label in &geometry.labels {
                    if let Err(error) = label.line.paint(label.origin, label.height, TextAlign::Left, None, window, cx) { tracing::warn!(%error, "绘制统计图标签失败"); }
                }
                for element in &mut elements { element.paint(window, cx); }
            }).size_full()).into_any_element()
    }
}
fn point2(x: f32, y: f32) -> Point<Pixels> { point(px(x), px(y)) }
fn geometry(bounds: Bounds<Pixels>, font: Font, font_generation: usize, points: &[ChartPoint], interval: StatisticsInterval, format: ValueFormat, p: Palette, window: &Window) -> Geometry {
    let scale = (f32::from(bounds.size.width) / 680.0).min(f32::from(bounds.size.height) / 228.0);
    let origin = bounds.origin + point2((f32::from(bounds.size.width) - 680.0 * scale) / 2.0, (f32::from(bounds.size.height) - 228.0 * scale) / 2.0);
    let mut geometry = Geometry { bounds, font, font_generation, foreground: p.text_secondary, primary: p.primary, border: p.border_light, paths: Vec::new(), labels: Vec::new(), points: Vec::new(), scale, error: None };
    let max = (points.iter().map(|point| point.value).fold(0.0_f64, f64::max) / 4.0).ceil().max(1.0) * 4.0;
    for tick in 0..5 {
        let value = max * tick as f64 / 4.0;
        let y = 186.0 - (value / max * 164.0) as f32;
        let mut path = PathBuilder::stroke(px(scale)).dash_array(&[px(3.0 * scale), px(4.0 * scale)]);
        path.move_to(origin + point2(64.0 * scale, y * scale)); path.line_to(origin + point2(646.0 * scale, y * scale));
        add_path(&mut geometry, path, p.border_light);
        add_label(&mut geometry, format.format(value), origin + point2(56.0 * scale, (y + 4.0) * scale), TextAlign::Right, window);
    }
    let options = StrokeOptions::default().with_line_width(2.0 * scale).with_line_join(lyon::path::LineJoin::Round);
    let mut path = PathBuilder::stroke(px(2.0 * scale)).with_style(PathStyle::Stroke(options));
    for (index, point) in points.iter().enumerate() {
        let x = if points.len() <= 1 { 360.0 } else { 64.0 + index as f32 / (points.len() - 1) as f32 * 582.0 };
        let y = 186.0 - (point.value / max * 164.0) as f32;
        let at = origin + point2(x * scale, y * scale);
        geometry.points.push(at);
        if index == 0 { path.move_to(at); } else { path.line_to(at); }
    }
    if points.len() > 1 { add_path(&mut geometry, path, p.primary); }
    let mut indices = vec![0, (points.len() - 1) / 2, points.len() - 1]; indices.dedup();
    for index in indices {
        let text = date_label(points[index].timestamp, interval);
        let x = geometry.points[index].x;
        add_label(&mut geometry, text, point(x, origin.y + px(214.0 * scale)), if index == 0 { TextAlign::Left } else if index == points.len() - 1 { TextAlign::Right } else { TextAlign::Center }, window);
    }
    if let Some(error) = &geometry.error {
        tracing::warn!(%error, "准备统计趋势图失败");
        geometry.paths.clear(); geometry.labels.clear(); geometry.points.clear();
        add_label(&mut geometry, "趋势图无法绘制".to_owned(), origin + point2(340.0 * scale, 114.0 * scale), TextAlign::Center, window);
    }
    geometry
}
fn add_path(geometry: &mut Geometry, builder: PathBuilder, color: Hsla) {
    match builder.build() { Ok(path) => geometry.paths.push((path, color)), Err(error) => geometry.error = Some(error.to_string()) }
}
fn add_label(geometry: &mut Geometry, text: String, baseline: Point<Pixels>, align: TextAlign, window: &Window) {
    let run = TextRun { len: text.len(), font: geometry.font.clone(), color: geometry.foreground, ..Default::default() };
    let line = window.text_system().shape_line(text.into(), px(11.0 * geometry.scale), &[run], None);
    let x = baseline.x - match align { TextAlign::Right => line.width, TextAlign::Center => line.width / 2.0, _ => px(0.0) };
    let height = line.ascent + line.descent;
    geometry.labels.push(Label { origin: point(x, baseline.y - line.ascent), line, height });
}
fn date_label(timestamp: i64, interval: StatisticsInterval) -> String {
    let Some(date) = chrono::DateTime::from_timestamp_millis(timestamp) else { return "无效日期".to_owned(); };
    let date = date.with_timezone(&chrono::Local);
    match interval {
        StatisticsInterval::Hour => date.format("%-m/%-d %H:%M").to_string(),
        StatisticsInterval::Month => format!("{}/{}", date.year(), date.month()),
        StatisticsInterval::Day => format!("{}/{}", date.month(), date.day()),
    }
}
