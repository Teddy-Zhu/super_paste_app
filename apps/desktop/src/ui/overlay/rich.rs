mod table;
mod images;
mod lists;
mod files;
mod rule;
mod variables;
mod colors;

use std::sync::Arc;
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{TextView, TextViewState, TextViewStyle};
use gpui_kit::base::text::editor_host::BlockFlow;
use gpui_kit::component::Theme;
use gpui_kit::base::text::editor_host::{BlockNode, DecorationFont, DecorationFontMetric, DecorationMetric, DecorationMetrics, DecorationOrigin, DecorationStyles, DecorationViewportAxis, used_decoration_thickness, ImageNode, ImageVerticalAlign, ImageVerticalStyle, InlineBaselineOffset, InlineBox, InlineVerticalAlign, InlineNode, ListMarker, LogicalAlignment, Paragraph, TextMark};
pub(super) use images::{LoadedFrame as RichImageFrame, Response as ImageResponse, Sources as ImageSources, StandaloneImage, display_spec, load as load_images};
use super_paste_core::preview::rich::{Alignment, CssColor, Decoration, DecorationPattern, DecorationThickness, Direction, Document, Element, ElementKind as Kind, FontStyle as RichFontStyle, LengthPercentage, LengthUnit, Node, ListStyle, StyleProperty as Property, StyleReset, VerticalAlignment, WhiteSpace, Weight};

const BODY_FONT_SIZE: f32 = 14.0;
const TABLE_FONT_SIZE: f32 = 12.0;
const LINE_HEIGHT: f32 = 1.65;
// The reference page styles body, not html. Its root keeps the WebView's
// initial 16px size and platform default font, independently of the Kit rem.
const ROOT_FONT_SIZE: f32 = 16.0;
const ROOT_FONT_FAMILY: &str = if cfg!(target_os = "windows") { "Times New Roman" }
    else if cfg!(target_os = "macos") { "Times" } else { "sans-serif" };

pub(super) fn automatic_style(text: &str) -> (TextDirection, TextAlign) {
    if Direction::first_strong(text) == Some(Direction::Rtl) {
        (TextDirection::Rtl, TextAlign::Right)
    } else {
        (TextDirection::Ltr, TextAlign::Left)
    }
}

pub(super) fn automatic_text<T: Styled>(element: T, source: &str) -> T {
    let (direction, alignment) = automatic_style(source);
    element.text_direction(direction).text_align(alignment)
}

#[derive(Clone)]
pub(super) struct RichCard { state: Entity<TextViewState>, pub truncated: bool, paper: bool,
    images: images::Sources, _image_owner: Arc<images::Owner>, variables: Option<Arc<VariableContext>> }
struct VariableContext {
    document: Arc<Document>, nodes: usize, text: usize,
    exact_font_style: bool,
    dark: std::sync::atomic::AtomicBool, failed: std::sync::atomic::AtomicBool,
}
impl RichCard {
    /// The core document and its bytes stay shared. Only native layout and
    /// selection state are private to this visible card.
    pub fn new(document: &Arc<Document>, cx: &mut App) -> Option<Self> {
        Self::with_limits(document, 2_000, 12_000, false, cx)
    }
    pub fn details(document: &Arc<Document>, cx: &mut App) -> Option<Self> {
        Self::with_limits(document, usize::MAX, usize::MAX, true, cx)
    }
    pub fn data_table(model: &Arc<super_paste_core::preview::PreviewModel>, cx: &mut App) -> Option<Self> {
        let super_paste_core::preview::PreviewBody::Table(table) = &model.body else { return None; };
        let block = table::from_preview(table)?;
        let source = SharedString::from(model.text.clone()); let owner = model.clone();
        let state = cx.new(|cx| TextViewState::from_owned_blocks(source, Arc::new(vec![block]), owner, cx));
        let images = images::Sources::default(); let image_owner = images.owner();
        Some(Self { state, truncated: false, paper: false, images, _image_owner: image_owner, variables: None })
    }
    pub fn files(model: &Arc<super_paste_core::preview::PreviewModel>, cx: &mut App) -> Self {
        let block = files::block(model.files.clone());
        let source = SharedString::from(model.text.clone()); let owner = model.clone();
        let state = cx.new(|cx| TextViewState::from_owned_blocks(source, Arc::new(vec![block]), owner, cx));
        let images = images::Sources::default(); let image_owner = images.owner();
        Self { state, truncated: false, paper: false, images, _image_owner: image_owner, variables: None }
    }
    fn with_limits(document: &Arc<Document>, nodes_left: usize, text_left: usize, exact_font_style: bool, cx: &mut App) -> Option<Self> {
        let images = images::Sources::default();
        let dark = Theme::global(cx).mode.is_dark();
        let variables = document.has_style_variables().then(|| Arc::new(VariableContext {
            document: document.clone(), nodes: nodes_left, text: text_left,
            exact_font_style,
            dark: std::sync::atomic::AtomicBool::new(dark), failed: std::sync::atomic::AtomicBool::new(false),
        }));
        let mut adapter = Adapter { dark, restyle_variables: variables.is_some(), exact_font_style, image_index: 0, nodes_left, text_left, truncated: false, unsupported: false, images: images.clone(),
            mono_family: Theme::global(cx).mono_font_family.clone() };
        let blocks = Arc::new(adapter.blocks(&document.nodes, &Inherited::default(), true));
        if adapter.unsupported { return None; }
        let owner = document.clone();
        let state = cx.new(|cx| TextViewState::from_owned_blocks(SharedString::from(document.text.clone()), blocks, owner, cx));
        let image_owner = images.owner();
        Some(Self { state, truncated: adapter.truncated, paper: true, images, _image_owner: image_owner, variables })
    }
    pub fn has_images(&self) -> bool { self.images.has_images() }
    pub fn image_sources(&self) -> images::Sources { self.images.clone() }
    pub fn wants_image(&self, key: &SharedString, generation: u64) -> bool { self.images.wants(key, generation) }
    pub fn retain_pixels(&self, alive: &mut std::collections::HashSet<super_paste_format::Digest>) { self.images.retain_pixels(alive); }
    pub fn image_failed(&self) -> bool { self.images.failed() }
    pub fn images_pending(&self) -> bool { self.images.visible_pending() }
    pub fn release_images(&self) { self.images.release_visible(); }
    pub fn image_loader_failed(&self) { self.images.fail_pending("图片加载任务异常结束".into()); }
    pub fn image_loader_timed_out(&self) { self.images.fail_pending("内嵌图片加载超时".into()); }
    pub fn accept_image(&self, key: SharedString, generation: u64, result: super_paste_core::Result<RichImageFrame>, cx: &mut App) {
        if self.images.accept(key, generation, result) {
            self.state.update(cx, |state, cx| state.invalidate_inline_layout(cx));
        }
    }
    fn refresh_variables(&self, window: &Window, cx: &mut App) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        let Some(context) = &self.variables else { return true; };
        if context.failed.load(Relaxed) { return false; }
        let mode = cx.global::<crate::ui::configuration::ConfigurationGlobal>().0.read(cx).projected.settings.color_mode;
        let dark = crate::ui::palette::Palette::for_window(mode, window).dark;
        if dark == context.dark.load(Relaxed) { return true; }
        let mut adapter = Adapter { dark, restyle_variables: true, exact_font_style: context.exact_font_style, image_index: 0, nodes_left: context.nodes, text_left: context.text,
            truncated: false, unsupported: false, images: self.images.clone(), mono_family: Theme::global(cx).mono_font_family.clone() };
        let blocks = adapter.blocks(&context.document.nodes, &Inherited::default(), true);
        if adapter.unsupported || !self.state.update(cx, |state, cx| state.restyle_blocks(blocks, cx)) {
            tracing::error!("富文本主题更新无法保留文档源状态");
            context.failed.store(true, Relaxed); return false;
        }
        context.dark.store(dark, Relaxed); true
    }
    pub fn render(&self, window: &Window, playing: bool, cx: &mut App) -> Div {
        if !self.refresh_variables(window, cx) { return div().child("无法更新富文本样式"); }
        self.images.set_playing(playing);
        let images = self.images.clone();
        div().size_full().min_h_0().overflow_hidden().bg(rgb(0xffffff)).text_color(rgb(0x334155)).text_size(px(BODY_FONT_SIZE))
            .line_height(relative(LINE_HEIGHT)).when(self.paper, |body| body.tab_size(8))
            .child(TextView::new(&self.state).selectable(false).scrollable(false).style(style(window, BODY_FONT_SIZE)).markdown_extensions(table::extensions().clone())
                .when(self.paper, |view| view.require_native_layout("此富文本包含暂不支持的排版组合"))
                .image_source(move |uri| images.resolve(uri)))
    }
    pub fn render_details(&self, window: &Window, cx: &mut App) -> Div {
        if !self.refresh_variables(window, cx) { return div().child("无法更新富文本样式"); }
        self.images.set_playing(true);
        let images = self.images.clone();
        div().min_w_0()
            .when(self.paper, |body| body.bg(rgb(0xffffff)).text_color(rgb(0x334155))).text_size(px(BODY_FONT_SIZE)).line_height(relative(LINE_HEIGHT))
            .when(self.paper, |body| body.tab_size(8))
            .child(TextView::new(&self.state).selectable(true).scrollable(false).style(if self.paper { style(window, BODY_FONT_SIZE) } else { TextViewStyle::default() }).markdown_extensions(table::extensions().clone())
                .when(self.paper, |view| view.require_native_layout("此富文本包含暂不支持的排版组合"))
                .image_source(move |uri| images.resolve(uri)))
    }
}
fn heading_scale(level: u8) -> f32 { match level { 1..=3 => 1.15, 4 => 1.0, 5 => 0.83, _ => 0.67 } }

