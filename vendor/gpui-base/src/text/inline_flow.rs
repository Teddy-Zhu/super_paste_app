use std::{
    cell::RefCell,
    ops::Range,
    rc::Rc,
    sync::{Arc, Mutex},
};
use unicode_segmentation::UnicodeSegmentation as _;

mod justification;
mod prepared;
mod contextual;
mod tabs;

use gpui::{
    AbsoluteLength, AnyElement, App, AvailableSpace, Bounds, DefiniteLength, Element, ElementId,
    GlobalElementId, Hsla, ImageSource, InspectorElementId, InteractiveElement as _, IntoElement,
    LayoutId, LineFragment as WrapLineFragment, ObjectFit, ParentElement as _, Pixels, Refineable as _, ShapedLine,
    SharedString, Size, StatefulInteractiveElement as _, StyleRefinement, Styled, StyledImage as _,
    TextAlign, TextDirection, TextRun, TextStyle, WhiteSpace, Window, img, point, prelude::FluentBuilder as _, px, relative,
    size,
};

use crate::text::text_view::{LinkClickHandlerFn, handle_link_click};
use crate::GlobalState;

use super::{
    inline::{Inline, InlineHighlight, InlineState, code_insets, text_runs, text_runs_for_window, text_size_ranges},
    inline_box::{BackgroundContext, InlineBox, InlineBackgrounds, InlineBoxMetrics, InlineBoxRow, InlineBoxGeometry},
    inline_object::{InlineObject, MeasuredInlineObject},
    node::{ImageVerticalAlign, ImageVerticalStyle, LinkMark, LogicalAlignment},
    range_highlight::RevealAt,
};

const ATOM_TEXT: &str = "\u{fffc}";
/// Flow layout, word selection and editor leaves share UTF-8 source offsets.
/// An image or custom object occupies one U+FFFC, never a synthetic byte.
pub(super) const ATOM_LEN: usize = ATOM_TEXT.len();
pub(super) const INLINE_CODE_PADDING: f32 = 4.;

pub(super) struct InlineFlow {
    id: ElementId,
    items: Vec<InlineFlowItem>,
    link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    native_layout: Option<super::native_layout::NativeLayoutGuard>,
    /// The block editor leaf this flow renders (design D3): its fragments
    /// and atoms are reported to the editor sink as pieces.
    editor_key: Option<u64>,
    justified: bool,
    logical_alignment: Option<(LogicalAlignment, TextDirection)>,
}

pub(super) type InlineRenderer = dyn Fn(&super::InlineRenderContext, &mut Window, &mut App) -> Option<super::InlineElement>
    + Send
    + Sync;

pub(super) enum InlineFlowItem {
    /// A non-replaced empty inline box: zero source/editor bytes, no painted glyph.
    // Absolute flow coordinates keep an empty box out of the shaping stream.
    Box { inline_box: Arc<InlineBox>, at: usize },
    Object {
        text: SharedString,
        id: usize,
        renderer: Arc<InlineRenderer>,
        accessibility_label: SharedString,
        selected: Arc<Mutex<Option<bool>>>,
        style: gpui::HighlightStyle,
        link: Option<LinkMark>,
        no_wrap: bool,
        inline_box: Option<Arc<InlineBox>>,
    },
    Text {
        state: Arc<Mutex<InlineState>>,
        text: SharedString,
        links: Vec<(Range<usize>, LinkMark)>,
        highlights: Vec<(Range<usize>, InlineHighlight)>,
        /// Range highlight backgrounds, in this item's byte space. They are
        /// only painted, so unlike `highlights` they take no part in layout.
        backgrounds: Vec<(Range<usize>, Hsla)>,
        /// The start of a pending reveal, when it is in this item.
        reveal: Option<RevealAt>,
    },
    Image {
        source: ImageSource,
        link: Option<LinkMark>,
        title: String,
        alt: SharedString,
        selected: Arc<Mutex<Option<bool>>>,
        width: Option<DefiniteLength>,
        height: Option<DefiniteLength>,
        natural_size: Option<Size<Pixels>>,
        baseline_shift: f32,
        vertical_style: Option<ImageVerticalStyle>,
        no_wrap: bool,
        on_visibility: Option<Arc<dyn Fn(bool, &mut Window, &mut App) + Send + Sync>>,
        inline_box: Option<Arc<InlineBox>>,
        background: Option<Hsla>,
    },
}

/// Source strings for one paint registration, shared by all of its fragments.
/// Object placeholders keep word boundaries without retaining image resources
/// or custom renderers. This snapshot owns no layout or word-boundary cache.
struct FlowWordSource {
    segments: smallvec::SmallVec<[SharedString; 1]>,
    geometry: Option<super::selection_adapter::WordSelectionGeometry>,
}

impl FlowWordSource {
    fn new(
        items: &[InlineFlowItem],
        geometry: Option<super::selection_adapter::WordSelectionGeometry>,
    ) -> Self {
        Self {
            segments: items.iter().map(|item| match item {
                InlineFlowItem::Text { text, .. } => text.clone(),
                InlineFlowItem::Box { .. } => SharedString::default(),
                InlineFlowItem::Image { .. } | InlineFlowItem::Object { .. } => {
                    SharedString::new_static(ATOM_TEXT)
                }
            }).collect(),
            geometry,
        }
    }
}

impl crate::text_selection::WordSelectionSource for FlowWordSource {
    fn segment_count(&self) -> usize { self.segments.len() }
    fn segment(&self, index: usize) -> Option<&str> {
        self.segments.get(index).map(|text| text.as_ref())
    }
    fn object_caret(&self, segment: usize, offset: usize) -> Option<Bounds<Pixels>> {
        self.geometry.as_ref()?.object_caret(self, segment, offset)
    }
}

pub(crate) struct InlineFlowLayoutState {
    typography: InlineFlowTypography,
    frame_state: Rc<RefCell<InlineFlowFrameState>>,
    /// The layout this element's last measurement returned, which is the
    /// one to paint. Per element, not per state: two flows under the same
    /// id (the same document shown twice) share the state's layouts but
    /// paint their own.
    current: Rc<RefCell<Option<Rc<InlineFlowLayout>>>>,
}

/// What a flow keeps from one frame to the next, as GPUI element state under
/// the flow's id.
#[derive(Default)]
struct InlineFlowFrameState {
    /// One state per text fragment, by text-fragment index. A fragment's `Inline`
    /// retains its shaped text under its state (see `RetainedLayout`), so
    /// the state has to be the same one next frame for the text to be
    /// found; a fresh state per frame shaped every fragment again and left
    /// an entry in the table that nobody would take.
    fragment_states: Vec<Arc<Mutex<InlineState>>>,
    /// What `layouts` were laid out from. A frame whose items or typography
    /// differ starts over.
    key: Option<FlowLayoutKey>,
    /// The layouts of `key`, one per wrap width the flow was measured at:
    /// taffy probes a flow at more than one width in a frame (unconstrained,
    /// then the column's). A frame that changes nothing finds every width
    /// here and wraps and shapes nothing.
    layouts: Vec<Rc<InlineFlowLayout>>,
    box_metrics: Option<Arc<InlineBoxMetrics>>,
}

/// How many wrap widths a flow keeps a layout for.
const LAYOUTS_PER_FLOW: usize = 3;

impl InlineFlowFrameState {
    fn fragment_state(&mut self, fragment_ix: usize) -> Arc<Mutex<InlineState>> {
        if self.fragment_states.len() <= fragment_ix {
            self.fragment_states
                .resize_with(fragment_ix + 1, Default::default);
        }
        self.fragment_states[fragment_ix].clone()
    }

    /// Makes `key` the current one, dropping the layouts of another key.
    fn set_key(&mut self, key: FlowLayoutKey, window: &Window) {
        if !self
            .key
            .as_ref()
            .is_some_and(|current| current.matches(&key))
        {
            self.layouts.clear();
            // Image geometry and fresh custom renderers invalidate placement,
            // but not immutable inline fonts, hierarchy or anchor positions.
            if !self.key.as_ref().is_some_and(|current| current.same_box_inputs(&key)) {
                self.box_metrics = measure_inline_boxes(&key.items, &key.typography.text_style, window);
            }
        }
        self.key = Some(key);
    }

    /// The layout at `wrap_width`, if the key was laid out there already.
    fn layout_at(&self, wrap_width: Option<Pixels>) -> Option<Rc<InlineFlowLayout>> {
        self.layouts
            .iter()
            .find(|layout| layout.wrap_width == wrap_width)
            .cloned()
    }

    /// Keeps `layout`, in place of the oldest one when the flow has been
    /// measured at more widths than it keeps.
    fn push_layout(&mut self, layout: Rc<InlineFlowLayout>) {
        if self.layouts.len() >= LAYOUTS_PER_FLOW {
            self.layouts.remove(0);
        }
        self.layouts.push(layout);
    }
}

/// Everything but the wrap width that a flow's layout depends on.
struct FlowLayoutKey {
    items: Vec<MeasureItem>,
    image_sizes: Vec<Option<Size<Pixels>>>,
    typography: InlineFlowTypography,
}

impl FlowLayoutKey {
    fn same_box_inputs(&self, other: &Self) -> bool {
        self.typography.font_generation == other.typography.font_generation
            && self.typography.viewport == other.typography.viewport
            && self.typography.text_style.font() == other.typography.text_style.font()
            && self.items.len() == other.items.len()
            && self.items.iter().zip(&other.items).all(|(item, other)| match (item, other) {
                (MeasureItem::Box { inline_box, at }, MeasureItem::Box { inline_box: other, at: other_at }) =>
                    inline_box == other && at == other_at,
                (MeasureItem::Text { highlights, .. }, MeasureItem::Text { highlights: other, .. }) =>
                    highlights.iter().filter_map(|(_, mark)| mark.inline_box.as_ref())
                        .eq(other.iter().filter_map(|(_, mark)| mark.inline_box.as_ref())),
                (MeasureItem::Image { inline_box, .. } | MeasureItem::Object { inline_box, .. },
                 MeasureItem::Image { inline_box: other, .. } | MeasureItem::Object { inline_box: other, .. }) =>
                    inline_box == other,
                _ => false,
            })
    }

    fn matches(&self, other: &Self) -> bool {
        self.items.len() == other.items.len()
            && self
                .items
                .iter()
                .zip(&other.items)
                .all(|(item, other)| item.same_layout_input(other))
            && self.image_sizes == other.image_sizes
            && self.typography == other.typography
    }
}

/// Resolved before `request_measured_layout`, while the parent's text-style
/// and rem stacks are still active.
#[derive(Clone, PartialEq)]
struct InlineFlowTypography {
    text_style: TextStyle,
    viewport: Option<Size<Pixels>>,
    font_generation: usize,
    require_native_layout: bool,
    rem_size: Pixels,
    line_height: Pixels,
    justified: bool,
    logical_alignment: Option<(LogicalAlignment, TextDirection)>,
}

#[derive(Default)]
struct InlineFlowLayout {
    native_failure: Option<super::native_layout::NativeLayoutFailure>,
    fragments: Vec<PositionedFragment>,
    empty_lines: Vec<(usize, Bounds<Pixels>)>,
    first_baseline: Option<Pixels>,
    size: Size<Pixels>,
    wrap_width: Option<Pixels>,
    box_metrics: Option<Arc<InlineBoxMetrics>>,
}

enum PositionedFragment {
    Object {
        item_ix: usize,
        origin: gpui::Point<Pixels>,
        baseline: Pixels,
        object: Box<MeasuredInlineObject>,
        selection_bounds: Bounds<Pixels>,
        inline_geometry: Option<Arc<InlineBoxGeometry>>,
        rtl: bool,
    },
    Text {
        item_ix: usize,
        origin: gpui::Point<Pixels>,
        baseline: Pixels,
        size: Size<Pixels>,
        source_range: Range<usize>,
        font_size: Pixels,
        text: gpui::ShapedText,
        links: Vec<(Range<usize>, LinkMark)>,
        highlights: Vec<(Range<usize>, InlineHighlight)>,
        selection_bounds: Bounds<Pixels>,
        /// The rounded box behind a code span, relative to the flow.
        code_background: Option<CodeBox>,
        spacing: Option<gpui::GlyphSpacing>,
        prepared_layout: Option<Arc<gpui::LineLayout>>,
    },
    Image {
        item_ix: usize,
        origin: gpui::Point<Pixels>,
        baseline: Pixels,
        size: Size<Pixels>,
        selection_bounds: Bounds<Pixels>,
        inline_geometry: Option<Arc<InlineBoxGeometry>>,
        rtl: bool,
    },
}

enum MeasureItem {
    Box { inline_box: Arc<InlineBox>, at: usize },
    Object {
        text: SharedString,
        id: usize,
        renderer: Arc<InlineRenderer>,
        style: gpui::HighlightStyle,
        no_wrap: bool,
        inline_box: Option<Arc<InlineBox>>,
    },
    Text {
        text: SharedString,
        links: Vec<(Range<usize>, LinkMark)>,
        highlights: Vec<(Range<usize>, InlineHighlight)>,
    },
    Image {
        source: ImageSource,
        width: Option<DefiniteLength>,
        height: Option<DefiniteLength>,
        natural_size: Option<Size<Pixels>>,
        baseline_shift: f32,
        vertical_style: Option<ImageVerticalStyle>,
        no_wrap: bool,
        inline_box: Option<Arc<InlineBox>>,
    },
}

struct LineFragmentLayout {
    item_ix: usize,
    kind: LineFragmentKind,
    size: Size<Pixels>,
    source_range: Range<usize>,
    baseline: Pixels,
    atomic_rtl: bool,
}

enum LineFragmentKind {
    Object(Box<MeasuredInlineObject>),
    Text {
        font_size: Pixels,
        text: gpui::ShapedText,
        links: Vec<(Range<usize>, LinkMark)>,
        highlights: Vec<(Range<usize>, InlineHighlight)>,
        /// The code span's box, relative to the fragment.
        code_background: Option<CodeBox>,
        spacing: Option<gpui::GlyphSpacing>,
        prepared_layout: Option<Arc<gpui::LineLayout>>,
    },
    Image,
}

/// The rounded box behind an inline code span, with its optional ring.
#[derive(Clone, Debug)]
struct CodeBox {
    bounds: Bounds<Pixels>,
    style: Arc<StyleRefinement>,
}

impl CodeBox {
    fn offset(self, by: gpui::Point<Pixels>) -> Self {
        Self {
            bounds: Bounds::new(by + self.bounds.origin, self.bounds.size),
            ..self
        }
    }
}

/// Atom decorations are overlays so they remain visible over opaque content.
pub(super) struct FlowPaintItem {
    element: AnyElement,
    background: Option<CodeBox>,
    overlays: Vec<(Bounds<Pixels>, Hsla)>,
}

