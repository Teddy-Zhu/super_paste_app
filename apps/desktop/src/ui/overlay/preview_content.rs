use gpui_kit::{prelude::*, *};
use std::sync::Arc;
use gpui_kit::base::{Button, TextView, TextViewState, TextViewStyle};
use gpui_kit::base::text::editor_host::{BlockNode, InlineNode, Paragraph, TextMark};
use gpui_kit::component::tooltip::Tooltip;
use super_paste_core::preview::{FilePreview, PreviewBody, PreviewKind, PreviewModel, PreviewNotice, excerpt, text_whitespace};
use super_paste_format::{HistoryEntry, RecordId};
use crate::ui::{icon, palette::Palette, text::{normal_flow_text, preformatted_text, trimmed_normal_flow_text}};
use super::OverlayView;

impl OverlayView {
    pub(super) fn card_content(&self, entry: &HistoryEntry, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let id = entry.id;
        let Some(preview) = self.previews.get(&id) else { return div().into_any_element(); };
        if let Some(error) = &preview.error {
            return div().size_full().flex().flex_col().items_center().justify_center().gap(px(8.0)).text_size(px(11.0)).text_color(p.danger)
                .child(div().max_h(px(70.0)).overflow_hidden().child(error.clone()))
                .child(Button::new("retry-preview").accessibility_label("重试预览").px(px(8.0)).py(px(4.0)).rounded(px(4.0)).border_1().border_color(p.control_border)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child("重试").on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.retry_preview(id, window, cx); }))).into_any_element();
        }
        let Some(model) = &preview.model else {
            return div().size_full().flex().flex_col().gap(px(10.0)).pt(px(6.0))
                .children((0..3).map(|index| div().h(px(10.0)).flex_shrink_0().rounded(px(3.0)).bg(p.bg_secondary)
                    .when(index == 2, |bar| bar.w(relative(0.6))))).into_any_element();
        };
        let base = div().size_full().min_w_0().min_h_0().flex().flex_col().gap(px(10.0)).text_size(px(14.0)).line_height(relative(1.65))
            .when_some(model.notice, |body, message| body.child(notice(message.text(), p)));
        let body = match &model.body {
            PreviewBody::Image => {
                let image = div().relative().flex_1().min_h_0().min_w_0().rounded(px(8.0)).overflow_hidden()
                    .child(checkerboard(p, 8.0)).when_some(preview.image.clone(), |frame, image| frame.child(img(image).absolute().inset_0().size_full().rounded(px(8.0)).object_fit(ObjectFit::Contain)))
                    .when_some(preview.image_error.as_ref(), |frame, error| frame.child(image_failure(error.clone(), p).absolute().inset_0()));
                base.child(image)
                    .when_some(preview.caption.as_ref(), |body, caption| body.child(div().flex_shrink_0().max_h(relative(0.35)).overflow_hidden().border_t_1().border_color(p.border_light).pt(px(6.0))
                        .child(div().text_size(px(10.0)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_secondary).child("附带文本"))
                        .child(div().mt(px(2.0)).text_size(px(12.0)).line_height(relative(1.5)).line_clamp(3)
                            .overflow_wrap(OverflowWrap::Anywhere).map(|text| super::rich::automatic_text(text, caption))
                            .child(caption.clone()))))
                    .when_some(model.files.first(), |body, file| {
                        let path = SharedString::from(file.path.clone());
                        let name = &preview.file_labels.as_ref().expect("loaded image files have prepared labels")[0].name;
                        body.child(div().id("image-file").min_w_0().flex_shrink_0().flex().items_center().gap(px(4.0)).text_size(px(11.0)).text_color(p.text_secondary)
                            .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
                            .child(icon("file-outline", 11.0).flex_shrink_0()).child(div().flex_1().min_w_0().text_ellipsis().child(name.clone())))
                    })
            },
            PreviewBody::Files => {
                let labels = preview.file_labels.as_ref().expect("loaded file previews have prepared labels");
                base.child(div().min_h_0().overflow_hidden().flex().flex_col().gap(px(10.0)).children(model.files.iter().take(4).enumerate().map(|(index, file)| {
                    let label = &labels[index];
                    let path = SharedString::from(file.path.clone());
                    let badge = div().w(px(40.0)).flex_shrink_0().flex().flex_col().items_center().text_color(p.type_teal)
                        .child(icon("file-outline", 28.0)).child(div().max_w_full().text_ellipsis().text_size(px(9.0)).line_height(relative(1.3)).child(label.extension.clone()));
                    let info = div().min_w_0().child(div().line_clamp(2).font_weight(FontWeight::SEMIBOLD).overflow_wrap(OverflowWrap::Anywhere)
                        .map(|element| super::rich::automatic_text(element, &label.name)).child(label.name.clone()))
                        .when(!file.directory.is_empty(), |row| row.child(div().text_ellipsis().text_size(px(11.0)).text_color(p.text_secondary)
                            .map(|element| super::rich::automatic_text(element, &label.directory)).child(label.directory.clone())));
                    div().id(("file-row", index)).min_w_0().flex_shrink_0().py(px(8.0)).flex().items_center().gap(px(10.0))
                        .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx)).child(badge).child(info)
            }))).when(model.files_total > model.files.len() as u64, |body| body.child(div().flex_shrink_0().text_size(px(11.0)).text_color(p.text_tertiary).child(format!("+{} 个文件", model.files_total - model.files.len() as u64))))
            },
            PreviewBody::Color(color) => {
                base.child(color_swatch(color, p).flex_1())
                    .child(div().flex_shrink_0().font_family("monospace").text_size(px(18.0)).text_center().child(SharedString::from(color.original.clone())))
                    .child(div().flex_shrink_0().font_family("monospace").text_size(px(12.0)).text_center().text_color(p.text_secondary).child(SharedString::from(color.rgb.clone())))
                    .child(div().flex_shrink_0().flex().justify_center().gap(px(8.0)).child(color_button(id, false, preview.copying, p, cx)).child(color_button(id, true, preview.copying, p, cx)))
                    .child(div().flex_shrink_0().h(px(16.0)).text_center().text_size(px(11.0)).line_height(px(16.0)).text_color(p.text_secondary)
                        .id("color-copy-status").role(Role::Status).child(preview.copied.map(|format| format!("已复制 {format}")).unwrap_or_default()))
            },
            PreviewBody::Link(link) => base
                .child(div().flex_shrink_0().flex().items_start().gap(px(8.0)).text_size(px(18.0)).text_color(p.type_orange)
                    .child(icon("link-variant", 18.0).flex_shrink_0().mt(px(5.0))).child(div().flex_1().min_w_0().font_weight(FontWeight::BOLD).overflow_wrap(OverflowWrap::Anywhere).child(SharedString::from(link.host.clone()))))
                .child(div().min_h_0().min_w_0().overflow_hidden().map(|element| preview.text.as_ref().expect("loaded links have prepared text").align(element))
                    .child(TextView::new(&preview.text.as_ref().expect("loaded links have a path text state").state).selectable(false).scrollable(false)
                        .style(TextViewStyle::default().with_foreground(p.text_primary))))
                .child(div().mt_auto().min_h_0().overflow_hidden().text_size(px(11.0)).text_color(p.text_secondary)
                    .overflow_wrap(OverflowWrap::Anywhere).map(|element| super::rich::automatic_text(element, &link.href))
                    .child(preview.href.as_ref().expect("loaded links have prepared URL text").clone())),
            PreviewBody::Contact => base
                .child(div().mt_auto().flex_shrink_0().flex().justify_center().text_color(p.type_red).child(icon(if model.kind == PreviewKind::Email { "at" } else { "phone-outline" }, 40.0)))
                .child(div().mb_auto().min_h_0().min_w_0().overflow_hidden().text_size(px(21.0)).font_weight(FontWeight::MEDIUM)
                    .map(|element| preview.text.as_ref().expect("loaded contacts have prepared text").align(element).text_center())
                    .child(TextView::new(&preview.text.as_ref().expect("loaded contacts have a text state").state).selectable(false).scrollable(false)
                        .style(TextViewStyle::default().with_foreground(p.text_primary)))),
            PreviewBody::Table(table) => {
                let truncated = table.rows.len() > 8 || table.columns > 4;
                let columns = table.rows.iter().take(8).map(|row| row.len().min(4)).max().unwrap_or(0);
                let mut grid = div().min_h_0().w_full().overflow_hidden().text_size(px(12.0)).grid()
                    .grid_cols_with_leading((columns + 1) as u16, px(28.0))
                    .border_t_1().border_l_1().border_color(p.control_border);
                for (row_index, row) in table.rows.iter().take(8).enumerate() {
                    grid = grid.child(div().min_w_0().col_start(1).row_start(row_index as i16 + 1)
                        .border_r_1().border_b_1().border_color(p.control_border).px(px(3.0)).py(px(5.0))
                        .bg(p.bg_secondary).text_center().text_size(px(10.0)).font_weight(FontWeight::BOLD).text_color(p.text_secondary)
                        .child((row_index + 1).to_string()));
                    for (column, cell) in row.iter().take(4).enumerate() {
                        let text = SharedString::from(cell.clone()); let tooltip = text.clone();
                        grid = grid.child(div().id(("table-cell", row_index * 4 + column)).min_w_0().overflow_hidden()
                            .col_start(column as i16 + 2).row_start(row_index as i16 + 1)
                            .border_r_1().border_b_1().border_color(p.control_border).px(px(8.0)).py(px(5.0))
                            .when(row_index % 2 == 1, |cell| cell.bg(p.bg_tertiary))
                            .map(|element| super::rich::automatic_text(element, cell))
                            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)).child(text));
                    }
                }
                base.child(grid).when(truncated, |body| body.child(notice(PreviewNotice::Excerpt.text(), p)))
            },
            PreviewBody::Empty => base.justify_center().items_center().text_color(p.text_secondary).child("暂无可预览内容"),
            PreviewBody::Text | PreviewBody::Code => text_content(base, model, preview.text.as_ref().expect("literal previews have a text state"), p),
            PreviewBody::Rich(_) => if let Some(rich) = &preview.rich {
                base.child(div().flex_1().min_h_0().overflow_hidden().child(rich.render(window, matches!(self.phase, super::Phase::Opening | super::Phase::Opened | super::Phase::Closing), cx)))
                    .when(rich.truncated, |body| body.child(notice(PreviewNotice::Excerpt.text(), p)))
                    .when(rich.image_failed(), |body| body.child(notice("部分图片无法预览", p)))
            } else { text_content(base, model, preview.text.as_ref().expect("unavailable rich previews have a literal text state"), p).child(notice(PreviewNotice::RichUnavailable.text(), p)) },
        };
        body.when(matches!(&model.body, PreviewBody::Text | PreviewBody::Code | PreviewBody::Rich(_)), |body| {
                body.relative().child(bottom_fade(p))
            }).into_any_element()
    }
}
fn text_content(base: Div, model: &PreviewModel, text: &PreparedText, p: Palette) -> Div {
    base.when(text.whitespace, |body| body.child(notice("空白文本", p)))
        .child(div().min_h_0().overflow_hidden().when(text.compact, |text| text.mt_auto().mb_auto().text_size(px(22.0)).line_height(relative(1.5)).font_weight(FontWeight::MEDIUM))
            .when(matches!(model.kind, PreviewKind::Code | PreviewKind::Json), |text| text.font_family("monospace").text_size(px(12.0)).line_height(relative(1.7)).p(px(10.0)).bg(p.bg_secondary).rounded(px(8.0)))
            .map(|element| text.align(element))
            .child(TextView::new(&text.state).selectable(false).scrollable(false)
                .style(TextViewStyle::default().with_foreground(p.text_primary))))
}