fn inline_kind(kind: &Kind) -> bool {
    matches!(kind, Kind::Span | Kind::Bold | Kind::Italic | Kind::Underline | Kind::Strike | Kind::Mark
        | Kind::Small | Kind::Subscript | Kind::Superscript | Kind::Code | Kind::Link | Kind::Image { .. } | Kind::Break)
}

fn has_block_descendant(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Text(_) => false,
        Node::Element(element) => !inline_kind(&element.kind) || has_block_descendant(&element.children),
    })
}

fn inline_baseline_shift(vertical: &VerticalAlignment, parent: &Inherited) -> f32 {
    let own = match vertical {
        VerticalAlignment::Superscript => 0.35,
        VerticalAlignment::Subscript => -0.2,
        _ => 0.,
    };
    parent.mark.baseline_shift.unwrap_or(0.) + parent.mark.font_size_scale.unwrap_or(1.) * own
}
fn style(window: &Window, font_size: f32) -> TextViewStyle {
    let mut quote = div().border_color(rgb(0xcbd5e1)).pl(px(10.0)).text_color(rgb(0x64748b));
    quote.style().border_widths.left = Some(px(3.0).into());
    TextViewStyle::default().with_authored_flow(true).with_foreground(rgb(0x334155).into()).with_muted_foreground(rgb(0x64748b).into())
        .with_border(rgb(0xcbd5e1).into()).with_paragraph_gap(rems(font_size * 0.4 / f32::from(window.rem_size()).max(1.0)))
        .with_blockquote_style(quote.style().clone()).with_heading(move |level| {
            let scale = heading_scale(level);
            let mut heading = div().text_size(px(font_size * scale)).line_height(relative(LINE_HEIGHT))
                .font_weight(FontWeight::BOLD);
            heading.style().clone()
        })
}
#[derive(Clone)]
enum LengthBasis {
    Pixels(f64),
    Metric(Arc<DecorationMetric>),
}
#[derive(Clone)]
enum ComputedDecoration {
    Basis(LengthBasis),
    Calculation(Arc<ComputedLength>),
}

/// Explicit inheritance reuses these computing bases, while percentages remain
/// relative to the receiving property's basis. No second layout cache.
struct ComputedLength {
    expression: Arc<LengthPercentage>, bases: Vec<(LengthUnit, LengthBasis)>, viewport_dependent: bool,
}
impl ComputedLength {
    fn resolve(&self, font: &Font, window: &Window, percentage_basis: f64) -> f64 {
        let mut body_font = Some(font.clone());
        let mut resolved = [None; LengthUnit::COUNT];
        self.expression.evaluate(|unit, value| {
            value * *resolved[unit as usize].get_or_insert_with(|| {
                let basis = &self.bases.iter().find(|(candidate, _)| *candidate == unit).expect("validated unit has a computing basis").1;
                match basis {
                    LengthBasis::Pixels(pixels) => *pixels,
                    LengthBasis::Metric(metric) => metric.length(&mut body_font, window),
                }
            })
        }, percentage_basis)
    }

    fn native(self: &Arc<Self>, line_height: f32) -> InlineBaselineOffset {
        let source = self.clone();
        InlineBaselineOffset::new(self.viewport_dependent, move |font, window| {
            let result = source.resolve(font, window, f64::from(line_height));
            const MAX_CSS_LENGTH: f64 = i32::MAX as f64 / 64.;
            px(if result.is_nan() { 0. } else { result.clamp(-MAX_CSS_LENGTH, MAX_CSS_LENGTH) as f32 })
        })
    }
}

#[derive(Clone)]
struct Inherited {
    mark: TextMark, isolation: Option<TextDirection>, font_size: f32, native_body_size: f32, space: WhiteSpace, alignment: Option<Alignment>, rtl: bool, plaintext: bool, list_depth: usize,
    authored_foreground: Hsla, painted_background: Rgba,
    // Returning to LTR inside a changed-direction subtree still requires a
    // complete paragraph context; it must not bypass the capability check.
    // Own computed values differ from the effects painted through descendants.
    background: Option<CssColor>, decoration: Decoration, decoration_length: Option<ComputedDecoration>, vertical: VerticalAlignment, vertical_offset: Option<Arc<ComputedLength>>, list_style: ListStyle,
}
impl Default for Inherited {
    fn default() -> Self {
        let mut mark = TextMark { foreground: Some(rgb(0x334155).into()), space_policy: Some(TextSpacePolicy::Collapse), ..TextMark::default() };
        mark.inline_box = Some(InlineBox::root(&mark, px(BODY_FONT_SIZE), px(BODY_FONT_SIZE * LINE_HEIGHT)));
        Self { mark, isolation: None, font_size: BODY_FONT_SIZE, native_body_size: BODY_FONT_SIZE,
            authored_foreground: rgb(0x334155).into(), painted_background: rgb(0xffffff),
            space: WhiteSpace::Normal, alignment: None, rtl: false, plaintext: false, list_depth: 0,
            background: None, decoration: Decoration::default(), decoration_length: None, vertical: VerticalAlignment::Baseline, vertical_offset: None, list_style: ListStyle::Disc }
    }
}
impl Inherited {
    fn background_color(&self) -> Option<Hsla> {
        self.background.as_ref().map(|color| color_value(color, self.authored_foreground))
    }