#[derive(Default)]
pub(super) struct FlowPrepaint {
    elements: Vec<FlowPaintItem>,
    backgrounds: Option<Rc<RefCell<InlineBackgrounds>>>,
}

impl InlineFlow {
    pub(super) fn new(
        id: impl Into<ElementId>,
        items: Vec<InlineFlowItem>,
        link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    ) -> Self {
        Self {
            id: id.into(),
            items,
            link_click_handler,
            native_layout: None,
            editor_key: None,
            justified: false,
            logical_alignment: None,
        }
    }

    pub(super) fn native_layout(mut self, guard: Option<super::native_layout::NativeLayoutGuard>) -> Self {
        self.native_layout = guard;
        self
    }

    pub(super) fn editor_key(mut self, key: Option<u64>) -> Self {
        if let Some(key) = key {
            self.id = ("editor-leaf", key).into();
        }
        self.editor_key = key;
        self
    }

    pub(super) fn justified(mut self, justified: bool) -> Self {
        self.justified = justified;
        self
    }

    pub(super) fn logical_alignment(mut self, alignment: Option<(LogicalAlignment, TextDirection)>) -> Self {
        self.logical_alignment = alignment;
        self
    }

    fn image_element(
        ix: usize,
        source: &ImageSource,
        link: &Option<LinkMark>,
        _title: &str,
        alt: &SharedString,
        hosted: bool,
        size: Size<Pixels>,
        link_click_handler: Option<Arc<LinkClickHandlerFn>>,
        cx: &App,
    ) -> AnyElement {
        let alternative = if alt.is_empty() { SharedString::from("图片无法预览") } else { alt.clone() };
        let view = GlobalState::global(cx).text_view_state();
        img(source.clone())
            .id(("flow-image", ix))
            .object_fit(ObjectFit::Contain)
            .max_w(relative(1.))
            .w(size.width)
            .h(size.height)
            .when(hosted, |image| image.with_fallback(move || gpui::div().size_full().overflow_hidden().child(alternative.clone()).into_any_element()))
            .when_some(link.clone(), |this, link| {
                let aux_link = link.clone();
                let aux_link_click_handler = link_click_handler.clone();
                let aux_view = view.clone();
                this.cursor_pointer()
                    .on_click(move |event, window, cx| {
                        if view.as_ref()
                            .is_some_and(|view| view.read(cx).has_selection(cx))
                        {
                            return;
                        }
                        crate::TextSelection::end(window, cx);
                        cx.stop_propagation();
                        handle_link_click(
                            &link_click_handler, link.url.clone(), event.clone(), window, cx,
                        );
                    })
                    .on_aux_click(move |event, window, cx| {
                        if aux_view.as_ref()
                            .is_some_and(|view| view.read(cx).has_selection(cx))
                        {
                            return;
                        }
                        crate::TextSelection::end(window, cx);
                        cx.stop_propagation();
                        handle_link_click(
                            &aux_link_click_handler, aux_link.url.clone(), event.clone(), window, cx,
                        );
                    })
            })
            .into_any_element()
    }
}

impl IntoElement for InlineFlow {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for InlineFlow {
    type RequestLayoutState = InlineFlowLayoutState;
    type PrepaintState = FlowPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let frame_state = window.with_optional_element_state(id, |state, _| {
            let state: Rc<RefCell<InlineFlowFrameState>> = state.flatten().unwrap_or_default();
            (state.clone(), Some(state))
        });
        let measure_items = self.items.iter().map(MeasureItem::from).collect::<Vec<_>>();
        // A measured-layout callback runs after the parent's text-style stack
        // is gone. Capture every resolved typography input while this element
        // is still requested under its Markdown block (notably headings).
        let typography = InlineFlowTypography {
            text_style: window.text_style(),
            viewport: measure_items.iter().any(MeasureItem::viewport_dependent).then(|| window.viewport_size()),
            font_generation: window.text_system().font_generation(),
            require_native_layout: self.native_layout.is_some(),
            rem_size: window.rem_size(),
            line_height: window.line_height(),
            justified: self.justified,
            logical_alignment: self.logical_alignment,
        };
        let image_sizes = measure_items
            .iter()
            .enumerate()
            .map(|(ix, item)| match item {
                MeasureItem::Image {
                    source,
                    width,
                    height,
                    natural_size,
                    ..
                } => Some(measure_image_size(
                    ix,
                    source,
                    *width,
                    *height,
                    *natural_size,
                    typography.line_height,
                    typography.rem_size,
                    window,
                    cx,
                )),
                MeasureItem::Text { .. } | MeasureItem::Object { .. } | MeasureItem::Box { .. } => None,
            })
            .collect::<Vec<_>>();
        let objects = prepare_objects(&measure_items, &typography.text_style, window, cx);
        frame_state.borrow_mut().set_key(FlowLayoutKey {
            items: measure_items,
            image_sizes,
            typography: typography.clone(),
        }, window);
        let layout_state = InlineFlowLayoutState {
            typography: typography.clone(),
            frame_state: frame_state.clone(),
            current: Rc::default(),
        };
        let current = layout_state.current.clone();
        let native_layout = self.native_layout.clone();

        let layout_id = window.request_measured_layout_with_first_baseline(Default::default(), {
            move |known_dimensions, available_space, window, cx| {
                let intrinsic_min = known_dimensions.width.is_none()
                    && matches!(available_space.width, AvailableSpace::MinContent)
                    && typography.text_style.white_space == WhiteSpace::Normal;
                let wrap_width = if typography.text_style.white_space == WhiteSpace::Normal {
                    known_dimensions.width.or(match available_space.width {
                        AvailableSpace::Definite(width) => Some(width),
                        // This flow permits emergency wrapping at grapheme
                        // boundaries. Its minimum is the widest indivisible
                        // fragment, not the unwrapped hard line.
                        AvailableSpace::MinContent => Some(Pixels::ZERO),
                        _ => None,
                    })
                } else {
                    None
                };
                let mut state = frame_state.borrow_mut();
                if let Some(layout) = state.layout_at(wrap_width) {
                    if layout.native_failure.is_some() && let Some(guard) = &native_layout { guard.fail(cx); }
                    let measured = (layout.size, layout.first_baseline);
                    *current.borrow_mut() = Some(layout);
                    return measured;
                }
                let state = &mut *state;
                let key = state.key.as_ref().expect("set by request_layout");
                let layout = window.with_rem_size(Some(typography.rem_size), |window| {
                    window.with_text_style(
                        Some(typography.text_style.subtract(&Default::default())),
                        |window| {
                            layout_measured_flow_with_boxes(
                                &key.items,
                                &key.image_sizes,
                                &objects,
                                &typography.text_style,
                                wrap_width,
                                typography.justified,
                                typography.logical_alignment,
                                typography.require_native_layout,
                                state.box_metrics.clone(),
                                window,
                                cx,
                            )
                        },
                    )
                });
                if layout.native_failure.is_some() && let Some(guard) = &native_layout { guard.fail(cx); }
                let measured = (layout.size, layout.first_baseline);
                let layout = Rc::new(layout);
                *current.borrow_mut() = Some(layout.clone());
                // Intrinsic probes can produce one fragment per grapheme.
                // Taffy caches their measured size; keep the bounded display
                // cache for actual wrap widths rather than those probes.
                if !intrinsic_min { state.push_layout(layout); }
                measured
            }
        });