/// Immutable presentation belongs to the same visible source as its native
/// text state. Theme and fonts stay in the renderer's existing layout keys.
#[derive(Clone)]
pub(super) struct PreparedText {
    state: Entity<TextViewState>, direction: TextDirection, alignment: TextAlign,
    compact: bool, whitespace: bool,
}
impl PreparedText {
    pub(super) fn new(owner: &Arc<PreviewModel>, cx: &mut App) -> Self {
        let (state, source, literal) = match &owner.body {
            PreviewBody::Link(link) => (literal_state(shared_excerpt(&link.detail, 12_000), owner, cx), link.detail.as_ref(), false),
            PreviewBody::Contact => (contact_state(owner, cx), owner.text.as_ref(), false),
            _ => (literal_state(display_text(&owner.display_text), owner, cx), owner.display_text.as_ref(), true),
        };
        let (direction, alignment) = super::rich::automatic_style(source);
        Self { state, direction, alignment, compact: literal && compact_text(source),
            whitespace: literal && source.trim_matches(text_whitespace).is_empty() }
    }
    fn align<T: Styled>(&self, element: T) -> T {
        element.text_direction(self.direction).text_align(self.alignment)
    }
}

pub(super) fn image_failure(message: SharedString, p: Palette) -> Stateful<Div> {
    div().id("image-preview-failure").size_full().min_h_0().rounded(px(8.0)).flex().items_center().justify_center()
        .p(px(16.0)).text_size(px(14.0)).text_center().text_color(p.danger).bg(p.danger_bg)
        .role(Role::Status).child(message)
}
pub(super) fn compact_text(text: &str) -> bool { text.encode_utf16().take(81).count() <= 80 && !text.contains(['\r', '\n']) }
pub(super) fn display_text(text: &Arc<str>) -> SharedString {
    if text.is_empty() || !text.trim_matches(text_whitespace).is_empty() { return SharedString::from(text.clone()); }
    let mut shown = String::with_capacity(text.len()); let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            ' ' => shown.push('·'), '\t' => shown.push_str("→   "),
            '\r' => { if characters.peek() == Some(&'\n') { characters.next(); } shown.push_str("↵\n"); },
            '\n' => shown.push_str("↵\n"), _ => shown.push(character),
        }
    }
    shown.into()
}