    fn apply_inline_box(&mut self, parent: &Inherited) {
        let alignment = match &self.vertical {
            VerticalAlignment::Middle => InlineVerticalAlign::Middle,
            VerticalAlignment::TextTop => InlineVerticalAlign::TextTop,
            VerticalAlignment::TextBottom => InlineVerticalAlign::TextBottom,
            VerticalAlignment::Top => InlineVerticalAlign::Top,
            VerticalAlignment::Bottom => InlineVerticalAlign::Bottom,
            _ => InlineVerticalAlign::Baseline,
        };
        // An otherwise anonymous paragraph needs a parent font only when a
        // child explicitly aligns against its metrics. Normal text keeps the
        // existing no-extra-parent path. This font scope is not a paragraph
        // root: sibling alignments must not create multiple paragraph struts.
        let parent_box = self.mark.inline_box.clone().or_else(|| {
            matches!(alignment, InlineVerticalAlign::Middle | InlineVerticalAlign::TextTop | InlineVerticalAlign::TextBottom).then(|| {
                InlineBox::new(None, None, &parent.mark, px(parent.font_size),
                    px(parent.native_body_size), px(parent.font_size * LINE_HEIGHT))
            })
        });
        self.mark.inline_box = Some(InlineBox::aligned_with_offset(
            parent_box, self.background_color(), &self.mark,
            px(self.font_size), px(self.native_body_size), px(self.font_size * LINE_HEIGHT), alignment, self.isolation,
            self.vertical_offset.as_ref().map(|offset| offset.native(self.font_size * LINE_HEIGHT)),
        ));
    }

    fn compute_length(&self, expression: Arc<LengthPercentage>) -> Arc<ComputedLength> {
        let mut bases = Vec::new();
        for unit in expression.units() {
            if bases.iter().any(|(candidate, _)| *candidate == unit) { continue; }
            let basis = self.compute_length_basis(1., unit).expect("all controlled length units have a native basis");
            bases.push((unit, basis));
        }
        let viewport_dependent = bases.iter().any(|(_, basis)| matches!(basis,
            LengthBasis::Metric(metric) if matches!(metric.as_ref(), DecorationMetric::Viewport { .. })));
        Arc::new(ComputedLength { expression, bases, viewport_dependent })
    }

    fn apply_block_strut(&mut self, parent: &Inherited) {
        self.mark.inline_box = Some(InlineBox::root_reusing(parent.mark.inline_box.as_ref(),
            &self.mark, px(self.font_size), px(self.font_size * LINE_HEIGHT)));
    }

    fn font_weight(&self) -> f32 { self.mark.font_weight.unwrap_or(FontWeight::NORMAL).0 }
    fn font_style(&self) -> FontStyle { self.mark.resolved_font_style() }

    fn decoration_font(&self, metric: DecorationFontMetric) -> Arc<DecorationMetric> {
        Arc::new(DecorationMetric::Font(DecorationFont { family: self.mark.font_family.clone(), inherit_host_features: true, weight: FontWeight(self.font_weight()),
            style: self.font_style(), size: px(self.font_size), metric }))
    }

    fn decoration_origin(&self, baseline_shift: Pixels) -> DecorationOrigin {
        DecorationOrigin { family: self.mark.font_family.clone(), weight: FontWeight(self.font_weight()),
            style: self.font_style(),
            size: px(self.font_size), baseline_shift, inline_box: self.mark.inline_box.clone() }
    }

    fn rebase_decoration_origins(&mut self) {
        let Some(source) = self.mark.decoration_styles.as_ref() else { return; };
        let candidate = self.decoration_origin(Pixels::ZERO);
        let mut path = Vec::new();
        let mut next = Some(source);
        let mut changed = false;
        while let Some(layer) = next {
            if let Some(metrics) = layer.metrics.as_deref() {
                changed |= [&metrics.underline_origin, &metrics.overline_origin, &metrics.strikethrough_origin]
                    .into_iter().filter_map(|source| source.as_deref()).any(|source| *source != candidate);
            }
            path.push(layer);
            next = layer.parent.as_ref();
        }
        if !changed { return; }
        // Each propagated box acquires this block's anonymous inline origin.
        // Stroke colors/thicknesses and the independent layer order survive.
        let origin = Arc::new(candidate);
        let mut parent = None;
        for layer in path.into_iter().rev() {
            let mut styles = layer.as_ref().clone();
            styles.parent = parent;
            if let Some(existing) = styles.metrics.as_deref() {
                let mut metrics = existing.clone();
                if metrics.underline_origin.is_some() { metrics.underline_origin = Some(origin.clone()); }
                if metrics.overline_origin.is_some() { metrics.overline_origin = Some(origin.clone()); }
                if metrics.strikethrough_origin.is_some() { metrics.strikethrough_origin = Some(origin.clone()); }
                DecorationMetrics::set_shared(&mut styles.metrics, metrics);
            }
            parent = Some(Arc::new(styles));
        }
        self.mark.decoration_styles = parent;
    }

    fn compute_length_basis(&self, value: f32, unit: LengthUnit) -> Option<LengthBasis> {
        let factor = f64::from(value);
        let pixels = match unit {
            LengthUnit::Em => Some(factor * f64::from(self.font_size)),
            LengthUnit::Rem => Some(factor * f64::from(ROOT_FONT_SIZE)),
            LengthUnit::Lh => Some(factor * f64::from(self.font_size) * f64::from(LINE_HEIGHT)),
            _ => (DecorationThickness::Length { value, unit }).absolute_pixels(),
        };
        if let Some(pixels) = pixels { return Some(LengthBasis::Pixels(pixels)); }
        // The controlled preview has horizontal writing and no eligible size
        // query containers. Container units therefore use the small viewport
        // (CSS Containment 3); the native desktop host has no retractable UA
        // chrome, so its small/large/dynamic viewports share the client size.
        let axis = match unit {
            LengthUnit::Vw | LengthUnit::Vi | LengthUnit::Svw | LengthUnit::Svi
                | LengthUnit::Lvw | LengthUnit::Lvi | LengthUnit::Dvw | LengthUnit::Dvi
                | LengthUnit::Cqw | LengthUnit::Cqi => Some(DecorationViewportAxis::Width),
            LengthUnit::Vh | LengthUnit::Vb | LengthUnit::Svh | LengthUnit::Svb
                | LengthUnit::Lvh | LengthUnit::Lvb | LengthUnit::Dvh | LengthUnit::Dvb
                | LengthUnit::Cqh | LengthUnit::Cqb => Some(DecorationViewportAxis::Height),
            LengthUnit::Vmin | LengthUnit::Svmin | LengthUnit::Lvmin | LengthUnit::Dvmin
                | LengthUnit::Cqmin => Some(DecorationViewportAxis::Min),
            LengthUnit::Vmax | LengthUnit::Svmax | LengthUnit::Lvmax | LengthUnit::Dvmax
                | LengthUnit::Cqmax => Some(DecorationViewportAxis::Max),
            _ => None,
        };
        if let Some(axis) = axis {
            return Some(LengthBasis::Metric(Arc::new(DecorationMetric::Viewport { axis, percentage: value })));
        }
        let (metric, root) = match unit {
            LengthUnit::Ex => (DecorationFontMetric::XHeight(value), false),
            LengthUnit::Cap => (DecorationFontMetric::CapHeight(value), false),
            LengthUnit::Ch => (DecorationFontMetric::ZeroAdvance(value), false),
            LengthUnit::Ic => (DecorationFontMetric::IdeographicAdvance(value), false),
            LengthUnit::Rex => (DecorationFontMetric::XHeight(value), true),
            LengthUnit::Rcap => (DecorationFontMetric::CapHeight(value), true),
            LengthUnit::Rch => (DecorationFontMetric::ZeroAdvance(value), true),
            LengthUnit::Ric => (DecorationFontMetric::IdeographicAdvance(value), true),
            LengthUnit::Rlh => (DecorationFontMetric::NormalLineHeight(value), true),
            _ => return None,
        };
        Some(LengthBasis::Metric(if root {
            Arc::new(DecorationMetric::Font(DecorationFont { family: Some(ROOT_FONT_FAMILY.into()), inherit_host_features: false,
                weight: FontWeight::NORMAL, style: FontStyle::Normal, size: px(ROOT_FONT_SIZE), metric }))
        } else { self.decoration_font(metric) }))
    }