        (layout_id, layout_state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let frame_state = request_layout.frame_state.clone();
        let Some(layout) = request_layout.current.borrow().clone() else {
            return FlowPrepaint::default();
        };
        let typography = request_layout.typography.clone();
        let text_style = &typography.text_style;
        let inherited_style = window.text_style().subtract(&Default::default());
        if let Some(baseline) = layout.first_baseline {
            crate::GlobalState::global(cx).report_first_line(bounds.top() + baseline);
        }
        let mut elements = Vec::with_capacity(layout.fragments.len());
        let background_frame = layout.box_metrics.as_ref().filter(|metrics| metrics.has_backgrounds())
            .map(|metrics| Rc::new(RefCell::new(InlineBackgrounds::new(metrics.clone()))));
        let word_source: Option<Rc<dyn crate::text_selection::WordSelectionSource>> =
            self.items.iter().any(|item| matches!(item, InlineFlowItem::Text { .. })).then(|| {
                let geometry = crate::GlobalState::global(cx).text_view_state().map(|view| {
                    view.read(cx).selection_adapter.word_selection_geometry()
                });
                Rc::new(FlowWordSource::new(&self.items, geometry))
                    as Rc<dyn crate::text_selection::WordSelectionSource>
            });

        // A reveal goes to the fragment it starts in. A line break lays out
        // no fragment, so a reveal starting on one, or on an empty line, goes
        // to the next fragment of its text, or to the last when none follows.
        let reveal_fragment = layout
            .fragments
            .iter()
            .enumerate()
            .filter_map(|(ix, fragment)| {
                let PositionedFragment::Text {
                    item_ix,
                    source_range,
                    ..
                } = fragment
                else {
                    return None;
                };
                let InlineFlowItem::Text {
                    reveal: Some(reveal),
                    ..
                } = &self.items[*item_ix]
                else {
                    return None;
                };
                Some((ix, source_range.end > reveal.offset()))
            })
            .reduce(|found, next| if found.1 { found } else { next })
            .map(|(ix, _)| ix);

        // Editor leaf byte offset where each item starts: text items count
        // their bytes, images and custom nodes one U+FFFC.
        let editor = self.editor_key.zip(super::editor_seam::current());
        // Ordinary TextViews do not allocate editor bookkeeping.
        let item_starts: Vec<usize> = if let Some((key, sink)) = &editor {
            sink.begin_pieces(*key);
            let mut at = 0;
            self.items
                .iter()
                .map(|item| {
                    let start = at;
                    at += match item {
                        InlineFlowItem::Text { text, .. } => text.len(),
                        InlineFlowItem::Box { .. } => 0,
                        _ => ATOM_LEN,
                    };
                    start
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some((key, sink)) = &editor {
            for (start, line_bounds) in &layout.empty_lines {
                sink.report_empty_piece(
                    *key,
                    *start,
                    Bounds::new(bounds.origin + line_bounds.origin, line_bounds.size),
                );
            }
        }
        let decorations = editor.as_ref().map(|(key, sink)| sink.backgrounds(*key));
        let report_box = |item_ix: usize, bounds: Bounds<Pixels>| {
            let mut overlays = Vec::new();
            if let Some((key, sink)) = &editor {
                let start = item_starts[item_ix];
                let end = start + ATOM_LEN;
                sink.report_box_piece(*key, start..end, bounds);
                for (range, color) in decorations.as_deref().unwrap_or_default() {
                    if range.start < range.end && range.start < end && range.end > start {
                        overlays.push((bounds, *color));
                    }
                }
            }
            overlays
        };

        let mut text_fragment_count = 0;
        for (fragment_ix, fragment) in layout.fragments.iter().enumerate() {
            match fragment {
                PositionedFragment::Object {
                    item_ix,
                    origin,
                    baseline,
                    object,
                    selection_bounds,
                    inline_geometry,
                    rtl,
                } => {
                    let InlineFlowItem::Object {
                        id,
                        accessibility_label,
                        selected,
                        link,
                        inline_box,
                        ..
                    } = &self.items[*item_ix]
                    else {
                        continue;
                    };
                    let object_size = object.metrics.size;
                    if let (Some(frame), Some(source)) = (&background_frame, inline_box) {
                        frame.borrow_mut().include(source, bounds.left() + origin.x,
                            bounds.left() + origin.x + object_size.width,
                            bounds.top() + *baseline, inline_geometry.as_deref(), window);
                    }
                    let overlays =
                        report_box(*item_ix, Bounds::new(bounds.origin + *origin, object_size));
                    let mut element = InlineObject::new(
                        ("inline-object", *id),
                        accessibility_label.clone(),
                        (**object).clone(),
                        selected.clone(),
                        Bounds::new(
                            bounds.origin + selection_bounds.origin,
                            selection_bounds.size,
                        ),
                        Bounds::new(
                            point(bounds.left(), bounds.top() + selection_bounds.top()),
                            size(bounds.size.width, selection_bounds.size.height),
                        ),
                    )
                    .direction(*rtl)
                    .word_source(word_source.as_ref().map(|source| (source.clone(), *item_ix)))
                    .link(link.clone(), self.link_click_handler.clone())
                    .into_any_element();
                    element.prepaint_as_root(
                        bounds.origin + *origin,
                        size(
                            AvailableSpace::Definite(object_size.width),
                            AvailableSpace::Definite(object_size.height),
                        ),
                        window,
                        cx,
                    );
                    elements.push(FlowPaintItem {
                        element,
                        background: None,
                        overlays,
                    });
                }
                PositionedFragment::Text {
                    item_ix,
                    origin,
                    baseline,
                    size: fragment_size,
                    source_range,
                    font_size,
                    text,
                    links,
                    highlights,
                    selection_bounds,
                    code_background,
                    spacing,
                    prepared_layout,
                } => {
                    let InlineFlowItem::Text {
                        state: source_state,
                        backgrounds,
                        reveal,
                        ..
                    } = &self.items[*item_ix]
                    else {
                        continue;
                    };
                    let (origin, fragment_size, font_size) = (*origin, *fragment_size, *font_size);
                    let state = frame_state.borrow_mut().fragment_state(text_fragment_count);
                    text_fragment_count += 1;
                    if let Ok(mut state) = state.lock() {
                        state.set_text(text.clone());
                    }

                    let insets = code_insets(highlights, font_size, window.rem_size());
                    let background = code_background
                        .as_ref()
                        .map(|code| code.clone().offset(bounds.origin));
                    // The fragment's typography: the flow's style at the
                    // fragment's size, on one line. Fragments are already
                    // wrapped, so this leaf must not wrap independently.
                    let mut fragment_style = text_style.clone();
                    fragment_style.font_size = font_size.into();
                    fragment_style.line_height =
                        (fragment_size.height - insets.top - insets.bottom).into();
                    fragment_style.white_space = WhiteSpace::Nowrap;
                    if self.justified { fragment_style.text_align = TextAlign::Left; }
                    let words = word_source.as_ref().expect("text item has flow source").clone();
                    let element = Inline::new(
                        state,
                        links.clone(),
                        highlights.clone(),
                        self.link_click_handler.clone(),
                    )
                    .selection_source(
                        source_state.clone(), source_range.clone(), words, *item_ix,
                    )
                    .additional_spacing(spacing.clone())
                    .prepared_layout(prepared_layout.clone())
                    .native_layout(self.native_layout.clone())
                    .range_backgrounds(slice_ranges(
                        backgrounds,
                        source_range.start,
                        source_range.end,
                        |range, color| (range, *color),
                    ))
                    .reveal(
                        reveal
                            .as_ref()
                            .filter(|_| reveal_fragment == Some(fragment_ix))
                            .map(|reveal| reveal.clamp(source_range.start, source_range.end)),
                    )
                    .text_style(fragment_style.clone());
                    let element = element.editor_piece(
                        editor
                            .as_ref()
                            .map(|(key, _)| (*key, item_starts[*item_ix] + source_range.start)),
                    );
                    let mut element = element
                        .selection_bounds(Bounds::new(
                            point(bounds.left(), bounds.top() + selection_bounds.top()),
                            size(bounds.size.width, selection_bounds.size.height),
                        ))
                        .paint_origin(bounds.origin + origin + point(insets.left, insets.top))
                        .inline_backgrounds(background_frame.as_ref().filter(|_| highlights.iter().any(|(_, mark)| mark.inline_box.as_ref().is_some_and(|source| source.paints_background()))).map(|frame| BackgroundContext {
                            frame: frame.clone(), baseline: bounds.top() + *baseline,
                        }))
                        .into_any_element();
                    // The `Inline` is its own layout root, with no box around
                    // it: its text measures with the window's text style, so
                    // the fragment's style is pushed for the layout.
                    window.with_rem_size(Some(typography.rem_size), |window| {
                        window.with_text_style(
                            Some(fragment_style.subtract(&inherited_style)),
                            |window| {
                                element.prepaint_as_root(
                                    bounds.origin + origin + point(insets.left, insets.top),
                                    size(
                                        AvailableSpace::Definite(
                                            fragment_size.width - insets.left - insets.right,
                                        ),
                                        AvailableSpace::Definite(
                                            fragment_size.height - insets.top - insets.bottom,
                                        ),
                                    ),
                                    window,
                                    cx,
                                );
                            },
                        );
                    });
                    elements.push(FlowPaintItem {
                        element,
                        background,
                        overlays: Vec::new(),
                    });
                }
                PositionedFragment::Image {
                    item_ix,
                    origin,
                    baseline,
                    size: fragment_size,
                    selection_bounds,
                    inline_geometry,
                    rtl,
                } => {
                    let InlineFlowItem::Image {
                        source,
                        link,
                        title,
                        alt,
                        selected,
                        on_visibility,
                        inline_box,
                        background,
                        ..
                    } = &self.items[*item_ix]
                    else {
                        continue;
                    };
                    let (origin, fragment_size) = (*origin, *fragment_size);
                    if let (Some(frame), Some(source)) = (&background_frame, inline_box) {
                        frame.borrow_mut().include(source, bounds.left() + origin.x,
                            bounds.left() + origin.x + fragment_size.width,
                            bounds.top() + *baseline, inline_geometry.as_deref(), window);
                    }
                    let overlays =
                        report_box(*item_ix, Bounds::new(bounds.origin + origin, fragment_size));
                    let visible = if let Some(observer) = on_visibility {
                        let visible = Bounds::new(bounds.origin + origin, fragment_size).intersect(&window.content_mask().bounds);
                        let visible = visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO;
                        observer(visible, window, cx);
                        visible
                    } else {
                        true
                    };
                    // Keep atomic selection geometry for clipped images without
                    // creating an image element or retaining its render resource.
                    let content = if visible {
                        Self::image_element(
                            *item_ix,
                            source,
                            link,
                            title.as_str(),
                            alt,
                            on_visibility.is_some(),
                            fragment_size,
                            self.link_click_handler.clone(),
                            cx,
                        )
                    } else {
                        gpui::div().size_full().into_any_element()
                    };
                    let content = if let Some(color) = background {
                        gpui::div().size_full().bg(*color).child(content).into_any_element()
                    } else { content };
                    let mut element = InlineObject::image(
                        ("inline-image", *item_ix),
                        alt.clone(),
                        content,
                        fragment_size,
                        selected.clone(),
                        Bounds::new(
                            bounds.origin + selection_bounds.origin,
                            selection_bounds.size,
                        ),
                        Bounds::new(
                            point(bounds.left(), bounds.top() + selection_bounds.top()),
                            size(bounds.size.width, selection_bounds.size.height),
                        ),
                    )
                    .direction(*rtl)
                    .word_source(word_source.as_ref().map(|source| (source.clone(), *item_ix)))
                    .into_any_element();
                    element.prepaint_as_root(
                        bounds.origin + origin,
                        size(
                            AvailableSpace::Definite(fragment_size.width),
                            AvailableSpace::Definite(fragment_size.height),
                        ),
                        window,
                        cx,
                    );
                    elements.push(FlowPaintItem {
                        element,
                        background: None,
                        overlays,
                    });
                }
            }
        }

        frame_state
            .borrow_mut()
            .fragment_states
            .truncate(text_fragment_count);

        FlowPrepaint { elements, backgrounds: background_frame }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let preserve_selection = crate::GlobalState::global(cx)
            .text_view_state()
            .is_some_and(|view| {
                let state = view.read(cx);
                (state.preserve_inline_selection || state.selection_adapter.has_open_selection(cx))
                    && !state.is_all_selected(cx)
                    && !state.selection_adapter.excludes_current_block(cx)
            });
        for item in &self.items {
            if !preserve_selection
                && let InlineFlowItem::Text { state, .. } = item
                && let Ok(mut state) = state.lock()
            {
                state.selection = None;
            }
        }
        let radius = crate::Theme::global(cx).tokens.radius.sm;
        if let Some(backgrounds) = &prepaint.backgrounds { backgrounds.borrow_mut().paint(window); }
        for FlowPaintItem {
            element,
            background,
            overlays,
        } in &mut prepaint.elements
        {
            if let Some(code) = background {
                let mut style = gpui::Style::default();
                style.corner_radii = gpui::Corners {
                    top_left: radius.into(),
                    top_right: radius.into(),
                    bottom_left: radius.into(),
                    bottom_right: radius.into(),
                };
                style.refine(&code.style);
                style.paint(code.bounds, window, cx, |_, _| {});
            }
            element.paint(window, cx);
            for (bounds, color) in overlays {
                window.paint_quad(gpui::fill(*bounds, *color));
            }
        }
    }
}

impl From<&InlineFlowItem> for MeasureItem {
    fn from(item: &InlineFlowItem) -> Self {
        match item {
            InlineFlowItem::Box { inline_box, at } => Self::Box { inline_box: inline_box.clone(), at: *at },
            InlineFlowItem::Object {
                text,
                id,
                renderer,
                style,
                no_wrap,
                inline_box,
                ..
            } => Self::Object {
                text: text.clone(),
                id: *id,
                renderer: renderer.clone(),
                style: *style,
                no_wrap: *no_wrap,
                inline_box: inline_box.clone(),
            },
            InlineFlowItem::Text {
                state: _,
                text,
                links,
                highlights,
                ..
            } => MeasureItem::Text {
                text: text.clone(),
                links: links.clone(),
                highlights: highlights.clone(),
            },
            InlineFlowItem::Image {
                source,
                width,
                height,
                natural_size,
                baseline_shift,
                vertical_style,
                no_wrap,
                inline_box,
                ..
            } => MeasureItem::Image {
                source: source.clone(),
                width: *width,
                height: *height,
                natural_size: *natural_size,
                baseline_shift: *baseline_shift,
                vertical_style: vertical_style.clone(),
                no_wrap: *no_wrap,
                inline_box: inline_box.clone(),
            },
        }
    }
}

impl MeasureItem {
    fn viewport_dependent(&self) -> bool {
        match self {
            Self::Box { inline_box, .. } => inline_box.viewport_dependent(),
            Self::Text { highlights, .. } => highlights.iter().any(|(_, mark)| mark.inline_box.as_ref().is_some_and(|scope| scope.viewport_dependent())),
            Self::Image { inline_box, vertical_style, .. } => inline_box.as_ref().is_some_and(|scope| scope.viewport_dependent())
                || vertical_style.as_ref().and_then(|style| style.baseline_offset.as_ref()).is_some_and(|offset| offset.viewport_dependent()),
            Self::Object { inline_box, .. } => inline_box.as_ref().is_some_and(|scope| scope.viewport_dependent()),
        }
    }

    fn len(&self) -> usize {
        match self {
            MeasureItem::Box { .. } => 0,
            MeasureItem::Text { text, .. } => text.len(),
            MeasureItem::Image { .. } | MeasureItem::Object { .. } => ATOM_LEN,
        }
    }

    /// Whether `other` lays out the same as this item. An object is
    /// rendered anew every frame, so a flow with one is never reused.
    fn same_layout_input(&self, other: &Self) -> bool {
        match (self, other) {
            (MeasureItem::Box { inline_box, at }, MeasureItem::Box { inline_box: other, at: other_at }) => inline_box == other && at == other_at,
            (
                MeasureItem::Text {
                    text,
                    links,
                    highlights,
                },
                MeasureItem::Text {
                    text: other_text,
                    links: other_links,
                    highlights: other_highlights,
                },
            ) => text == other_text && links == other_links && highlights == other_highlights,
            (
                MeasureItem::Image { width, height, natural_size, baseline_shift, vertical_style, no_wrap, inline_box, .. },
                MeasureItem::Image {
                    width: other_width,
                    height: other_height,
                    natural_size: other_natural_size,
                    baseline_shift: other_baseline_shift,
                    vertical_style: other_vertical_style,
                    no_wrap: other_no_wrap,
                    inline_box: other_inline_box,
                    ..
                },
            ) => width == other_width && height == other_height && natural_size == other_natural_size && baseline_shift == other_baseline_shift && vertical_style == other_vertical_style && no_wrap == other_no_wrap && inline_box == other_inline_box,
            _ => false,
        }
    }
}

/// Intrinsic width for table sizing, using the same objects and wrapping inputs as painting.
pub(super) fn intrinsic_width(
    items: &[InlineFlowItem],
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    let items = items.iter().map(MeasureItem::from).collect::<Vec<_>>();
    let line_height = window.line_height();
    let rem_size = window.rem_size();
    let images = items
        .iter()
        .enumerate()
        .map(|(ix, item)| match item {
            MeasureItem::Image {
                source,
                width,
                height,
                natural_size,
                ..
            } => Some(measure_image_size(
                ix,
                source,
                *width,
                *height,
                *natural_size,
                line_height,
                rem_size,
                window,
                cx,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    layout_flow(&items, &images, &window.text_style(), None, window, cx)
        .size
        .width
}

fn prepare_objects(
    items: &[MeasureItem],
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) -> Vec<Option<MeasuredInlineObject>> {
    items
        .iter()
        .map(|item| match item {
            MeasureItem::Object {
                text,
                id,
                renderer,
                style,
                ..
            } => {
                let style = text_style.clone().highlight(*style);
                let context = super::InlineRenderContext::new(
                    style.clone(),
                    style.font_size.to_pixels(window.rem_size()),
                    style.line_height_in_pixels(window.rem_size()),
                    window.rem_size(),
                );
                // Must be the id `InlineObject` paints under: GPUI asserts a
                // stateful child (a plugin's HoverCard) sees the same id path
                // in request_layout as in prepaint, and the element is laid
                // out here but painted inside `InlineObject`.
                Some(window.with_id(("inline-object", *id), |window| {
                    let element = window
                        .with_text_style(Some(style.subtract(&Default::default())), |window| {
                            renderer(&context, window, cx)
                        });
                    MeasuredInlineObject::measure(text, element, &style, window, cx)
                }))
            }
            _ => None,
        })
        .collect()
}

fn layout_flow(
    items: &[MeasureItem],
    image_sizes: &[Option<Size<Pixels>>],
    text_style: &TextStyle,
    wrap_width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> InlineFlowLayout {
    let objects = prepare_objects(items, text_style, window, cx);
    layout_measured_flow(
        items,
        image_sizes,
        &objects,
        text_style,
        wrap_width,
        false,
        window,
        cx,
    )
}

fn image_alignment_baseline(
    style: &ImageVerticalStyle,
    height: Pixels,
    text_style: &TextStyle,
    body_font_size: Pixels,
    window: &Window,
) -> Pixels {
    if style.align == ImageVerticalAlign::Baseline {
        return height + style.baseline_offset.as_ref().map_or(Pixels::ZERO, |offset| offset.pixels(&text_style.font(), window));
    }
    if matches!(style.align, ImageVerticalAlign::Top | ImageVerticalAlign::Bottom) {
        return Pixels::ZERO; // Resolved against the final line box instead.
    }
    let mut font = text_style.font();
    if let Some(family) = &style.parent_font_family { font.family = family.clone(); }
    if let Some(weight) = style.parent_font_weight { font.weight = weight; }
    font.style = style.parent_font_style;
    let scale = if style.parent_font_scale.is_finite() && style.parent_font_scale > 0. { style.parent_font_scale } else { 1. };
    let font_size = body_font_size * scale;
    let system = window.text_system();
    let font_id = system.resolve_font(&font);
    match style.align {
        ImageVerticalAlign::Middle => height / 2. + system.x_height(font_id, font_size) / 2.,
        ImageVerticalAlign::TextTop => system.ascent(font_id, font_size),
        // FontMetrics keeps descent signed, unlike a shaped line's descent.
        ImageVerticalAlign::TextBottom => height + system.descent(font_id, font_size),
        ImageVerticalAlign::Baseline | ImageVerticalAlign::Top | ImageVerticalAlign::Bottom => Pixels::ZERO,
    }
}

#[cfg(test)]
thread_local! {
    /// How many flows were laid out (wrapped and shaped) on this thread.
    pub(super) static FLOW_LAYOUTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn measure_inline_boxes(items: &[MeasureItem], text_style: &TextStyle, window: &Window) -> Option<Arc<InlineBoxMetrics>> {
    // Inline boxes have no supported border/padding of their own. A flow
    // made solely of empty boxes is a phantom line, not an empty text row.
    if !items.iter().any(|item| item.len() > 0) { return None; }
    let mut metrics = InlineBoxMetrics::default();
    let font = text_style.font();
    for item in items {
        match item {
            MeasureItem::Box { inline_box, at } => metrics.register_anchor(*at, inline_box, &font, window),
            MeasureItem::Text { highlights, .. } => for (_, mark) in highlights {
                if let Some(source) = &mark.inline_box { metrics.register(source, &font, window); }
            },
            MeasureItem::Image { inline_box, .. } | MeasureItem::Object { inline_box, .. } => {
                if let Some(source) = inline_box { metrics.register(source, &font, window); }
            },
        }
    }
    metrics.seal();
    (!metrics.is_empty()).then(|| Arc::new(metrics))
}

fn layout_measured_flow(
    items: &[MeasureItem], image_sizes: &[Option<Size<Pixels>>],
    prepared_objects: &[Option<MeasuredInlineObject>], text_style: &TextStyle,
    wrap_width: Option<Pixels>, justified: bool, window: &mut Window, cx: &mut App,
) -> InlineFlowLayout {
    let box_metrics = measure_inline_boxes(items, text_style, window);
    layout_measured_flow_with_boxes(items, image_sizes, prepared_objects, text_style, wrap_width, justified, None, false, box_metrics, window, cx)
}

fn layout_measured_flow_with_boxes(
    items: &[MeasureItem],
    image_sizes: &[Option<Size<Pixels>>],
    prepared_objects: &[Option<MeasuredInlineObject>],
    text_style: &TextStyle,
    wrap_width: Option<Pixels>,
    justified: bool,
    logical_alignment: Option<(LogicalAlignment, TextDirection)>,
    require_native_layout: bool,
    box_metrics: Option<Arc<InlineBoxMetrics>>,
    window: &mut Window,
    _cx: &mut App,
) -> InlineFlowLayout {
    #[cfg(test)]
    FLOW_LAYOUTS.with(|layouts| layouts.set(layouts.get() + 1));
    let rem_size = window.rem_size();
    let line_height = plain_line_height(text_style, rem_size, window);
    let total_len = items.iter().map(MeasureItem::len).sum::<usize>();
    if total_len == 0 {
        return InlineFlowLayout::default();
    }

    // Host images keep their original CSS geometry even when their GPU raster
    // is downsampled. Constrain that geometry to the paragraph's content width.
    let fitted_images = image_sizes.iter().zip(items).map(|(image, item)| image.map(|original| {
        if matches!(item, MeasureItem::Image { natural_size: Some(_), .. })
            && let Some(width) = wrap_width.filter(|width| *width >= Pixels::ZERO && original.width > *width) {
            size(width, original.height * (width / original.width))
        } else { original }
    })).collect::<Vec<_>>();
    let image_sizes = &fitted_images;
    let objects = prepared_objects
        .iter()
        .map(|object| object.clone().map(|object| object.fit_text(wrap_width)))
        .collect::<Vec<_>>();
    let authored_spaces = items.iter().any(|item| matches!(item,
        MeasureItem::Text { highlights, .. } if highlights.iter().any(|(_, mark)| mark.space_policy.is_some())));
    let font_size = text_style.font_size.to_pixels(rem_size);
    let tab_rows_required = text_style.tab_size.is_some() && items.iter()
        .any(|item| matches!(item, MeasureItem::Text { text, .. } if text.contains('\t')));
    // Tab geometry shares the complete native paragraph's font and source
    // ownership. Unsupported legacy spans must take the explicit failure path.
    let native_layout_required = require_native_layout || tab_rows_required;
    let (line_ranges, source_spans) = match line_ranges(items, image_sizes, &objects, text_style, wrap_width, justified || authored_spaces,
        logical_alignment.map(|(_, fallback)| fallback), native_layout_required, window) {
        Ok(layout) => layout,
        Err(error) => return InlineFlowLayout {
            native_failure: Some(error), wrap_width, ..Default::default()
        },
    };
    let native_rows_required = native_layout_required && source_spans.iter()
        .any(|span| matches!(span, ShapedSpan::Text { contexts: Some(_), .. }));
    let mut prepared_rows = prepared::Rows::new(&source_spans, text_style.text_direction);
    let mut fragments = Vec::new();
    let mut empty_lines = Vec::new();
    let mut first_baseline = None;
    let mut max_width = Pixels::ZERO;
    let mut y = Pixels::ZERO;

    // The same body strut serves every row of this layout. Shaped-line descent
    // is positive; FontMetrics keeps it signed, so use the paint path's metrics
    // here instead of mixing conventions through baseline_offset().
    let body_runs = text_runs(1, text_style, &[]);
    let body_line = shape_line(" ".into(), font_size, &body_runs, text_style.text_direction, window);
    let body_baseline = centered_baseline(&body_line, line_height);
    // Body glyphs may overflow a smaller line height without enlarging it.
    let body_overflow_above = (body_line.ascent - body_baseline).max(Pixels::ZERO);
    let body_overflow_below = (body_line.descent - (line_height - body_baseline)).max(Pixels::ZERO);

    let (line_height, body_baseline) = box_metrics.as_ref().and_then(|metrics| metrics.root_strut())
        .map_or((line_height, body_baseline), |(above, below)| (above + below, above));
    let mut box_row = box_metrics.as_ref().map(|metrics| InlineBoxRow::new(metrics));
    let mut previous_row_end = None;
    // Rows consume source in order. Retain only the next child's source
    // position instead of scanning the consumed prefix again for every row.
    let mut next_item = 0;
    let mut next_item_start = 0;
    let mut lines = line_ranges.into_iter().peekable();
    while let Some(line) = lines.next() {
        let line_range = line.range;
        // Resolve L1/L2 for the complete soft row, including its source atoms,
        // before distributing fragments to the original source-order children.
        let parts = prepared_rows.take_contextual(line_range.clone(), native_layout_required, window);
        if (native_rows_required || tab_rows_required) && !line_range.is_empty() && parts.is_none() {
            return InlineFlowLayout {
                native_failure: Some(super::native_layout::NativeLayoutFailure::UnsafeClusterBoundary),
                wrap_width,
                ..Default::default()
            };
        }
        let mut native_parts = parts.map(|parts| parts.into_iter().peekable());
        let soft_wrap = lines.peek().is_some_and(|next| next.range.start == line_range.end);
        let mut justification = (authored_spaces || (justified && wrap_width.is_some()))
            .then(|| justification::Opportunities::new(justified && soft_wrap));
        let mut line_fragments = Vec::new();
        let mut visual_order = Vec::new();
        let mut line_width = Pixels::ZERO;
        if let (Some(row), Some(metrics)) = (&mut box_row, &box_metrics) {
            row.begin();
            row.anchors(&line_range, previous_row_end == Some(line_range.start), metrics);
        }
        let mut line_ascent = body_baseline;
        let mut line_descent = line_height - body_baseline;
        let mut top_image_height = Pixels::ZERO;
        let mut bottom_image_height = Pixels::ZERO;
        while let Some(item) = items.get(next_item) {
            let end = next_item_start + item.len();
            if !matches!(item, MeasureItem::Box { .. }) && end > line_range.start {
                break;
            }
            next_item_start = end;
            next_item += 1;
        }
        let mut item_start = next_item_start;
        let mut has_hard_break = false;
        let mut has_scoped_atoms = false;

        for (item_ix, item) in items.iter().enumerate().skip(next_item) {
            let item_end = item_start + item.len();
            if matches!(item, MeasureItem::Box { .. }) { continue; }
            // A hard break has no painted glyph fragment, but its inline box
            // still participates in the row it terminates, including an empty row.
            if let (MeasureItem::Text { text, highlights, .. }, Some(row), Some(metrics)) = (item, &mut box_row, &box_metrics)
                && item_start <= line_range.end && line_range.end < item_end
            {
                let at = line_range.end - item_start;
                if text.as_bytes()[at] == b'\n' {
                    has_hard_break = true;
                    for (range, mark) in highlights {
                        if range.contains(&at) && let Some(source) = &mark.inline_box { row.activate(source, metrics); }
                    }
                }
            }
            if item_end <= line_range.start {
                item_start = item_end;
                continue;
            }
            if item_start >= line_range.end {
                break;
            }

            let atom = if matches!(item, MeasureItem::Object { .. } | MeasureItem::Image { .. }) {
                native_parts.as_mut().map(|parts| {
                    let part = parts.next().expect("a native row must retain its source atom");
                    assert_eq!(part.range, item_start..item_end, "a native atom must retain its complete source");
                    assert!(part.line.is_none(), "native object font glyphs must never reach paint");
                    (part.visual_rank, part.atomic_rtl)
                })
            } else { None };
            let (atom_rank, atomic_rtl) = atom.unwrap_or((None, false));

            match item {
                MeasureItem::Box { .. } => {},
                MeasureItem::Text {
                    text,
                    links,
                    highlights,
                } => {
                    let local_start = line_range.start.max(item_start) - item_start;
                    let local_end = line_range.end.min(item_end) - item_start;
                    let native = native_parts.as_mut().map(|parts| std::iter::from_fn(move ||
                        parts.next_if(|part| part.range.start < item_end)));
                    let legacy = native.is_none().then(||
                        text_size_ranges(text.len(), highlights, font_size, window.rem_size()))
                        .into_iter().flatten().filter_map(|(segment, scale)| {
                            let start = local_start.max(segment.start);
                            let end = local_end.min(segment.end);
                            (start < end).then_some(prepared::Fragment {
                                range: item_start + start..item_start + end,
                                scale, line: None, visual_rank: None, atomic_rtl: false,
                            })
                        });
                    for part in native.into_iter().flatten().chain(legacy) {
                        assert!(part.range.start >= item_start && part.range.end <= item_end,
                            "native text fragments must retain their original source child");
                        let start = part.range.start - item_start;
                        let end = part.range.end - item_start;
                        let scale = part.scale;
                        if let Some(rank) = part.visual_rank { visual_order.push((line_fragments.len(), rank)); }
                        let mut highlights = slice_ranges(highlights, start, end, |range, style| (range, style.clone()));
                        let links =
                            slice_ranges(links, start, end, |range, link| (range, link.clone()));
                        let runs = text_runs_for_window(end - start, text_style, &highlights, window);
                        let segment_font_size = font_size * scale;
                        let scoped_len = highlights.iter().filter(|(_, mark)| mark.inline_box.is_some())
                            .map(|(range, _)| range.len()).sum::<usize>();
                        let (shaped_line, prepared_layout) = if let Some(shaped) = part.line {
                            let shaped = prepared::fit_unpainted_fragment(shaped, &runs, window);
                            let layout = Some(shaped.shared_layout());
                            (shaped, layout)
                        } else { match prepared_rows.take(
                            item_start + start..item_start + end, &runs, scoped_len >= end - start, window,
                        ) {
                            Some(row) if row.reusable => {
                                let layout = Some(row.line.shared_layout());
                                (row.line, layout)
                            }
                            row => {
                                let subtext = row.map_or_else(
                                    || shared_text_range(text, start..end),
                                    |row| row.line.text.to_shared_string(),
                                );
                                let shaped = shape_line(subtext, segment_font_size, &runs, line.shaping_direction, window);
                                let layout = Some(shaped.shared_layout());
                                (shaped, layout)
                            }
                        }};
                        let subtext = shaped_line.text.clone();
                        if let Some(justification) = &mut justification {
                            justification.text(line_fragments.len(), &shaped_line, &highlights);
                        }
                        let is_code = highlights.iter().any(|(_, h)| h.font_size_scale.is_some());
                        let insets = code_insets(&highlights, segment_font_size, window.rem_size());
                        let width = shaped_line.width() + insets.left + insets.right;
                        // Paint from a glyph-only local box. Legacy Markdown
                        // uses the body leading; structured CSS boxes contribute
                        // their own half-leading to the row below. Code fills
                        // remain independent of the expanded selection box.
                        let glyph_size = size(
                            width,
                            shaped_line.ascent + shaped_line.descent + insets.top + insets.bottom,
                        );
                        let baseline = shaped_line.ascent + insets.top;
                        let code_background = is_code
                            .then(|| {
                                code_background(
                                    &shaped_line,
                                    &runs,
                                    segment_font_size,
                                    &mut highlights,
                                    glyph_size,
                                    baseline,
                                    window,
                                )
                            })
                            .flatten();
                        // Keep the local glyph/code box unchanged. Moving its
                        // baseline changes its origin and the line's extents;
                        // text painting and hit testing still share one box.
                        let shift = highlights.iter().find_map(|(_, mark)| mark.authored_baseline_shift)
                            .filter(|shift| shift.is_finite()).unwrap_or(0.);
                        let shift = highlights.iter().find_map(|(_, mark)| {
                            mark.inline_box.as_ref().zip(box_metrics.as_ref())
                                .map(|(scope, geometry)| geometry.shift(scope))
                        }).unwrap_or(font_size * shift);
                        let baseline = baseline + shift;
                        if let (Some(row), Some(metrics)) = (&mut box_row, &box_metrics) {
                            row.text(&highlights, &shaped_line, segment_font_size, metrics, window);
                        }
                        if scoped_len < subtext.len() {
                            line_ascent = line_ascent.max(baseline - body_overflow_above);
                            line_descent = line_descent.max(glyph_size.height - baseline - body_overflow_below);
                        }
                        line_width += width;
                        line_fragments.push(LineFragmentLayout {
                            item_ix,
                            kind: LineFragmentKind::Text {
                                font_size: segment_font_size,
                                text: subtext,
                                links,
                                highlights,
                                code_background,
                                spacing: None,
                                prepared_layout,
                            },
                            size: glyph_size,
                            source_range: start..end,
                            baseline: baseline,
                            atomic_rtl: false,
                        });
                    }
                }
                MeasureItem::Object { inline_box, .. } => {
                    if let (Some(source), Some(row), Some(metrics)) = (inline_box, &mut box_row, &box_metrics) { row.activate(source, metrics); }
                    if let Some(justification) = &mut justification { justification.object(line_fragments.len()); }
                    let object = objects[item_ix].as_ref().unwrap().clone();
                    let metrics = object.metrics;
                    let shift = inline_box.as_ref().zip(box_metrics.as_ref())
                        .map_or(Pixels::ZERO, |(scope, geometry)| geometry.shift(scope));
                    let baseline = metrics.baseline + shift;
                    if inline_box.is_none() {
                        line_ascent = line_ascent.max(baseline);
                        line_descent = line_descent.max(metrics.size.height - baseline);
                    } else { has_scoped_atoms = true; }
                    line_width += metrics.size.width;
                    if let Some(rank) = atom_rank { visual_order.push((line_fragments.len(), rank)); }
                    line_fragments.push(LineFragmentLayout {
                        item_ix,
                        kind: LineFragmentKind::Object(Box::new(object)),
                        size: metrics.size,
                        source_range: 0..ATOM_LEN,
                        baseline,
                        atomic_rtl,
                    });
                }
                MeasureItem::Image { natural_size, baseline_shift, vertical_style, inline_box, .. } => {
                    if let (Some(source), Some(row), Some(metrics)) = (inline_box, &mut box_row, &box_metrics) { row.activate(source, metrics); }
                    if line_range.start <= item_start && item_end <= line_range.end {
                        if let Some(justification) = &mut justification { justification.object(line_fragments.len()); }
                        let size = image_sizes[item_ix]
                            .expect("image size should be measured before layout");
                        line_width += size.width;
                        if let Some(rank) = atom_rank { visual_order.push((line_fragments.len(), rank)); }
                        let baseline = if let Some(style) = vertical_style {
                            image_alignment_baseline(style, size.height, text_style, font_size, window)
                        } else if natural_size.is_some() { size.height } else {
                            (size.height / 2. + body_baseline - line_height / 2.).max(Pixels::ZERO).min(size.height)
                        };
                        let adjustment = inline_box.as_ref().zip(box_metrics.as_ref())
                            .map_or(Pixels::ZERO, |(scope, geometry)| geometry.adjustment(scope));
                        let baseline = baseline + adjustment
                            + font_size * if baseline_shift.is_finite() { *baseline_shift } else { 0. };
                        match vertical_style.as_ref().map(|style| style.align) {
                            Some(ImageVerticalAlign::Top) => top_image_height = top_image_height.max(size.height),
                            Some(ImageVerticalAlign::Bottom) => bottom_image_height = bottom_image_height.max(size.height),
                            _ if inline_box.is_none() => {
                                line_ascent = line_ascent.max(baseline);
                                line_descent = line_descent.max(size.height - baseline);
                            },
                            _ => has_scoped_atoms = true,
                        }
                        line_fragments.push(LineFragmentLayout {
                            item_ix,
                            kind: LineFragmentKind::Image,
                            size,
                            source_range: 0..ATOM_LEN,
                            baseline: baseline,
                            atomic_rtl,
                        });
                    }
                }
            }

            item_start = item_end;
        }
        assert!(native_parts.as_mut().is_none_or(|parts| parts.next().is_none()),
            "all native row source fragments must be distributed");

        // CSS phantom lines do not contribute height. Empty inline fonts
        // influence a real row, but do not create one by themselves. A forced
        // newline preserves the row it terminates, not a trailing caret row.
        // Legacy Markdown/editor flows keep their existing empty-row policy.
        if box_metrics.is_some() && line_fragments.is_empty() && !has_hard_break {
            previous_row_end = Some(line_range.end);
            continue;
        }

        let row_geometry = if let (Some(row), Some(metrics)) = (&mut box_row, &box_metrics) {
            row.resolve(metrics, &mut line_ascent, &mut line_descent);
            // Scoped atomic content contributes to its nearest edge subtree.
            // Own top/bottom images remain independent line-edge constraints.
            // Read provisional row shifts here, then apply each fragment's
            // final translation exactly once after all groups are positioned.
            if has_scoped_atoms {
                for fragment in &line_fragments {
                    let scope = match (&fragment.kind, &items[fragment.item_ix]) {
                        (LineFragmentKind::Object(_), MeasureItem::Object { inline_box, .. }) => inline_box.as_ref(),
                        (LineFragmentKind::Image, MeasureItem::Image { inline_box, vertical_style, .. }) => {
                            if vertical_style.as_ref().is_some_and(|style| matches!(style.align, ImageVerticalAlign::Top | ImageVerticalAlign::Bottom)) {
                                None
                            } else { inline_box.as_ref() }
                        },
                        _ => None,
                    };
                    if let Some(scope) = scope {
                        let baseline = fragment.baseline + row.shift(scope, metrics) - metrics.shift(scope);
                        row.atomic(scope, baseline, fragment.size.height, metrics, &mut line_ascent, &mut line_descent);
                    }
                }
            }
            row.finish(metrics, &mut line_ascent, &mut line_descent, top_image_height, bottom_image_height)
        } else {
            line_ascent = line_ascent.max(bottom_image_height - line_descent);
            line_descent = line_descent.max(top_image_height - line_ascent);
            None
        };
        if let (Some(geometry), Some(metrics)) = (&row_geometry, &box_metrics) {
            for fragment in &mut line_fragments {
                match &mut fragment.kind {
                    LineFragmentKind::Text { highlights, .. } => {
                        if let Some(scope) = highlights.iter().find_map(|(_, mark)| mark.inline_box.as_ref()) {
                            fragment.baseline += geometry.shift(scope) - metrics.shift(scope);
                        }
                        for (_, mark) in highlights {
                            if mark.inline_box.is_some() { mark.inline_geometry = Some(geometry.clone()); }
                        }
                    },
                    LineFragmentKind::Image | LineFragmentKind::Object(_) => {
                        let scope = match &items[fragment.item_ix] {
                            MeasureItem::Image { inline_box, .. } | MeasureItem::Object { inline_box, .. } => inline_box.as_ref(),
                            _ => unreachable!("atomic fragment retains its measured item"),
                        };
                        if let Some(scope) = scope {
                            fragment.baseline += geometry.shift(scope) - metrics.shift(scope);
                        }
                    },
                }
            }
        }
        first_baseline.get_or_insert(y + line_ascent);
        let available_width = wrap_width.unwrap_or(line_width);
        let mut hanging = Pixels::ZERO;
        if let Some(justification) = justification {
            let spacing = justification.apply(&mut line_fragments, line_width, available_width, soft_wrap, justified && soft_wrap);
            line_width = spacing.width;
            hanging = spacing.hanging;
        }
        let physical_offsets = (!visual_order.is_empty()).then(|| {
            // Position by visual rank without reordering the children. Their
            // source order still owns reading order and selection registration.
            let mut offsets = vec![Pixels::ZERO; line_fragments.len()];
            assert_eq!(visual_order.len(), line_fragments.len(), "a reordered native row must rank every painted fragment");
            visual_order.sort_unstable_by_key(|(_, rank)| *rank);
            let mut x = Pixels::ZERO;
            for &(fragment, _) in &visual_order {
                offsets[fragment] = x;
                x += line_fragments[fragment].size.width;
            }
            offsets
        });
        let alignment = if justified || logical_alignment.is_some() {
            let end = logical_alignment.is_some_and(|(alignment, _)| alignment == LogicalAlignment::End) && !justified;
            if (line.direction == TextDirection::Rtl) != end { TextAlign::Right } else { TextAlign::Left }
        } else { text_style.text_align };
        let alignment_offset = match alignment {
            TextAlign::Left => Pixels::ZERO,
            TextAlign::Center => (available_width - line_width) / 2.,
            TextAlign::Right => available_width - line_width,
        } - if line.direction == TextDirection::Rtl { hanging } else { Pixels::ZERO };
        if line_range.is_empty() {
            empty_lines.push((
                line_range.start,
                Bounds::new(
                    point(alignment_offset, y),
                    size(Pixels::ZERO, line_ascent + line_descent),
                ),
            ));
        }
        let mut x = alignment_offset;
        for (fragment_index, mut fragment) in line_fragments.into_iter().enumerate() {
            if let Some(offsets) = &physical_offsets {
                x = alignment_offset + offsets[fragment_index];
            }
            if let MeasureItem::Image { vertical_style: Some(style), .. } = &items[fragment.item_ix] {
                match style.align {
                    ImageVerticalAlign::Top => fragment.baseline = line_ascent,
                    ImageVerticalAlign::Bottom => fragment.baseline = fragment.size.height - line_descent,
                    _ => {},
                }
            }
            // Give a text run's glyph box equal leading on both sides, as
            // much as the line allows: its glyphs are painted centered in the
            // box, so this keeps them on the line's baseline, and a body run
            // gets exactly the line box. The leading is negative where the
            // glyphs overflow the line, as a plain text line's do.
            let leading = match fragment.kind {
                LineFragmentKind::Text { .. } => (line_ascent - fragment.baseline)
                    .min(line_descent - (fragment.size.height - fragment.baseline))
                    .max(-fragment.size.height / 2.),
                _ => Pixels::ZERO,
            };
            fragment.size.height += leading * 2.;
            fragment.baseline += leading;
            let origin = point(x, y + line_ascent - fragment.baseline);
            let selection_bounds = Bounds::new(
                point(x, y),
                size(fragment.size.width, line_ascent + line_descent),
            );
            let positioned = match fragment.kind {
                LineFragmentKind::Object(object) => PositionedFragment::Object {
                    item_ix: fragment.item_ix,
                    origin,
                    baseline: y + line_ascent,
                    object,
                    selection_bounds,
                    inline_geometry: row_geometry.clone(),
                    rtl: fragment.atomic_rtl,
                },
                LineFragmentKind::Text {
                    font_size,
                    text,
                    links,
                    highlights,
                    code_background,
                    spacing,
                    prepared_layout,
                } => PositionedFragment::Text {
                    item_ix: fragment.item_ix,
                    origin,
                    baseline: y + line_ascent,
                    size: fragment.size,
                    source_range: fragment.source_range,
                    selection_bounds,
                    font_size,
                    text,
                    links,
                    highlights,
                    spacing,
                    prepared_layout,
                    code_background: code_background
                        .map(|code| code.offset(origin + point(Pixels::ZERO, leading))),
                },
                LineFragmentKind::Image => PositionedFragment::Image {
                    item_ix: fragment.item_ix,
                    origin,
                    baseline: y + line_ascent,
                    size: fragment.size,
                    selection_bounds,
                    inline_geometry: row_geometry.clone(),
                    rtl: fragment.atomic_rtl,
                },
            };
            x += fragment.size.width;
            fragments.push(positioned);
        }

        max_width = max_width.max(line_width);
        y += line_ascent + line_descent;
        previous_row_end = Some(line_range.end);
    }

    InlineFlowLayout {
        native_failure: None,
        box_metrics,
        fragments,
        empty_lines,
        first_baseline,
        // Alignment's containing width is zero for intrinsic-min probes; it
        // must not replace the width of their indivisible shaped fragments.
        size: size(if wrap_width.is_some_and(|width| width <= Pixels::ZERO)
            || (!justified && logical_alignment.is_none() && text_style.text_align == TextAlign::Left) {
            max_width
        } else { wrap_width.unwrap_or(max_width) }, y),
        wrap_width,
    }
}

/// The rounded box behind a code span, relative to the fragment: centered on
/// the capital-height body of the text, with the descender room shared
/// between both sides instead of added only below. Takes the background
/// color out of `highlights`, so the text does not paint it a second time
/// at the full line height.
fn code_background(
    shaped_line: &ShapedLine,
    runs: &[TextRun],
    font_size: Pixels,
    highlights: &mut [(Range<usize>, InlineHighlight)],
    fragment_size: Size<Pixels>,
    baseline: Pixels,
    window: &Window,
) -> Option<CodeBox> {
    let cap_height = runs
        .iter()
        .map(|run| {
            let font = window.text_system().resolve_font(&run.font);
            window.text_system().cap_height(font, font_size)
        })
        .fold(Pixels::ZERO, Pixels::max);
    let vertical_padding = font_size * 0.125 + shaped_line.descent / 2.;
    let color = highlights
        .iter()
        .find_map(|(_, h)| h.style.background_color);
    let mut style = highlights
        .iter()
        .find_map(|(_, h)| h.code_style.clone())
        .unwrap_or_default();
    if style.background.is_none() {
        style.background = color.map(Into::into);
    }
    for (_, highlight) in highlights.iter_mut() {
        highlight.style.background_color = None;
    }
    let insets = code_insets(highlights, font_size, window.rem_size());
    let top = if style.padding.top.is_some() {
        shaped_line.ascent - cap_height + insets.top
    } else {
        vertical_padding
            + style
                .border_widths
                .top
                .map(|width| width.to_pixels(window.rem_size()))
                .unwrap_or_default()
    };
    let bottom = if style.padding.bottom.is_some() {
        shaped_line.descent + insets.bottom
    } else {
        vertical_padding
            + style
                .border_widths
                .bottom
                .map(|width| width.to_pixels(window.rem_size()))
                .unwrap_or_default()
    };
    Some(CodeBox {
        bounds: Bounds::new(
            point(Pixels::ZERO, baseline - cap_height - top),
            size(fragment_size.width, cap_height + top + bottom),
        ),
        style: Arc::new(style),
    })
}

struct FlowLineRange { range: Range<usize>, direction: TextDirection, shaping_direction: TextDirection }

fn hard_paragraph_direction(items: &[MeasureItem], range: &Range<usize>, direction: TextDirection, spans: &[ShapedSpan]) -> Option<TextDirection> {
    if direction != TextDirection::Auto { return Some(direction); }
    if let [ShapedSpan::Text { line, contexts: Some(_), .. }] = spans {
        return line.detected_paragraph_direction();
    }
    let mut offset = 0;
    let fragments = items.iter().scan((), |_, item| {
        let start = offset;
        offset += item.len();
        if start >= range.end { return None; }
        Some(match item {
            MeasureItem::Text { text, .. } if offset > range.start => {
                Some(&text[range.start.saturating_sub(start)..range.end.min(offset) - start])
            },
            _ => None,
        })
    }).flatten();
    TextDirection::detect_fragments(fragments)
}

fn line_ranges(
    items: &[MeasureItem],
    image_sizes: &[Option<Size<Pixels>>],
    objects: &[Option<MeasuredInlineObject>],
    text_style: &TextStyle,
    wrap_width: Option<Pixels>,
    collapse_spaces: bool,
    neutral_direction: Option<TextDirection>,
    require_native_layout: bool,
    window: &mut Window,
) -> Result<(Vec<FlowLineRange>, Vec<ShapedSpan>), super::native_layout::NativeLayoutFailure> {
    let total_len = items.iter().map(MeasureItem::len).sum::<usize>();
    let contextual = contextual::Paragraph::new(items, image_sizes, objects, text_style, window);
    if require_native_layout && contextual.is_none()
        && (items.iter().any(|item| matches!(item, MeasureItem::Text { text, .. } if !text.is_empty()))
            || (text_style.text_direction == TextDirection::Rtl && items.iter()
                .filter(|item| matches!(item, MeasureItem::Image { .. } | MeasureItem::Object { .. }))
                .take(2).count() == 2)) {
        return Err(super::native_layout::NativeLayoutFailure::UnsupportedSource);
    }
    let mut hard_lines = Vec::new();
    let mut line_start = 0;
    let mut item_start = 0;

    for item in items {
        if let MeasureItem::Text { text, .. } = item {
            for (newline, _) in text.match_indices('\n') {
                let newline = item_start + newline;
                hard_lines.push(line_start..newline);
                line_start = newline + 1;
            }
        }
        item_start += item.len();
    }
    hard_lines.push(line_start..total_len);
    let mut previous_direction = neutral_direction.unwrap_or(TextDirection::Ltr).resolve("");

    let Some(wrap_width) = wrap_width else {
        let mut spans = Vec::new();
        let ranges = hard_lines.into_iter().map(|range| {
            let shaped = shaped_spans(items, image_sizes, objects, text_style, &range, contextual.as_ref(), require_native_layout, window)?;
            let strong = hard_paragraph_direction(items, &range, text_style.text_direction, &shaped);
            let shaping_direction = strong.unwrap_or(TextDirection::Ltr);
            let direction = strong.unwrap_or(previous_direction);
            previous_direction = direction;
            spans.extend(shaped);
            Ok(FlowLineRange { range, direction, shaping_direction })
        }).collect::<Result<Vec<_>, super::native_layout::NativeLayoutFailure>>()?;
        return Ok((ranges, spans));
    };
    let rem_size = window.rem_size();
    let font_size = text_style.font_size.to_pixels(rem_size);
    let mut wrapper = window
        .text_system()
        .line_wrapper(text_style.font(), font_size);
    let mut ranges = Vec::new();
    let mut source_spans = Vec::new();

    for hard_line in hard_lines {
        let mut spans = shaped_spans(items, image_sizes, objects, text_style, &hard_line, contextual.as_ref(), require_native_layout, window)?;
        let strong = hard_paragraph_direction(items, &hard_line, text_style.text_direction, &spans);
        let shaping_direction = strong.unwrap_or(TextDirection::Ltr);
        let direction = strong.unwrap_or(previous_direction);
        previous_direction = direction;
        if hard_line.is_empty() {
            ranges.push(FlowLineRange { range: hard_line, direction, shaping_direction });
            continue;
        }
        if let Some(paragraph) = contextual.as_ref()
            && matches!(spans.as_slice(), [ShapedSpan::Text { contexts: Some(_), .. }])
        {
            let (rows, span) = paragraph.wrap(spans.pop().expect("one contextual paragraph"), text_style,
                &hard_line, wrap_width, collapse_spaces, require_native_layout, &mut wrapper, window)?;
            ranges.extend(rows.into_iter().map(|range| FlowLineRange { range, direction, shaping_direction }));
            source_spans.push(span);
            continue;
        }
        let mut wrap_fragments = Vec::new();
        {
            let mut item_start = 0;
            for (ix, item) in items.iter().enumerate() {
                let item_end = item_start + item.len();
                if item_end > hard_line.start && item_start < hard_line.end {
                    match item {
                        MeasureItem::Box { .. } => {},
                        MeasureItem::Text {
                            text, highlights, ..
                        } => {
                            let start = hard_line.start.max(item_start) - item_start;
                            let end = hard_line.end.min(item_end) - item_start;
                            if start < end {
                                push_text_wrap_fragments(
                                    &mut wrap_fragments,
                                    text,
                                    highlights,
                                    start..end,
                                    text_style,
                                    wrap_width,
                                    window,
                                );
                            }
                        }
                        MeasureItem::Object { no_wrap, .. } => {
                            let width = objects[ix].as_ref().unwrap().metrics.size.width;
                            if *no_wrap { wrap_fragments.push(WrapLineFragment::no_wrap_inline_object(width)); }
                            else { wrap_fragments.push(WrapLineFragment::inline_object(width)); }
                        }
                        MeasureItem::Image { no_wrap, .. } => {
                            if hard_line.start <= item_start && item_end <= hard_line.end {
                                let width = image_sizes[ix].expect("image size should be measured before wrapping").width;
                                if *no_wrap { wrap_fragments.push(WrapLineFragment::no_wrap_inline_object(width)); }
                                else { wrap_fragments.push(WrapLineFragment::inline_object(width)); }
                            }
                        }
                    }
                }
                item_start = item_end;
            }

        }

        // The wrapper sums per-character widths, which can be narrower than the
        // shaped run (CoreText sets an isolated full-width `，` at half width).
        // Check each line against the shaped spans and re-wrap it tighter when
        // it overflows, so the painted line never exceeds `wrap_width`.
        if wrap_width <= Pixels::ZERO {
            // Consume the native grapheme/no-wrap iterator once. Restarting
            // it for every tiny row would repeatedly scan the remaining
            // source and allocate suffix-fragment vectors during min sizing.
            // Flow rows have no continuation-indent placement. Repeating the
            // source's leading indent here would only reduce their width and
            // would not match CSS pre-wrap continuation lines.
            let mut start = hard_line.start;
            for boundary in wrapper.wrap_line_with_space_policy(
                &wrap_fragments, wrap_width, gpui::IndentAdjustment::NoIndent, collapse_spaces,
            ) {
                let end = (hard_line.start + boundary.ix).min(hard_line.end);
                if end > start { ranges.push(FlowLineRange { range: start..end, direction, shaping_direction }); start = end; }
            }
            if start < hard_line.end { ranges.push(FlowLineRange { range: start..hard_line.end, direction, shaping_direction }); }
            source_spans.extend(spans);
            continue;
        }
        let width_of = |range| if collapse_spaces { collapsed_spans_width(&spans, range) } else { spans_width(&spans, range) };
        let mut start = hard_line.start;
        while start < hard_line.end {
            let rest = fragments_from(&wrap_fragments, start - hard_line.start);
            let mut budget = wrap_width;
            let mut end = next_wrap(&mut wrapper, &rest, budget, start, hard_line.end, collapse_spaces);
            let mut overflow = width_of(start..end) - wrap_width;
            // Shrink the wrapper's budget by the overflow until it breaks earlier,
            // doubling the step while the break stays put. A single glyph wider
            // than the line keeps its line and overflows as before.
            let mut step = overflow;
            while overflow > Pixels::ZERO {
                budget -= step.max(px(0.5));
                if budget <= Pixels::ZERO {
                    break;
                }
                let tighter = next_wrap(&mut wrapper, &rest, budget, start, hard_line.end, collapse_spaces);
                if tighter < end {
                    end = tighter;
                    overflow = width_of(start..end) - wrap_width;
                    step = overflow;
                } else {
                    step = step * 2.;
                }
            }
            // The wrapper can also break a rest that fits: kerning makes a
            // shaped run narrower than its summed characters. A table column
            // sized to its measured text would drop its last character.
            if end < hard_line.end && width_of(start..hard_line.end) <= wrap_width {
                end = hard_line.end;
            }
            ranges.push(FlowLineRange { range: start..end, direction, shaping_direction });
            start = end;
        }

        if hard_line.is_empty() {
            ranges.push(FlowLineRange { range: hard_line.clone(), direction, shaping_direction });
        }
        source_spans.extend(spans);
    }

    Ok((ranges, source_spans))
}

/// Where the first line of `fragments` (which start at byte `start`) wraps at
/// `wrap_width`; `end` when it all fits.
fn next_wrap(
    wrapper: &mut gpui::LineWrapperHandle,
    fragments: &[WrapLineFragment],
    wrap_width: Pixels,
    start: usize,
    end: usize,
    collapse_spaces: bool,
) -> usize {
    wrapper
        .wrap_line_with_space_policy(fragments, wrap_width, gpui::IndentAdjustment::NoIndent, collapse_spaces)
        .next()
        .map_or(end, |boundary| (start + boundary.ix).min(end))
}

/// `fragments` with their first `skip` bytes dropped. Wrap boundaries fall on
/// character or element starts, so a text fragment is split on a char boundary.
fn fragments_from<'a>(
    fragments: &[WrapLineFragment<'a>],
    skip: usize,
) -> Vec<WrapLineFragment<'a>> {
    let mut offset = 0;
    let mut out = Vec::with_capacity(fragments.len());
    for fragment in fragments {
        let len = match fragment {
            WrapLineFragment::Text { text } | WrapLineFragment::ShapedText { text, .. }
                | WrapLineFragment::NoWrapText { text, .. } | WrapLineFragment::MeasuredText { text, .. } => text.len(),
            WrapLineFragment::Element { len_utf8, .. } | WrapLineFragment::NoWrap { len_utf8, .. } => *len_utf8,
            WrapLineFragment::InlineObject { .. } => ATOM_LEN,
        };
        if offset + len <= skip {
            offset += len;
            continue;
        }
        out.push(match fragment {
            WrapLineFragment::Text { text } => {
                WrapLineFragment::text(&text[skip.saturating_sub(offset)..])
            }
            WrapLineFragment::ShapedText { text, layout, logical_advances, no_wrap_ranges, space_policy, space_policy_ranges, inline_objects, offset: source_offset, padding } => {
                let skipped = skip.saturating_sub(offset);
                WrapLineFragment::ShapedText { text: &text[skipped..], layout: layout.clone(), logical_advances: logical_advances.clone(), no_wrap_ranges, space_policy: *space_policy, space_policy_ranges, inline_objects, offset: source_offset + skipped, padding: *padding }
            }
            WrapLineFragment::Element { width, len_utf8 } => {
                WrapLineFragment::element(*width, *len_utf8)
            }
            WrapLineFragment::InlineObject { width, no_wrap } => {
                assert!(skip <= offset, "a continuation cannot split an inline object");
                if *no_wrap { WrapLineFragment::no_wrap_inline_object(*width) }
                else { WrapLineFragment::inline_object(*width) }
            }
            WrapLineFragment::NoWrapText { text, width } => {
                assert!(skip <= offset, "a continuation cannot split protected text");
                WrapLineFragment::no_wrap_text(text, *width)
            }
            WrapLineFragment::MeasuredText { text, width } => {
                assert!(skip <= offset, "a continuation cannot split a measured text unit");
                WrapLineFragment::measured_text(text, *width)
            }
            WrapLineFragment::NoWrap { width, len_utf8 } => WrapLineFragment::NoWrap { width: *width, len_utf8: *len_utf8 },
        });
        offset += len;
    }
    out
}

/// One run of a hard line as the renderer lays it out: text shaped once in
/// context, or an object / image of fixed width. Ranges are hard-line offsets.
enum ShapedSpan {
    Text {
        range: Range<usize>,
        line: Box<ShapedLine>,
        padding: Pixels,
        /// Unbreakable source ranges, independent of space preservation.
        nowrap_ranges: Vec<Range<usize>>,
        space_policies: Vec<(Range<usize>, gpui::TextSpacePolicy)>,
        /// Coalesced font-resolved tab geometry in hard-line source order.
        tab_stops: Vec<gpui::TabStopRun>,
        requested_fonts: smallvec::SmallVec<[gpui::FontId; 4]>,
        /// Final size/box contexts in absolute flow-byte order. Present only
        /// when the complete text paragraph passed native cluster partitioning.
        contexts: Option<Vec<(Range<usize>, f32)>>,
        /// Source-only controls and atomic objects need their original source
        /// bidi runs rather than ownership by an adjacent native font glyph.
        needs_source_runs: bool,
        /// Actual replaced objects, in hard-line UTF-8 coordinates. Native
        /// font glyphs supply context only and never paint these source units.
        atoms: Vec<gpui::InlineObjectAdvance>,
    },
    Fixed {
        range: Range<usize>,
        width: Pixels,
    },
}

fn shared_text_range(text: &SharedString, range: Range<usize>) -> SharedString {
    if range.start == 0 && range.end == text.len() { text.clone() }
    else { SharedString::from(text[range].to_string()) }
}

fn shaped_spans(
    items: &[MeasureItem],
    image_sizes: &[Option<Size<Pixels>>],
    objects: &[Option<MeasuredInlineObject>],
    text_style: &TextStyle,
    hard_line: &Range<usize>,
    contextual: Option<&contextual::Paragraph<'_>>,
    require_native_layout: bool,
    window: &mut Window,
) -> Result<Vec<ShapedSpan>, super::native_layout::NativeLayoutFailure> {
    if hard_line.is_empty() { return Ok(Vec::new()); }
    if let Some(span) = contextual.and_then(|paragraph| paragraph.shape(text_style, hard_line, window)) {
        return Ok(vec![span]);
    }
    if require_native_layout && contextual.is_some() {
        return Err(super::native_layout::NativeLayoutFailure::UnsafeClusterBoundary);
    }
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let mut spans = Vec::new();
    let mut item_start = 0;
    for (ix, item) in items.iter().enumerate() {
        let item_end = item_start + item.len();
        if item_end > hard_line.start && item_start < hard_line.end {
            match item {
                MeasureItem::Box { .. } => {},
                MeasureItem::Text {
                    text, highlights, ..
                } => {
                    let local_start = hard_line.start.max(item_start) - item_start;
                    let local_end = hard_line.end.min(item_end) - item_start;
                    for (segment, scale) in
                        text_size_ranges(text.len(), highlights, font_size, window.rem_size())
                    {
                        let start = local_start.max(segment.start);
                        let end = local_end.min(segment.end);
                        if start >= end {
                            continue;
                        }
                        let highlights = slice_ranges(highlights, start, end, |range, style| {
                            (range, style.clone())
                        });
                        let runs = text_runs_for_window(end - start, text_style, &highlights, window);
                        let line = Box::new(shape_line(
                            shared_text_range(text, start..end),
                            font_size * scale,
                            &runs,
                            text_style.text_direction,
                            window,
                        ));
                        let insets = code_insets(&highlights, font_size * scale, window.rem_size());
                        let padding = insets.left + insets.right;
                        spans.push(ShapedSpan::Text {
                            range: item_start + start..item_start + end,
                            line,
                            padding,
                            nowrap_ranges: highlights.iter().filter(|(_, mark)| mark.no_wrap).map(|(range, _)| range.clone()).collect(),
                            space_policies: highlights.iter().filter_map(|(range, mark)| mark.space_policy.map(|policy| (range.clone(), policy))).collect(),
                            requested_fonts: prepared::font_ids(runs.iter().filter(|run| run.len > 0)
                                .map(|run| window.text_system().resolve_font(&run.font))),
                            contexts: None,
                            needs_source_runs: false,
                            atoms: Vec::new(),
                            tab_stops: Vec::new(),
                        });
                    }
                }
                MeasureItem::Object { .. } => spans.push(ShapedSpan::Fixed {
                    range: item_start..item_end,
                    width: objects[ix].as_ref().unwrap().metrics.size.width,
                }),
                MeasureItem::Image { .. } => {
                    if let Some(size) = image_sizes[ix] {
                        spans.push(ShapedSpan::Fixed {
                            range: item_start..item_end,
                            width: size.width,
                        });
                    }
                }
            }
        }
        item_start = item_end;
    }
    Ok(spans)
}

/// Fit a row using the same collapsing-space policy as its final layout. Keep
/// atom boxes and code padding; only subtract the trailing text's advances.
fn collapsed_spans_width(spans: &[ShapedSpan], range: Range<usize>) -> Pixels {
    let mut width = spans_width(spans, range.clone());
    let mut tail = range.end;
    for span in spans.iter().rev() {
        let span_range = match span {
            ShapedSpan::Text { range, .. } | ShapedSpan::Fixed { range, .. } => range,
        };
        if span_range.start >= tail { continue; }
        if span_range.end < tail || span_range.end <= range.start { break; }
        let ShapedSpan::Text { line, nowrap_ranges, space_policies, .. } = span else { break; };
        let start = range.start.max(span_range.start) - span_range.start;
        let end = tail - span_range.start;
        let mut trim = end;
        let mut policy_index = space_policies.partition_point(|(range, _)| range.start < end);
        let mut nowrap_index = nowrap_ranges.partition_point(|range| range.start < end);
        for (offset, character) in line.text[start..end].char_indices().rev() {
            let index = start + offset;
            while policy_index > 0 && space_policies[policy_index - 1].0.start > index { policy_index -= 1; }
            while nowrap_index > 0 && nowrap_ranges[nowrap_index - 1].start > index { nowrap_index -= 1; }
            let policy = policy_index.checked_sub(1).and_then(|last| space_policies.get(last))
                .filter(|(range, _)| range.contains(&index)).map(|(_, policy)| policy);
            let hangs = policy.map_or_else(|| character == ' ' && !nowrap_index.checked_sub(1)
                .is_some_and(|last| nowrap_ranges[last].contains(&index)), |policy| policy.hangs_at_line_end(character));
            if !hangs { break; }
            trim = index;
        }
        if trim == end { break; }
        width -= line.advance_for_range(trim..end);
        tail = span_range.start + trim;
        if trim > start { break; }
    }
    width
}

/// Painted width of `range`: in-context glyph advances of the shaped spans it
/// covers, plus the fixed spans that lie inside it.
fn spans_width(spans: &[ShapedSpan], range: Range<usize>) -> Pixels {
    spans
        .iter()
        .map(|span| match span {
            ShapedSpan::Text {
                range: span_range,
                line,
                padding,
                atoms,
                ..
            } => {
                let start = range.start.max(span_range.start);
                let end = range.end.min(span_range.end);
                if start >= end {
                    return Pixels::ZERO;
                }
                let local = start - span_range.start..end - span_range.start;
                if atoms.is_empty() { return line.advance_for_range(local) + *padding; }
                let mut width = *padding;
                for glyph in line.runs.iter().flat_map(|run| &run.glyphs)
                    .filter(|glyph| local.contains(&glyph.index)) {
                    if atoms.binary_search_by_key(&glyph.index, |atom| atom.index).is_err() {
                        width += glyph.advance;
                    }
                }
                for atom in atoms.iter().filter(|atom| local.contains(&atom.index)) {
                    assert!(atom.source_range().end <= local.end, "measured ranges cannot split source atoms");
                    width += atom.width;
                }
                width
            }
            ShapedSpan::Fixed {
                range: span_range,
                width,
            } => {
                if range.start <= span_range.start && span_range.end <= range.end {
                    *width
                } else {
                    Pixels::ZERO
                }
            }
        })
        .sum()
}

/// Appends the wrap fragments for `range` of `text`. The line wrapper
/// measures text fragments in the body font, so a span whose highlight sets
/// another family is shaped with the same run the renderer uses and enters
/// the wrapper with shared shaped glyph positions. No-wrap ranges are atomic
/// across style/image boundaries. Legacy inline-code boxes retain their own
/// word/grapheme splitting and padding behavior.
fn push_text_wrap_fragments<'a>(
    fragments: &mut Vec<WrapLineFragment<'a>>,
    text: &'a str,
    highlights: &[(Range<usize>, InlineHighlight)],
    range: Range<usize>,
    text_style: &TextStyle,
    wrap_width: Pixels,
    window: &mut Window,
) {
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let mut cursor = range.start;
    for (highlight_range, highlight) in highlights {
        if highlight.font_family.is_none() && highlight.font_size_scale.is_none() && highlight.authored_font_size_scale.is_none()
            && highlight.style.font_weight.is_none() && highlight.style.font_style.is_none() && !highlight.no_wrap && highlight.space_policy.is_none() {
            continue;
        }
        let start = highlight_range.start.max(cursor);
        let end = highlight_range.end.min(range.end);
        if start >= end {
            continue;
        }
        if cursor < start {
            fragments.push(WrapLineFragment::text(&text[cursor..start]));
        }
        let span = &text[start..end];
        let rem_size = window.rem_size();
        let measured_font_size = highlight.font_size(font_size, rem_size);
        let insets = highlight.code_insets(measured_font_size, rem_size);
        let padding = insets.left + insets.right;
        if !highlight.no_wrap && highlight.code_style.is_none() {
            let runs = text_runs_for_window(span.len(), text_style, &[(0..span.len(), highlight.clone())], window);
            let shaped = shape_line(SharedString::from(span.to_string()), measured_font_size, &runs, text_style.text_direction, window);
            fragments.push(WrapLineFragment::shaped_text(span, &shaped, padding).with_space_policy(highlight.space_policy));
            cursor = end;
            continue;
        }
        let measure = |text: &str| {
            let runs = text_runs_for_window(
                text.len(),
                text_style,
                &[(0..text.len(), highlight.clone())],
                window,
            );
            window
                .text_system()
                .layout_line(text, measured_font_size, &runs, None)
                .width
        };
        let width = measure(span) + padding;
        if highlight.no_wrap {
            fragments.push(WrapLineFragment::no_wrap_text(span, width));
        } else if width <= wrap_width {
            fragments.push(WrapLineFragment::measured_text(span, width));
        } else {
            for word in span.split_word_bounds() {
                let width = measure(word) + padding;
                if width <= wrap_width {
                    fragments.push(WrapLineFragment::measured_text(word, width));
                } else {
                    for grapheme in word.graphemes(true) {
                        fragments.push(WrapLineFragment::measured_text(
                            grapheme,
                            measure(grapheme) + padding,
                        ));
                    }
                }
            }
        }
        cursor = end;
    }
    if cursor < range.end {
        fragments.push(WrapLineFragment::text(&text[cursor..range.end]));
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_image_size(
    ix: usize,
    source: &ImageSource,
    width: Option<DefiniteLength>,
    height: Option<DefiniteLength>,
    natural_size: Option<Size<Pixels>>,
    line_height: Pixels,
    rem_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> Size<Pixels> {
    if width.is_none() && height.is_none() {
        if let Some(size) = natural_size { return size; }
    }
    let intrinsic_size = if natural_size.is_some() { natural_size }
    else if width.is_some() && height.is_some() {
        None
    } else {
        intrinsic_image_size(ix, source, width, height, window, cx)
    };
    image_size(width, height, intrinsic_size, line_height, rem_size)
}

fn intrinsic_image_size(
    ix: usize,
    source: &ImageSource,
    width: Option<DefiniteLength>,
    height: Option<DefiniteLength>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Size<Pixels>> {
    let mut element = img(source.clone())
        .id(ix)
        .object_fit(ObjectFit::Contain)
        .max_w(relative(1.))
        .when_some(width, |this, width| this.w(width))
        .when_some(height, |this, height| this.h(height))
        .into_any_element();
    let measured_size = element.layout_as_root(AvailableSpace::min_size(), window, cx);

    if measured_size.width <= Pixels::ZERO || measured_size.height <= Pixels::ZERO {
        None
    } else {
        Some(measured_size)
    }
}

fn image_size(
    width: Option<DefiniteLength>,
    height: Option<DefiniteLength>,
    intrinsic_size: Option<Size<Pixels>>,
    line_height: Pixels,
    rem_size: Pixels,
) -> Size<Pixels> {
    let base_size = AbsoluteLength::Pixels(line_height);
    match (width, height) {
        (Some(width), Some(height)) => size(
            width.to_pixels(base_size, rem_size),
            height.to_pixels(base_size, rem_size),
        ),
        (Some(width), None) => {
            let width = width.to_pixels(base_size, rem_size);
            let height = intrinsic_size
                .and_then(|intrinsic_size| {
                    (intrinsic_size.width > Pixels::ZERO && intrinsic_size.height > Pixels::ZERO)
                        .then(|| width * (intrinsic_size.height / intrinsic_size.width))
                })
                .unwrap_or(line_height);
            size(width, height)
        }
        (None, Some(height)) => {
            let height = height.to_pixels(base_size, rem_size);
            let width = intrinsic_size
                .and_then(|intrinsic_size| {
                    (intrinsic_size.width > Pixels::ZERO && intrinsic_size.height > Pixels::ZERO)
                        .then(|| height * (intrinsic_size.width / intrinsic_size.height))
                })
                .unwrap_or(height);
            size(width, height)
        }
        (None, None) => inline_image_size_for_line(intrinsic_size, line_height),
    }
}

fn inline_image_size_for_line(
    intrinsic_size: Option<Size<Pixels>>,
    line_height: Pixels,
) -> Size<Pixels> {
    let height = line_height * 0.75;
    let aspect_ratio = intrinsic_size
        .and_then(|intrinsic_size| {
            (intrinsic_size.width > Pixels::ZERO && intrinsic_size.height > Pixels::ZERO)
                .then(|| intrinsic_size.width / intrinsic_size.height)
        })
        .unwrap_or(1.);

    size((height * aspect_ratio).max(px(1.)), height.max(px(1.)))
}

fn shape_line(
    text: SharedString,
    font_size: Pixels,
    runs: &[TextRun],
    direction: TextDirection,
    window: &mut Window,
) -> ShapedLine {
    window.text_system().shape_line_with_direction(text, font_size, runs, None, direction)
}

/// The line height GPUI's text layout gives a plain line in `text_style`.
/// Unlike `Window::line_height`, it is not first rounded to a whole logical
/// pixel, which would make a flow line taller than a plain one.
fn plain_line_height(text_style: &TextStyle, rem_size: Pixels, window: &Window) -> Pixels {
    window.pixel_snap(
        text_style
            .line_height
            .to_pixels(text_style.font_size, rem_size),
    )
}

/// Returns where GPUI text painting puts the baseline of `shaped_line` in a
/// box `line_height` tall: centered, even when the glyphs overflow the box.
/// `ShapedLine::descent` is positive; do not substitute the signed
/// `FontMetrics::descent` exposed by `TextSystem::baseline_offset` here.
fn centered_baseline(shaped_line: &ShapedLine, line_height: Pixels) -> Pixels {
    (line_height - shaped_line.ascent - shaped_line.descent) / 2. + shaped_line.ascent
}

pub(super) fn slice_ranges<T, U>(
    ranges: &[(Range<usize>, T)],
    start: usize,
    end: usize,
    map: impl Fn(Range<usize>, &T) -> U,
) -> Vec<U> {
    ranges
        .iter()
        .filter_map(|(range, value)| {
            let clipped_start = range.start.max(start);
            let clipped_end = range.end.min(end);
            (clipped_start < clipped_end)
                .then(|| map((clipped_start - start)..(clipped_end - start), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::SharedUri;

    mod frames {
        use super::super::FLOW_LAYOUTS;
        use crate::text::{TextView, TextViewState};
        use gpui::{
            AppContext as _, Context, Entity, IntoElement, ParentElement as _, Pixels, Render,
            Styled as _, TestAppContext, VisualTestContext, Window, div, px,
        };

        struct Answer {
            state: Entity<TextViewState>,
            width: Pixels,
        }

        impl Render for Answer {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(self.width).child(TextView::new(&self.state))
            }
        }

        fn draw(cx: &mut VisualTestContext) {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }

        fn layouts() -> usize {
            FLOW_LAYOUTS.with(|layouts| layouts.get())
        }

        /// Scrolling redraws every visible paragraph each frame; a flow whose
        /// text, style and width are unchanged has nothing to wrap or shape.
        #[gpui::test]
        fn an_unchanged_flow_is_not_laid_out_again(cx: &mut TestAppContext) {
            cx.update(crate::init);
            let (_, cx) = cx.add_window_view(|_, cx| Answer {
                state: cx.new(|cx| TextViewState::markdown("Call `foo` now.", cx)),
                width: px(300.),
            });
            cx.run_until_parked();
            draw(cx);
            let after_first_frame = layouts();
            assert!(after_first_frame >= 1, "the first frame lays the flow out");

            for _ in 0..3 {
                draw(cx);
            }

            assert_eq!(layouts(), after_first_frame, "a frame that changes nothing");
        }

        #[gpui::test]
        fn a_flow_is_laid_out_again_at_another_width(cx: &mut TestAppContext) {
            cx.update(crate::init);
            let (answer, cx) = cx.add_window_view(|_, cx| Answer {
                state: cx.new(|cx| TextViewState::markdown("Call `foo` now.", cx)),
                width: px(300.),
            });
            cx.run_until_parked();
            draw(cx);
            let before = layouts();

            answer.update(cx, |answer, cx| {
                answer.width = px(120.);
                cx.notify();
            });
            draw(cx);

            assert!(layouts() > before, "a narrower flow wraps differently");
        }

        #[gpui::test]
        fn a_flow_is_laid_out_again_when_its_text_changes(cx: &mut TestAppContext) {
            cx.update(crate::init);
            let (answer, cx) = cx.add_window_view(|_, cx| Answer {
                state: cx.new(|cx| TextViewState::markdown("Call `foo` now.", cx)),
                width: px(300.),
            });
            cx.run_until_parked();
            draw(cx);
            let before = layouts();

            answer.update(cx, |answer, cx| {
                answer.state.update(cx, |state, cx| {
                    state.set_text("Call `foo` and `bar` now.", cx);
                })
            });
            cx.run_until_parked();
            draw(cx);

            assert!(layouts() > before, "a new text has a new layout");
        }
    }

    #[test]
    fn atomic_objects_wrap_and_share_a_baseline_at_multiple_font_sizes() {
        use super::super::inline::test_draw::in_prepaint;
        use super::super::inline::test_fonts::{BODY, WideMonoTextSystem};
        use gpui::TestApp;
        let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
        in_prepaint(&mut app, |window, cx| {
            for scale in [1., 1.5, 2.] {
                let style = TextStyle {
                    font_family: BODY.into(),
                    font_size: px(16. * scale).into(),
                    ..Default::default()
                };
                let items = vec![
                    MeasureItem::Text {
                        text: "中文 ".into(),
                        links: vec![],
                        highlights: vec![],
                    },
                    MeasureItem::Object {
                        text: "x²".into(),
                        id: 0,
                        renderer: Arc::new(|_, _, _| None),
                        style: Default::default(),
                        no_wrap: false,
                        inline_box: None,
                    },
                    MeasureItem::Object {
                        text: "y²".into(),
                        id: 1,
                        renderer: Arc::new(|_, _, _| None),
                        style: Default::default(),
                        no_wrap: false,
                        inline_box: None,
                    },
                    MeasureItem::Text {
                        text: " English".into(),
                        links: vec![],
                        highlights: vec![],
                    },
                ];
                for width in [1., 30., 80., 500.] {
                    let layout = layout_flow(
                        &items,
                        &[None, None, None, None],
                        &style,
                        Some(px(width)),
                        window,
                        cx,
                    );
                    let objects: Vec<_> = layout
                        .fragments
                        .iter()
                        .filter_map(|fragment| match fragment {
                            PositionedFragment::Object { origin, object, .. } => {
                                Some((origin, object))
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(objects.len(), 2);
                    for (origin, object) in &objects {
                        assert!(object.metrics.size.width <= px(width));
                        assert!(origin.y >= px(0.));
                        assert!(origin.y + object.metrics.size.height <= layout.size.height);
                    }
                    if width == 500. {
                        assert_eq!(
                            objects[0].0.y + objects[0].1.metrics.baseline,
                            objects[1].0.y + objects[1].1.metrics.baseline
                        );
                    }
                }
            }
        });
    }

    #[test]
    fn inline_image_without_explicit_size_scales_intrinsic_ratio_to_line_height() {
        let line_height = px(20.);
        let intrinsic_size = size(px(160.), px(40.));

        let measured = inline_image_size_for_line(Some(intrinsic_size), line_height);

        assert_eq!(measured, size(px(60.), px(15.)));
    }

    #[test]
    fn inline_image_without_intrinsic_size_uses_compact_square_fallback() {
        let measured = inline_image_size_for_line(None, px(20.));

        assert_eq!(measured, size(px(15.), px(15.)));
    }

    /// Line breaking must see the width of an inline code span in its own
    /// family. With a body-font-only wrapper the span below is measured at
    /// half its shaped width, the line is kept whole, and the flow reports a
    /// width past `wrap_width`.
    #[test]
    fn inline_code_near_the_wrap_width_does_not_overflow_the_flow() {
        use super::super::inline::test_fonts::{BODY, MONO, WideMonoTextSystem};
        use gpui::{AbsoluteLength, Empty, HighlightStyle, TestApp};

        let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
        let mut window = app.open_window(|_, _| Empty);

        let font_size = px(10.);
        let text_style = TextStyle {
            font_family: SharedString::from(BODY),
            font_size: AbsoluteLength::Pixels(font_size),
            ..Default::default()
        };
        let lead = SharedString::from("See ");
        let tail_text = " with code_span_here end";
        let code = tail_text.find("code_span_here").unwrap();
        let code_range = code..code + "code_span_here".len();
        let code_highlight = InlineHighlight {
            style: HighlightStyle::default(),
            font_family: Some(SharedString::from(MONO)),
            font_size_scale: None,
            authored_font_size_scale: None,
            authored_baseline_shift: None,
            inline_box: None,
            inline_geometry: None,
            no_wrap: false,
            space_policy: None,
            decoration_metrics: None,
            decoration_layers: None,
            code_style: None,
        };
        let items = vec![
            MeasureItem::Text {
                text: lead.clone(),
                links: vec![],
                highlights: vec![],
            },
            MeasureItem::Image {
                source: SharedUri::from("https://example.com/badge.png").into(),
                width: None,
                height: None,
                natural_size: None,
                baseline_shift: 0.,
                vertical_style: None,
                no_wrap: false,
                inline_box: None,
            },
            MeasureItem::Text {
                text: SharedString::from(tail_text),
                links: vec![],
                highlights: vec![(code_range.clone(), code_highlight)],
            },
        ];
        let image_size = size(px(10.), px(10.));
        let image_sizes = vec![None, Some(image_size), None];
        // Body text, the image and the mono span fill the wrap width exactly;
        // the trailing "end" only fits if the span is under-measured.
        let wrap_width = WideMonoTextSystem::width_of("See  with  ", BODY, font_size)
            + image_size.width
            + WideMonoTextSystem::width_of("code_span_here", MONO, font_size);

        let layout = window.update(|_, window, cx| {
            layout_flow(
                &items,
                &image_sizes,
                &text_style,
                Some(wrap_width),
                window,
                cx,
            )
        });

        assert!(
            layout.size.width <= wrap_width,
            "flow width {:?} exceeds wrap width {:?}",
            layout.size.width,
            wrap_width
        );
        let mono_fragment_width = layout
            .fragments
            .iter()
            .find_map(|fragment| match fragment {
                PositionedFragment::Text { text, size, .. } if text.contains("code_span_here") => {
                    Some(size.width)
                }
                _ => None,
            })
            .expect("the code span is laid out as a text fragment");
        assert!(
            mono_fragment_width >= WideMonoTextSystem::width_of("code_span_here", MONO, font_size),
            "the code span fragment is shaped in the mono family"
        );
        // The image sits vertically centred in its line, so line membership is
        // read off the text fragments only.
        let text_lines = layout
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                PositionedFragment::Text { text, origin, .. } => Some((text.trim(), origin.y)),
                PositionedFragment::Image { .. } | PositionedFragment::Object { .. } => None,
            })
            .collect::<Vec<_>>();
        let first_y = text_lines[0].1;
        assert!(
            text_lines
                .iter()
                .any(|(text, y)| text.contains("code_span_here") && *y == first_y),
            "the span stays on the first line: {text_lines:?}"
        );
        assert!(
            text_lines
                .iter()
                .any(|(text, y)| *text == "end" && *y > first_y),
            "the trailing word wraps to a second line: {text_lines:?}"
        );
    }
    #[test]
    fn long_inline_code_wraps_in_mixed_flow() {
        use super::super::inline::test_fonts::{BODY, MONO, WideMonoTextSystem};
        use gpui::{Empty, TestApp};
        let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
        let mut window = app.open_window(|_, _| Empty);
        let style = TextStyle {
            font_family: BODY.into(),
            font_size: AbsoluteLength::Pixels(px(10.)),
            ..Default::default()
        };
        for text in [
            "one two three four five six",
            "very_long_unbroken_identifier",
            "你好世界你好世界你好世界",
            "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
        ] {
            let items = vec![
                MeasureItem::Image {
                    source: SharedUri::from("https://example.com/icon.png").into(),
                    width: None,
                    height: None,
                    natural_size: None,
                    baseline_shift: 0.,
                    vertical_style: None,
                    no_wrap: false,
                    inline_box: None,
                },
                MeasureItem::Text {
                    text: text.into(),
                    links: vec![],
                    highlights: vec![(
                        0..text.len(),
                        InlineHighlight {
                            font_family: Some(MONO.into()),
                            ..Default::default()
                        },
                    )],
                },
            ];
            let image_sizes = vec![Some(size(px(10.), px(10.))), None];
            let layout = window.update(|_, window, cx| {
                layout_flow(&items, &image_sizes, &style, Some(px(100.)), window, cx)
            });
            assert!(layout.size.width <= px(100.), "{text:?}: {:?}", layout.size);
            let reconstructed: String = layout
                .fragments
                .iter()
                .filter_map(|fragment| match fragment {
                    PositionedFragment::Text { text, .. } => Some(text.as_ref()),
                    _ => None,
                })
                .collect();
            assert_eq!(reconstructed, text);
        }
    }

    #[test]
    fn inline_code_line_is_as_tall_as_a_plain_line_when_glyphs_overflow_it() {
        use super::super::inline::test_fonts::{BODY, MONO, WideMonoTextSystem};
        use gpui::{Empty, TestApp};

        let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
        let mut window = app.open_window(|_, _| Empty);
        let style = TextStyle {
            font_family: BODY.into(),
            font_size: AbsoluteLength::Pixels(px(40.)),
            line_height: px(30.).into(),
            ..Default::default()
        };
        let items = vec![MeasureItem::Text {
            text: "a code z".into(),
            links: vec![],
            highlights: vec![(
                2..6,
                InlineHighlight {
                    font_family: Some(MONO.into()),
                    font_size_scale: Some(0.875),
                    ..Default::default()
                },
            )],
        }];
        // The 40px glyphs are taller than the 30px line. A plain text line
        // keeps the line height and lets them overflow, so a line with
        // inline code must not grow to fit them.
        window.update(|_, window, cx| {
            let layout = layout_flow(&items, &[None], &style, None, window, cx);
            assert_eq!(layout.size.height, window.pixel_snap(px(30.)));
        });
    }

    #[test]
    fn inline_code_style_keeps_equal_size_prose_separate_and_measures_its_box() {
        use super::super::inline::test_fonts::{BODY, MONO, WideMonoTextSystem};
        use gpui::{Empty, HighlightStyle, TestApp, rems};
        let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
        let mut window = app.open_window(|_, _| Empty);
        for rem_size in [px(14.), px(28.)] {
            window.update(|_, window, cx| {
                window.set_rem_size(rem_size);
                let text_style = TextStyle {
                    font_family: BODY.into(),
                    font_size: rem_size.into(),
                    ..Default::default()
                };
                let code_style = StyleRefinement::default()
                    .text_size(rems(1.))
                    .px(rems(0.25))
                    .py(rems(0.125))
                    .bg(gpui::red())
                    .border_1()
                    .border_color(gpui::blue())
                    .rounded(rems(0.25));
                let highlight = InlineHighlight {
                    font_family: Some(MONO.into()),
                    font_size_scale: Some(0.875),
                    code_style: Some(code_style.clone()),
                    authored_font_size_scale: None,
                    authored_baseline_shift: None,
                    inline_box: None,
                    inline_geometry: None,
                    no_wrap: false,
                    space_policy: None,
                    decoration_metrics: None,
            decoration_layers: None,
                    style: HighlightStyle {
                        background_color: Some(gpui::green()),
                        ..Default::default()
                    },
                };
                let items = vec![MeasureItem::Text {
                    text: "before code after".into(),
                    links: vec![],
                    highlights: vec![(7..11, highlight)],
                }];
                let layout = layout_flow(&items, &[None], &text_style, None, window, cx);
                assert_eq!(layout.fragments.len(), 3);
                for fragment in &layout.fragments {
                    let PositionedFragment::Text {
                        text,
                        size,
                        font_size,
                        code_background,
                        ..
                    } = fragment
                    else {
                        panic!("expected text");
                    };
                    assert_eq!(*font_size, rem_size);
                    if text.as_ref() == "code" {
                        let code = code_background.as_ref().expect("code box");
                        assert_eq!(code.style.background, code_style.background);
                        assert_eq!(code.style.border_color, Some(gpui::blue()));
                        assert_eq!(code.style.corner_radii, code_style.corner_radii);
                        assert_eq!(code.bounds.size.width, size.width);
                        assert_eq!(
                            size.width,
                            WideMonoTextSystem::width_of("code", MONO, rem_size)
                                + rem_size * 0.5
                                + px(2.)
                        );
                    } else {
                        assert!(code_background.is_none());
                        assert_eq!(
                            size.width,
                            WideMonoTextSystem::width_of(text, BODY, rem_size)
                        );
                    }
                }
                // The same insets must be included by wrapping, not just drawing.
                let wrap_width = WideMonoTextSystem::width_of("before ", BODY, rem_size)
                    + WideMonoTextSystem::width_of("code", MONO, rem_size)
                    + rem_size * 0.5
                    + px(1.);
                let wrapped =
                    layout_flow(&items, &[None], &text_style, Some(wrap_width), window, cx);
                assert!(wrapped.size.height > layout.size.height);
                assert!(wrapped.size.width <= wrap_width);
                let reconstructed = wrapped
                    .fragments
                    .iter()
                    .filter_map(|fragment| match fragment {
                        PositionedFragment::Text { text, .. } => Some(text.as_ref()),
                        _ => None,
                    })
                    .collect::<String>();
                assert_eq!(reconstructed, "before code after");
            });
        }
    }

    #[test]
    fn inline_code_size_is_relative_and_uses_the_native_body_baseline() {
        use super::super::inline::test_fonts::{BODY, MONO, WideMonoTextSystem};
        use gpui::{Empty, TestApp};

        let mut app = TestApp::with_text_system(Arc::new(WideMonoTextSystem));
        let mut window = app.open_window(|_, _| Empty);
        // These are the 16px body size and the example-markdown preview's
        // 1.25x/1.5x inherited text-size zooms.
        for body_size in [16., 20., 24.] {
            let style = TextStyle {
                font_family: BODY.into(),
                font_size: AbsoluteLength::Pixels(px(body_size)),
                ..Default::default()
            };
            let items = vec![MeasureItem::Text {
                text: "a code z".into(),
                links: vec![],
                highlights: vec![(
                    2..6,
                    InlineHighlight {
                        font_family: Some(MONO.into()),
                        font_size_scale: Some(0.875),
                        ..Default::default()
                    },
                )],
            }];
            window.update(|_, window, cx| {
                let layout = layout_flow(&items, &[None], &style, None, window, cx);
                let text_fragments = layout
                    .fragments
                    .iter()
                    .filter_map(|fragment| match fragment {
                        PositionedFragment::Text {
                            text,
                            font_size,
                            origin,
                            size,
                            ..
                        } => Some((text.as_ref(), *font_size, origin.y, *size)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(text_fragments.len(), 3);
                assert_eq!(text_fragments[0].1, px(body_size));
                assert_eq!(text_fragments[1].0, "code");
                assert_eq!(text_fragments[1].1, px(body_size * 0.875));
                // The code run's box stays inside the line, so whatever it
                // paints over its box cannot reach a neighboring line.
                assert!(text_fragments[1].2 >= Pixels::ZERO);
                assert!(text_fragments[1].2 + text_fragments[1].3.height <= layout.size.height);
                assert_eq!(
                    text_fragments[1].3.width,
                    WideMonoTextSystem::width_of("code", MONO, px(body_size * 0.875))
                        + px(INLINE_CODE_PADDING * 2.)
                );

                // GPUI paints shaped lines with positive `LineLayout::descent`.
                // Derive the ordinary body glyph baseline directly from that
                // native-compatible shaped line, rather than `baseline_offset`,
                // whose FontMetrics descent is intentionally signed.
                let body_runs = text_runs(1, &style, &[]);
                let body_line = shape_line("a".into(), px(body_size), &body_runs, style.text_direction, window);
                let body_line_height = plain_line_height(&style, window.rem_size(), window);
                let plain_body_baseline = (body_line_height - body_line.ascent - body_line.descent)
                    / 2.
                    + body_line.ascent;
                // The smaller code run fits inside the body line, so the
                // line must be exactly as tall as a plain one.
                assert_eq!(
                    layout.size.height, body_line_height,
                    "{body_size}px line height"
                );
                let mut painted_glyph_baseline =
                    |fragment: &(&str, Pixels, Pixels, Size<Pixels>)| {
                        let runs = if fragment.0 == "code" {
                            text_runs(
                                fragment.0.len(),
                                &style,
                                &[(
                                    0..fragment.0.len(),
                                    InlineHighlight {
                                        font_family: Some(MONO.into()),
                                        font_size_scale: Some(0.875),
                                        ..Default::default()
                                    },
                                )],
                            )
                        } else {
                            text_runs(fragment.0.len(), &style, &[])
                        };
                        let shaped = shape_line(fragment.0.into(), fragment.1, &runs, style.text_direction, window);
                        fragment.2
                            + (fragment.3.height - shaped.ascent - shaped.descent) / 2.
                            + shaped.ascent
                    };

                // `origin` is passed unchanged to Inline::paint_origin, so this
                // checks the first body glyph's painted row position, not only
                // the enclosing paragraph height. It must agree with a normal
                // body line and with the adjacent inline-code glyph baseline.
                assert_eq!(
                    text_fragments[0].2,
                    Pixels::ZERO,
                    "{body_size}px body origin"
                );
                assert_eq!(
                    painted_glyph_baseline(&text_fragments[0]),
                    plain_body_baseline,
                    "{body_size}px body glyph baseline"
                );
                assert_eq!(
                    painted_glyph_baseline(&text_fragments[1]),
                    plain_body_baseline,
                    "{body_size}px code glyph baseline"
                );
            });
        }
    }
}