/// One native layout/selection owner per visible source, shared by card and
/// detail construction. Formatting is prepared here, never during rendering.
pub(super) fn literal_state(source: SharedString, owner: &Arc<PreviewModel>, cx: &mut App) -> Entity<TextViewState> {
    text_state(source, owner, TextSpacePolicy::PreWrap, cx)
}

pub(super) fn contact_state(owner: &Arc<PreviewModel>, cx: &mut App) -> Entity<TextViewState> {
    let source = trimmed_normal_flow_text(SharedString::from(owner.text.clone()));
    text_state(source, owner, TextSpacePolicy::Collapse, cx)
}

fn text_state(source: SharedString, owner: &Arc<PreviewModel>, policy: TextSpacePolicy, cx: &mut App) -> Entity<TextViewState> {
    // DOM text preserves CR, but CSS treats it as space. Keep the authoritative
    // payload untouched and normalize only the selection view when needed.
    let source = preformatted_text(source);
    let mut inline = InlineNode::new(source.clone());
    if !source.is_empty() {
        inline = inline.marks(vec![(0..source.len(), TextMark { space_policy: Some(policy), ..TextMark::default() })]);
    }
    let blocks = Arc::new(vec![BlockNode::Paragraph(Paragraph::from_runs(vec![inline]))]);
    let owner = owner.clone();
    cx.new(|cx| TextViewState::from_owned_blocks(source, blocks, owner, cx))
}