    fn text_direction(&self) -> TextDirection {
        if self.plaintext { TextDirection::Auto } else if self.rtl { TextDirection::Rtl } else { TextDirection::Ltr }
    }

    fn logical_alignment(&self) -> Option<LogicalAlignment> {
        match self.alignment.unwrap_or(Alignment::Start) {
            Alignment::Start | Alignment::Justify => Some(LogicalAlignment::Start),
            Alignment::End => Some(LogicalAlignment::End),
            _ => None,
        }
    }

    fn text_alignment(&self) -> Option<TextAlign> {
        match self.alignment.unwrap_or(Alignment::Start) {
            Alignment::Left => Some(TextAlign::Left),
            Alignment::Right => Some(TextAlign::Right),
            Alignment::Center => Some(TextAlign::Center),
            Alignment::Start => Some(if self.rtl { TextAlign::Right } else { TextAlign::Left }),
            Alignment::End => Some(if self.rtl { TextAlign::Left } else { TextAlign::Right }),
            Alignment::Justify => Some(if self.rtl { TextAlign::Right } else { TextAlign::Left }),
            Alignment::MatchParent => unreachable!("text alignment must be computed before layout"),
        }
    }
}
struct Adapter { dark: bool, restyle_variables: bool, exact_font_style: bool, image_index: usize, nodes_left: usize, text_left: usize, truncated: bool, unsupported: bool,
    images: images::Sources, mono_family: SharedString }
#[derive(Default)]
struct Runs { children: Vec<InlineNode>, line_has_text: bool, pending_space: bool, pending_mark: Option<TextMark>, pending_anchor: Option<usize>, alignment: Option<TextAlign>, logical_alignment: Option<LogicalAlignment>, neutral_direction: TextDirection, direction: TextDirection, justified: bool }
impl Runs {
    fn take_paragraph(&mut self) -> Paragraph {
        Paragraph::from_runs(std::mem::take(&mut self.children)).with_alignment(self.alignment)
            .with_logical_alignment(self.logical_alignment, self.neutral_direction)
            .with_direction(Some(self.direction)).with_justification(self.justified)
    }
}
impl Adapter {
    fn layout_node(&mut self) -> bool {
        if self.nodes_left == 0 || self.text_left == 0 { self.truncated = true; false }
        else { self.nodes_left -= 1; true }
    }