pub(super) fn shared_excerpt(source: &Arc<str>, limit: usize) -> SharedString {
    let clipped = excerpt(source, limit);
    if clipped.len() == source.len() { SharedString::from(source.clone()) } else { SharedString::new(clipped) }
}

pub(super) struct FileLabels { pub name: SharedString, pub directory: SharedString, pub extension: SharedString }
impl FileLabels {
    pub fn new(file: &FilePreview) -> Self {
        Self { name: normal_flow_text(SharedString::from(file.name.clone())),
            directory: normal_flow_text(SharedString::from(file.directory.clone())),
            extension: normal_flow_text(SharedString::from(file.extension.clone())) }
    }
}
pub(super) fn file_labels(model: &PreviewModel) -> Option<Arc<[FileLabels]>> {
    if model.files.is_empty() { return None; }
    match &model.body {
        PreviewBody::Files => Some(model.files.iter().take(4).map(FileLabels::new).collect::<Vec<_>>().into()),
        PreviewBody::Image => Some(vec![FileLabels { name: normal_flow_text(SharedString::from(model.files[0].name.clone())),
            directory: SharedString::default(), extension: SharedString::default() }].into()),
        _ => None,
    }
}

pub(super) fn image_caption(model: &PreviewModel) -> Option<SharedString> {
    if !matches!(&model.body, PreviewBody::Image) || model.text.trim_matches(text_whitespace).is_empty() { return None; }
    let source = excerpt(&model.text, 300);
    Some(if source.contains('\r') { SharedString::from(source.replace('\r', " ")) }
        else if source.len() == model.text.len() { SharedString::from(model.text.clone()) }
        else { SharedString::new(source) })
}
fn notice(text: &'static str, p: Palette) -> Div { div().flex_shrink_0().text_size(px(11.0)).text_color(p.text_tertiary).child(text) }
fn bottom_fade(p: Palette) -> impl IntoElement {
    // The card body has a uniform opaque bg_primary. Compositing this gradient
    // gives the Web mask's result without a separate rendered-texture allocation.
    // Canvas adds no hitbox, so text/content interaction still reaches the card.
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        window.paint_quad(fill(bounds, linear_gradient(180.0,
            linear_color_stop(p.bg_primary.opacity(0.0), 0.0), linear_color_stop(p.bg_primary, 1.0))));
    }).absolute().left_0().right_0().bottom_0().h(px(14.0))
}
fn color_button(id: RecordId, rgb: bool, disabled: bool, p: Palette, cx: &mut Context<OverlayView>) -> Button {
    color_copy_button(if rgb { "color-rgb" } else { "color-hex" }, rgb, disabled, p)
        .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.copy_color(id, rgb, window, cx); }))
}