    fn blocks(&mut self, nodes: &[Node], inherited: &Inherited, root: bool) -> Vec<BlockNode> {
        let mut blocks = Vec::new(); let mut runs = Runs { alignment: inherited.text_alignment(), logical_alignment: inherited.logical_alignment(), neutral_direction: if inherited.rtl { TextDirection::Rtl } else { TextDirection::Ltr }, direction: inherited.text_direction(), justified: inherited.alignment == Some(Alignment::Justify), ..Default::default() };
        self.visit(nodes, inherited, &mut blocks, &mut runs, root);
        flush(&mut blocks, &mut runs); blocks
    }
    fn visit(&mut self, nodes: &[Node], inherited: &Inherited, blocks: &mut Vec<BlockNode>, runs: &mut Runs, root: bool) {
        // :first-child considers elements, not preceding anonymous text.
        let mut first_element = root;
        for node in nodes {
            if self.nodes_left == 0 || self.text_left == 0 { self.truncated = true; break; }
            match node {
                Node::Text(text) => self.text(text, inherited, runs),
                Node::Element(element) => {
                    let reset_top = std::mem::take(&mut first_element);
                    let fragmented_heading = matches!(element.kind, Kind::Heading(_))
                        && has_block_descendant(&element.children);
                    let cost = match element.kind {
                        Kind::Heading(_) => if fragmented_heading { 1 } else { 2 },
                        Kind::Quote | Kind::OrderedList { .. } | Kind::UnorderedList | Kind::ListItem | Kind::Rule | Kind::Table => 2,
                        Kind::Paragraph | Kind::Container | Kind::Pre | Kind::Caption | Kind::Image { .. } => 1,
                        _ => 0,
                    };
                    if self.nodes_left < cost { self.truncated = true; break; }
                    self.nodes_left -= cost;
                    let mut state = self.styled(element, inherited);
                    if fragmented_heading {
                        // A heading with block children has anonymous inline
                        // paragraphs between those blocks. Their host is the
                        // enclosing body's font, not Kit's inline-only heading
                        // leaf; retain the computed heading size in the marks.
                        state.native_body_size = inherited.native_body_size;
                        state.mark.font_size_scale = (state.font_size != state.native_body_size)
                            .then_some(state.font_size / state.native_body_size);
                    }
                    // vertical-align is not inherited and has no effect on a
                    // block container. Its table-cell use is handled by table().
                    let inline = inline_kind(&element.kind);
                    let background = if inline { None } else {
                        // A block paints its own fill. Ancestor block fills
                        // are already behind it; inline fills do not inherit
                        // into an intervening block formatting box.
                        state.mark.highlight = None;
                        state.mark.inline_box = None;
                        state.mark.baseline_shift = None;
                        state.apply_block_strut(inherited);
                        state.background_color().filter(|color| color.a > 0.0)
                    };
                    let flow = block_flow(&element.kind, &state, reset_top);
                    match &element.kind {
                        Kind::Break => { self.push("\n".into(), &state.mark, runs); runs.pending_space = false; runs.pending_mark = None; runs.pending_anchor = None; },
                        Kind::Paragraph | Kind::Container | Kind::Pre | Kind::Caption => {
                            flush(blocks, runs);
                            let children = self.blocks(&element.children, &state, false);
                            blocks.push(block_box(children, background, flow));
                        },
                        Kind::Heading(level) => {
                            if fragmented_heading {
                                flush(blocks, runs);
                                let children = self.blocks(&element.children, &state, false);
                                blocks.push(block_box(children, background, flow));
                                continue;
                            }
                            flush(blocks, runs); let mut children = Vec::new(); let mut heading = Runs { alignment: state.text_alignment(), logical_alignment: state.logical_alignment(), neutral_direction: if state.rtl { TextDirection::Rtl } else { TextDirection::Ltr }, direction: state.text_direction(), justified: state.alignment == Some(Alignment::Justify), ..Default::default() };
                            self.visit(&element.children, &state, &mut children, &mut heading, false);
                            debug_assert!(children.is_empty(), "inline-only heading produced a block");
                            let paragraph = heading.take_paragraph();
                            let heading = BlockNode::Heading { level: *level, children: paragraph, background: None, span: None };
                            blocks.push(block_box(vec![heading], background, flow));
                        },
                        Kind::Quote => {
                            flush(blocks, runs);
                            let quote = BlockNode::Blockquote { children: self.blocks(&element.children, &state, false), span: None };
                            blocks.push(block_box(vec![quote], background, flow));
                        },
                        Kind::UnorderedList | Kind::OrderedList { .. } => {
                            flush(blocks, runs);
                            let ordered = matches!(&element.kind, Kind::OrderedList { .. });
                            let first = if let Kind::OrderedList { start } = &element.kind { i64::from(*start) } else { 1 };
                            let mut child_state = state.clone(); child_state.list_depth += 1;
                            let children = self.blocks(&element.children, &child_state, false);
                            let count = children.iter().filter(|child| matches!(child, BlockNode::ListItem { .. })).count();
                            let markers: Vec<_> = element.children.iter().filter_map(|node| match node {
                                Node::Element(item) if matches!(item.kind, Kind::ListItem) => Some(item), _ => None,
                            }).take(count).enumerate().map(|(index, item)| {
                                let mut item_state = self.styled(item, &state);
                                // Outside markers do not paint the list item's
                                // background; the content box owns that fill.
                                item_state.mark.highlight = None;
                                let text = lists::label(&item_state.list_style, first.saturating_add(index as i64), item_state.rtl);
                                ListMarker { text, direction: Some(item_state.text_direction()), mark: item_state.mark,
                                    indent_em: 1.5 * state.mark.font_size_scale.unwrap_or(1.0) }
                            }).collect();
                            // The preview authors physical padding-left. In RTL
                            // it coexists with the UA's 40px inline-start inset;
                            // an individual LI's direction cannot change it.
                            let padding = Edges { top: px(0.0), bottom: px(0.0), left: px(1.5 * state.font_size),
                                right: px(if state.rtl { 40.0 } else { 0.0 }) };
                            let list = BlockNode::List { children, ordered, start: u32::try_from(first).ok(), markers: Some(markers.into()), padding: Some(padding), span: None };
                            blocks.push(block_box(vec![list], background, flow));
                        },
                        Kind::ListItem => {
                            flush(blocks, runs);
                            let children = vec![block_box(self.blocks(&element.children, &state, false), background, flow)];
                            blocks.push(BlockNode::ListItem { children, spread: true, checked: None, span: None });
                        },
                        Kind::Rule => {
                            let appearance = rule::appearance(state.mark.foreground.unwrap_or_else(|| rgb(0x808080).into()));
                            flush(blocks, runs); blocks.push(block_box(vec![BlockNode::HorizontalRule { appearance: Some(appearance), span: None }], background, flow));
                        },
                        Kind::Table => {
                            flush(blocks, runs);
                            let children = self.table(&element.children, &state);
                            // The grid owns the table fill; captions are outside it.
                            blocks.push(block_box(children, None, flow));
                        },
                        Kind::Image { data, alt } => {
                            self.flush_pending_space(&state.mark, runs);
                            if self.truncated { break; }
                            if let Some(source) = data {
                                let natural = source.dimensions().filter(|(width, height)| *width > 0 && *height > 0 && *width <= 65_536 && *height <= 65_536 && u64::from(*width) * u64::from(*height) <= 40_000_000)
                                    .map(|(width, height)| size(px(width as f32), px(height as f32)));
                                let key = self.images.insert(self.image_index, source.clone(), natural.is_none());
                                self.image_index += 1;
                                let Some(key) = key else { self.unsupported = true; continue; };
                                let Some(natural) = natural else {
                                    state.apply_inline_box(inherited);
                                    self.push(format!("[{}]", if alt.is_empty() { "图片无法预览" } else { alt.as_ref() }).into(), &state.mark, runs);
                                    continue;
                                };
                                let mut image = ImageNode::from_source(SharedUri::from(key.clone()), Some(natural));
                                let dimensions = self.images.clone(); let size_key = key.clone();
                                image.natural_size_resolver = Some(Arc::new(move || dimensions.natural_size(&size_key)));
                                image.baseline_shift = state.mark.baseline_shift.unwrap_or(0.0);
                                image.inline_box = if let Some(direction) = state.isolation {
                                    let parent = state.mark.inline_box.clone().unwrap_or_else(|| InlineBox::root(
                                        &inherited.mark, px(inherited.font_size), px(inherited.font_size * LINE_HEIGHT)));
                                    Some(InlineBox::isolated(parent, direction))
                                } else { state.mark.inline_box.clone() };
                                image.background = state.background_color().filter(|color| color.a > 0.);
                                image.vertical_style = match &state.vertical {
                                    VerticalAlignment::Offset(_) => Some(ImageVerticalAlign::Baseline),
                                    VerticalAlignment::Top => Some(ImageVerticalAlign::Top),
                                    VerticalAlignment::Middle => Some(ImageVerticalAlign::Middle),
                                    VerticalAlignment::Bottom => Some(ImageVerticalAlign::Bottom),
                                    VerticalAlignment::TextTop => Some(ImageVerticalAlign::TextTop),
                                    VerticalAlignment::TextBottom => Some(ImageVerticalAlign::TextBottom),
                                    _ => None,
                                }.map(|align| ImageVerticalStyle {
                                    align,
                                    baseline_offset: state.vertical_offset.as_ref().map(|offset| offset.native(state.font_size * LINE_HEIGHT)),
                                    parent_font_family: inherited.mark.font_family.clone(),
                                    parent_font_weight: inherited.mark.font_weight,
                                    parent_font_style: inherited.font_style(),
                                    parent_font_scale: inherited.mark.font_size_scale.unwrap_or(1.0),
                                });
                                image.no_wrap = state.mark.no_wrap;
                                image.alt = Some(SharedString::from(alt.clone())); image.title = element.title.clone().map(SharedString::from);
                                let images = self.images.clone();
                                image.on_visibility = Some(Arc::new(move |visible, window, cx| { images.visible(&key, visible, window, cx); }));
                                runs.children.push(InlineNode::image(image)); runs.line_has_text = true;
                            } else {
                                state.apply_inline_box(inherited);
                                self.push(format!("[{}]", if alt.is_empty() { "图片无法预览" } else { alt.as_ref() }).into(), &state.mark, runs);
                            }
                        },
                        Kind::TableHead | Kind::TableBody | Kind::TableFoot | Kind::TableRow | Kind::Cell { .. } => { self.unsupported = true; },
                        _ => {
                            let before = (runs.children.len(), blocks.len());
                            self.visit(&element.children, &state, blocks, runs, false);
                            if inline && !self.truncated && before == (runs.children.len(), blocks.len())
                                && let Some(source) = &state.mark.inline_box
                            {
                                let cost = if runs.children.is_empty() { 2 } else { 1 };
                                if self.nodes_left < cost { self.truncated = true; break; }
                                self.nodes_left -= cost;
                                if runs.pending_space { runs.pending_anchor.get_or_insert(runs.children.len()); }
                                runs.children.push(InlineNode::empty_inline_box(source.clone()));
                            }
                        },
                    }
                },
            }
        }
    }
    fn direction(element: &Element, state: &mut Inherited) {
        let Some(direction) = element.direction else { return; };
        state.rtl = match direction {
            Direction::Auto => element.automatic_direction() == Direction::Rtl,
            Direction::Rtl => true,
            Direction::Ltr => false,
        };
        if direction == Direction::Auto && matches!(element.kind, Kind::Pre) { state.plaintext = true; }
        if inline_kind(&element.kind) {
            // HTML resolves dir=auto from the whole element once. Its CSS
            // isolate keeps that direction across hard paragraphs; it is not
            // a fresh Unicode FSI decision on every shaped line.
            state.isolation = Some(if state.rtl { TextDirection::Rtl } else { TextDirection::Ltr });
        }
    }
    fn styled(&mut self, element: &Element, parent: &Inherited) -> Inherited {
        let mut state = parent.clone();
        // Readability corrections belong to painting, never the CSS cascade.
        // Descendants and currentcolor still compute from the copied source.
        state.mark.foreground = Some(parent.authored_foreground);
        state.isolation = None;
        // Plaintext affects anonymous lines of its own block, not descendant
        // blocks. Inline descendants remain part of that same paragraph input.
        state.plaintext = inline_kind(&element.kind) && parent.plaintext;
        state.background = None; state.decoration = Decoration::default(); state.vertical = VerticalAlignment::Baseline; state.vertical_offset = None;
        // HTML defaults form the lower origin. The preview's own CSS is applied
        // separately so revert and inline revert-layer have different results.
        match element.kind {
            Kind::Bold => state.mark.font_weight = Some(FontWeight(Weight::Bolder.resolve(parent.font_weight()))),
            Kind::Heading(level) => { state.mark.font_weight = Some(FontWeight::BOLD); state.font_size *= heading_scale(level); state.native_body_size *= heading_scale(level); },
            Kind::Italic => state.mark.font_style = Some(FontStyle::Italic),
            Kind::Underline => state.decoration.underline = true,
            Kind::Strike => state.decoration.strike = true,
            Kind::Mark => { state.background = Some(CssColor::Rgba([255, 255, 0, 255])); state.mark.foreground = Some(rgb(0x000000).into()); },
            Kind::Rule => state.mark.foreground = Some(rgb(0x808080).into()),
            Kind::Small | Kind::Subscript | Kind::Superscript => {
                state.mark.font_size_scale = Some(state.mark.font_size_scale.unwrap_or(1.0) / 1.2);
                state.font_size /= 1.2;
            },
            Kind::Table => {
                // The preview's explicit 12px table font starts a new size
                // basis; an outer SMALL/SUB/SUP scale must not apply twice.
                state.font_size = TABLE_FONT_SIZE; state.mark.font_size_scale = None;
            },
            Kind::Code | Kind::Pre => state.mark.font_family = Some(self.mono_family.clone()),
            Kind::OrderedList { .. } => state.list_style = ListStyle::Decimal,
            Kind::UnorderedList => state.list_style = match parent.list_depth { 0 => ListStyle::Disc, 1 => ListStyle::Circle, _ => ListStyle::Square },
            Kind::Caption => {
                state.alignment = Some(Alignment::Center);
                // Captions are siblings of the native table block, so retain
                // the table's size against the enclosing text view's basis.
                state.mark.font_size_scale = (state.font_size != state.native_body_size).then_some(state.font_size / state.native_body_size);
            },
            Kind::TableHead | Kind::TableBody | Kind::TableFoot => state.vertical = VerticalAlignment::Middle,
            Kind::TableRow | Kind::Cell { .. } => state.vertical = parent.vertical.clone(),
            _ => {},
        }
        match element.kind {
            Kind::Pre => state.space = WhiteSpace::Pre,
            Kind::Subscript => state.vertical = VerticalAlignment::Subscript,
            Kind::Superscript => state.vertical = VerticalAlignment::Superscript,
            Kind::Cell { heading: true, .. } => { state.mark.font_weight = Some(FontWeight::BOLD); state.alignment = Some(Alignment::Center); },
            _ => {},
        }
        let style = element.style.resolve_variables(|name| variables::lookup(name, self.dark));
        let parent_color = parent.authored_foreground;
        let ua_color = state.mark.foreground.unwrap_or(parent_color);
        let default_color = if matches!(element.kind, Kind::Quote) { rgb(0x64748b).into() } else { ua_color };
        state.mark.foreground = Some(computed(style.foreground.as_ref().map(|color| color_value(color, parent_color)), style.resets.get(Property::Color),
            parent_color, rgb(0x000000).into(), ua_color, default_color));
        state.authored_foreground = state.mark.foreground.unwrap_or(parent_color);
        // currentcolor remains a computed keyword through explicit background
        // inheritance; only this element's painted box resolves its foreground.
        state.background = computed(style.background.clone().map(Some), style.resets.get(Property::Background),
            parent.background.clone(), None, state.background.clone(), state.background.clone());
        state.painted_background = state.background_color().map_or(parent.painted_background,
            |background| parent.painted_background.blend(background.into()));
        state.mark.foreground = Some(colors::readable(state.authored_foreground, state.painted_background));
        state.mark.font_weight = Some(FontWeight(computed(style.weight.map(|weight| weight.resolve(parent.font_weight())), style.resets.get(Property::Weight),
            parent.font_weight(), FontWeight::NORMAL.0, state.font_weight(), state.font_weight())));
        state.mark.font_style = Some(computed(style.font_style.map(|style| match style {
            RichFontStyle::Normal => FontStyle::Normal, RichFontStyle::Italic => FontStyle::Italic, RichFontStyle::Oblique => FontStyle::Oblique,
            RichFontStyle::ObliqueAngle(angle) => match FontStyle::with_oblique_angle(angle) {
                Ok(native) if cfg!(any(target_os = "linux", target_os = "windows", target_os = "macos")) || matches!(native, FontStyle::Normal | FontStyle::Oblique) => native,
                _ => {
                    // Backends without authored-angle support retain the
                    // existing explicit detail fallback.
                    self.unsupported |= self.exact_font_style;
                    FontStyle::Oblique
                },
            },
        }), style.resets.get(Property::FontStyle), parent.font_style(), FontStyle::Normal, state.font_style(), state.font_style()));
        // currentcolor stays symbolic through explicit inheritance. A newly
        // decorating descendant resolves it against its own foreground.
        state.decoration = computed(style.decoration.clone(), style.resets.get(Property::Decoration),
            parent.decoration.clone(), Decoration::default(), state.decoration.clone(), state.decoration.clone());
        // Lengths retain their computing element's basis, even when it draws
        // no line. Explicit inherit must not recompute them using a child's
        // font/line height. Percentages and from-font stay symbolic instead.
        state.decoration_length = if style.resets.get(Property::Decoration) == Some(StyleReset::Inherit) {
            parent.decoration_length.clone()
        } else {
            match &state.decoration.thickness {
                DecorationThickness::Length { value, unit } => state.compute_length_basis(*value, *unit).map(ComputedDecoration::Basis),
                DecorationThickness::Calculation(expression) => Some(ComputedDecoration::Calculation(state.compute_length(expression.clone()))),
                _ => None,
            }
        };
        // LI's UA match-parent resolves start/end against the parent, even
        // when the item subsequently establishes the opposite direction.
        // Explicit inherit/unset still inherit the unresolved CSS value.
        let parent_alignment = Some((parent.alignment.unwrap_or(Alignment::Start), parent.rtl));
        let ua_alignment = if matches!(element.kind, Kind::ListItem) {
            Alignment::MatchParent.resolve_match_parent(parent_alignment)
        } else { state.alignment.unwrap_or(Alignment::Start) };
        state.alignment = Some(computed(style.alignment, style.resets.get(Property::Alignment), parent.alignment.unwrap_or(Alignment::Start),
            Alignment::Start, ua_alignment, if matches!(element.kind, Kind::Table) { Alignment::Start } else { ua_alignment })
            .resolve_match_parent(parent_alignment));
        state.vertical = computed(style.vertical.clone(), style.resets.get(Property::Vertical), parent.vertical.clone(), VerticalAlignment::Baseline,
            state.vertical.clone(), if matches!(element.kind, Kind::Cell { .. }) { VerticalAlignment::Top } else { state.vertical.clone() });
        state.vertical_offset = if style.resets.get(Property::Vertical) == Some(StyleReset::Inherit)
            || (style.vertical.is_none() && style.resets.get(Property::Vertical).is_none() && matches!(element.kind, Kind::TableRow | Kind::Cell { .. })) {
            parent.vertical_offset.clone()
        } else if let VerticalAlignment::Offset(expression) = &state.vertical {
            Some(state.compute_length(expression.clone()))
        } else { None };
        state.space = computed(style.white_space, style.resets.get(Property::Space), parent.space, WhiteSpace::Normal,
            state.space, if matches!(element.kind, Kind::Code | Kind::Pre) { WhiteSpace::PreWrap } else { state.space });
        state.list_style = computed(style.list_style.clone(), style.resets.get(Property::List), parent.list_style.clone(), ListStyle::Disc,
            state.list_style.clone(), state.list_style.clone());
        // Decorations propagate independently of inheritance. A child's none
        // cancels its own lines, never those already drawn by an ancestor.
        state.mark.underline = parent.mark.underline || state.decoration.underline;
        state.mark.strikethrough = parent.mark.strikethrough || state.decoration.strike;
        Self::direction(element, &mut state);
        let inline = inline_kind(&element.kind);
        let origin_shift = if inline { inline_baseline_shift(&state.vertical, parent) } else { 0. };
        if inline {
            state.mark.baseline_shift = Some(origin_shift);
            // Decoration origins retain this exact scope, so propagated
            // ancestor strokes stay anchored when a descendant is aligned.
            if !matches!(element.kind, Kind::Image { .. }) { state.apply_inline_box(parent); }
        } else {
            state.mark.inline_box = None;
            state.rebase_decoration_origins();
        }
        let origin_shift = px(state.native_body_size * origin_shift);
        self.decorate(&mut state, origin_shift);
        state.mark.no_wrap = !state.space.allows_wrap();
        state.mark.space_policy = Some(match state.space {
            WhiteSpace::Normal | WhiteSpace::NoWrap | WhiteSpace::PreLine | WhiteSpace::PreLineNoWrap => TextSpacePolicy::Collapse,
            WhiteSpace::Pre => TextSpacePolicy::Preserve,
            WhiteSpace::PreWrap => TextSpacePolicy::PreWrap,
            WhiteSpace::BreakSpaces | WhiteSpace::BreakSpacesNoWrap => TextSpacePolicy::BreakSpaces,
        });
        state
    }
    fn decorate(&mut self, state: &mut Inherited, baseline_shift: Pixels) {
        let decoration = &state.decoration;
        if !decoration.underline && !decoration.overline && !decoration.strike { return; }
        let pattern = match decoration.pattern {
            DecorationPattern::Solid => TextDecorationPattern::Solid,
            DecorationPattern::Double => TextDecorationPattern::Double,
            DecorationPattern::Dotted => TextDecorationPattern::Dotted,
            DecorationPattern::Dashed => TextDecorationPattern::Dashed,
            DecorationPattern::Wavy => TextDecorationPattern::Wavy,
        };
        let thickness = match &decoration.thickness {
            DecorationThickness::Auto | DecorationThickness::FromFont => Some(f64::from(state.font_size) / 10.),
            DecorationThickness::Length { .. } | DecorationThickness::Calculation(_) => match &state.decoration_length {
                Some(ComputedDecoration::Basis(LengthBasis::Pixels(pixels))) => Some(*pixels),
                Some(ComputedDecoration::Basis(LengthBasis::Metric(_)) | ComputedDecoration::Calculation(_)) => Some(1.0),
                None => None,
            },
            DecorationThickness::Percentage(value) => {
                let pixels = f64::from(*value) * f64::from(state.font_size);
                pixels.is_finite().then_some(pixels)
            },
        };
        let Some(thickness) = thickness else { self.unsupported = true; return; };
        // CSS floors used thickness at one device pixel, including zero and
        // negative lengths. Keep a positive logical width so GPUI's ordinary
        // per-window stroke snapping applies that floor at the current scale.
        let thickness = used_decoration_thickness(thickness);
        let color = Some(colors::readable(color_value(&decoration.color, state.authored_foreground), state.painted_background));
        let parent = state.mark.decoration_styles.clone();
        let mut styles = DecorationStyles { parent: parent.clone(), ..Default::default() };
        let metric = if decoration.thickness == DecorationThickness::FromFont {
            Some(state.decoration_font(DecorationFontMetric::Underline))
        } else {
            match &state.decoration_length {
                Some(ComputedDecoration::Basis(LengthBasis::Metric(metric))) => Some(metric.clone()),
                Some(ComputedDecoration::Calculation(source)) => {
                    let source = source.clone();
                    let percentage_basis = f64::from(state.font_size);
                    Some(Arc::new(DecorationMetric::calculated(move |font, window| source.resolve(font, window, percentage_basis))))
                },
                _ => None,
            }
        };
        let inherited_metrics = parent.as_deref().and_then(|styles| styles.metrics.as_deref());
        let mut metrics = DecorationMetrics::default();
        let candidate = state.decoration_origin(baseline_shift);
        let origin = inherited_metrics.and_then(|metrics| {
            [&metrics.underline_origin, &metrics.overline_origin, &metrics.strikethrough_origin]
                .into_iter().filter_map(|source| source.as_ref()).find(|source| source.as_ref() == &candidate).cloned()
        }).unwrap_or_else(|| Arc::new(candidate));
        if decoration.underline { metrics.underline = metric.clone(); metrics.underline_origin = Some(origin.clone()); }
        if decoration.overline { metrics.overline = metric.clone(); metrics.overline_origin = Some(origin.clone()); }
        if decoration.strike { metrics.strikethrough = metric; metrics.strikethrough_origin = Some(origin); }
        DecorationMetrics::set_shared(&mut styles.metrics, metrics);
        if decoration.underline { styles.underline = Some(UnderlineStyle { thickness, color, wavy: false, skip_ink: true, pattern, ..Default::default() }); }
        if decoration.overline { styles.overline = Some(UnderlineStyle { thickness, color, wavy: false, skip_ink: true, pattern, ..Default::default() }); }
        if decoration.strike { styles.strikethrough = Some(StrikethroughStyle { thickness, color, pattern, ..Default::default() }); }
        state.mark.set_decoration_styles(styles);
    }
    fn text(&mut self, source: &Arc<str>, state: &Inherited, runs: &mut Runs) {
        let preserved = state.space.preserves_spaces();
        if preserved {
            self.flush_pending_space(&state.mark, runs);
            if self.truncated { return; }
            self.push(SharedString::from(source.clone()), &state.mark, runs); return;
        }
        if !runs.pending_space && !source.chars().any(|character| matches!(character, ' ' | '\t' | '\n' | '\r' | '\x0c')) {
            self.push(SharedString::from(source.clone()), &state.mark, runs); return;
        }
        // Charge the collapsed display text, not invisible HTML indentation.
        // Empty spans and leading spaces must not exhaust the visible prefix.
        let mut text = String::new(); let mut units = 0;
        for character in source.chars() {
            if units >= self.text_left { self.truncated = true; break; }
            if matches!(character, ' ' | '\t' | '\n' | '\r' | '\x0c') {
                if character == '\n' && state.space.preserves_breaks() {
                    text.push('\n'); units += 1;
                    runs.pending_space = false; runs.pending_mark = None; runs.pending_anchor = None;
                } else {
                    let space = if text.is_empty() { runs.line_has_text } else { !text.ends_with('\n') };
                    if space && !runs.pending_space { runs.pending_mark = Some(state.mark.clone()); }
                    runs.pending_space = space;
                    if !space { runs.pending_mark = None; runs.pending_anchor = None; }
                }
            } else {
                if runs.pending_space {
                    if text.is_empty() && (runs.pending_anchor.is_some()
                        || runs.pending_mark.as_ref().is_some_and(|mark| mark != &state.mark)) {
                        self.flush_pending_space(&state.mark, runs);
                        if self.truncated { break; }
                    } else {
                        text.push(' '); units += 1;
                        runs.pending_space = false; runs.pending_mark = None; runs.pending_anchor = None;
                    }
                }
                text.push(character); units += character.len_utf16();
            }
        }
        if !text.is_empty() { self.push(text.into(), &state.mark, runs); }
    }
    fn flush_pending_space(&mut self, fallback: &TextMark, runs: &mut Runs) {
        if !runs.pending_space { return; }
        let mark = runs.pending_mark.take().unwrap_or_else(|| fallback.clone());
        let anchor = runs.pending_anchor.take();
        runs.pending_space = false;
        let before = runs.children.len();
        self.push(" ".into(), &mark, runs);
        // Empty boxes may have arrived while whitespace was deferred. Keep
        // the eventual space before those DOM nodes without another buffer.
        if let Some(at) = anchor.filter(|_| runs.children.len() > before) {
            runs.children[at..].rotate_right(1);
        }
    }

    fn push(&mut self, text: SharedString, mark: &TextMark, runs: &mut Runs) {
        if text.is_empty() { return; }
        let clipped = super_paste_core::preview::excerpt(&text, self.text_left);
        let was_clipped = clipped.len() != text.len();
        if was_clipped { self.truncated = true; }
        if clipped.is_empty() { if was_clipped { self.text_left = 0; } return; }
        let cost = if runs.children.is_empty() { 2 } else { 1 };
        if self.nodes_left < cost { self.truncated = true; self.nodes_left = 0; return; }
        self.nodes_left -= cost;
        self.text_left = if was_clipped { 0 } else { self.text_left - clipped.encode_utf16().count() };
        // HTML parsing normalizes literal newlines already. A remaining CR
        // (e.g. a character reference) is CSS space, also in preformatted text.
        // Normalize only this visible prefix and keep its UTF-8/UTF-16 length.
        let text = if clipped.contains('\r') { SharedString::from(clipped.replace('\r', " ")) }
            else if clipped.len() == text.len() { text } else { SharedString::new(clipped) };
        push_run(text, mark, runs);
    }
}
fn flush(blocks: &mut Vec<BlockNode>, runs: &mut Runs) {
    if !runs.children.is_empty() {
        blocks.push(BlockNode::Paragraph(runs.take_paragraph()));
    }
    runs.line_has_text = false; runs.pending_space = false; runs.pending_mark = None; runs.pending_anchor = None;
}
fn push_run(text: SharedString, mark: &TextMark, runs: &mut Runs) {
    if text.is_empty() { return; }
    let length = text.len(); runs.line_has_text = !text.ends_with('\n'); runs.children.push(InlineNode::new(text).marks(vec![(0..length, mark.clone())]));
}
fn block_flow(kind: &Kind, state: &Inherited, reset_top: bool) -> BlockFlow {
    let em = match kind {
        Kind::Paragraph | Kind::Quote | Kind::UnorderedList | Kind::OrderedList { .. } => 0.4,
        Kind::Heading(1..=3) | Kind::Rule => 0.5,
        Kind::Heading(4) => 1.33,
        Kind::Heading(5) => 1.67,
        Kind::Heading(_) => 2.33,
        Kind::Pre => 1.0,
        _ => 0.0,
    };
    let vertical = px(state.font_size * em);
    let horizontal = if matches!(kind, Kind::Quote) { px(40.0) } else { px(0.0) };
    BlockFlow {
        margin: Edges { top: if reset_top { px(0.0) } else { vertical }, bottom: vertical, left: horizontal, right: horizontal },
        independent: matches!(kind, Kind::Caption | Kind::Table),
    }
}

fn block_box(children: Vec<BlockNode>, background: Option<Hsla>, flow: BlockFlow) -> BlockNode {
    // Empty blocks still carry a collapsible margin strut. Keeping them also
    // prevents transparent containers from changing their children's flow.
    BlockNode::Root { children, background, flow: Some(flow), span: None }
}

fn computed<T>(specified: Option<T>, reset: Option<StyleReset>, parent: T, initial: T, ua: T, default: T) -> T {
    match reset {
        Some(StyleReset::Inherit) => parent, Some(StyleReset::Initial) => initial,
        Some(StyleReset::Revert) => ua, Some(StyleReset::RevertLayer) => default,
        None => specified.unwrap_or(default),
    }
}

fn color_value(color: &CssColor, current: Hsla) -> Hsla {
    match color {
        CssColor::Current => current,
        CssColor::Computed(value) => {
            let current: gpui::Rgba = current.into();
            let [r, g, b, a] = value.resolve([current.r, current.g, current.b, current.a]);
            gpui::Rgba { r, g, b, a }.into()
        },
        CssColor::Rgba([r, g, b, a]) => rgba((u32::from(*r) << 24) | (u32::from(*g) << 16) | (u32::from(*b) << 8) | u32::from(*a)).into(),
    }
}