pub(super) fn color_swatch(color: &super_paste_core::preview::ColorPreview, p: Palette) -> Div {
    let [r, g, b, a] = color.rgba;
    let fill = Hsla::from(Rgba { r, g, b, a });
    div().relative().min_h(px(48.0)).rounded(px(10.0)).overflow_hidden().border_1().border_color(p.border_light)
        .when(a < 1.0, |swatch| swatch.child(checkerboard(p, 10.0)))
        .child(div().absolute().inset_0().rounded(px(10.0)).bg(fill))
}

pub(super) fn color_copy_button(id: impl Into<ElementId>, rgb: bool, disabled: bool, p: Palette) -> Button {
    Button::new(id).disabled(disabled).px(px(8.0)).py(px(4.0)).border_1().border_color(p.control_border)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .rounded(px(4.0)).bg(p.bg_secondary).text_color(p.text_primary).text_size(px(11.0)).hover(move |button| button.bg(p.selection).text_color(p.primary))
        .child(if rgb { "复制为 RGB" } else { "复制为 HEX" })
}
pub(super) fn checkerboard(p: Palette, radius: f32) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let radius = px(radius);
        let corners = Corners { top_left: radius, top_right: radius, bottom_right: radius, bottom_left: radius }
            .clamp_radii_for_quad_size(bounds.size);
        let mut background = fill(bounds, p.bg_primary); background.corner_radii = corners;
        window.paint_quad(background);
        // The native shader anchors an 8px square at the top-left, matching the
        // original 16px conic-gradient tile without hundreds of per-cell quads.
        // Shader coordinates are device pixels; keep the square 8 logical pixels.
        let mut pattern = fill(bounds, gpui_kit::checkerboard(p.border_light, 8.0 * window.scale_factor()));
        pattern.corner_radii = corners;
        window.paint_quad(pattern);
    }).absolute().inset_0()
}
