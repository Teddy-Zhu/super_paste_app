use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, CursorStyle, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Refineable as _, Role, SharedString, Size, Styled, StyledText,
    TextStyle, Window, div, px, size,
};

use super::{
    InlineElement, inline::point_in_text_selection, state::LineSpan,
};
use crate::{GlobalState, touch_selection::caret_line_box};

#[derive(Clone, Copy, Debug)]
pub(super) struct InlineMetrics {
    pub size: Size<Pixels>,
    pub baseline: Pixels,
}

impl InlineMetrics {
    fn is_valid(self) -> bool {
        let width = f32::from(self.size.width);
        let height = f32::from(self.size.height);
        let baseline = f32::from(self.baseline);
        width.is_finite()
            && height.is_finite()
            && baseline.is_finite()
            && width > 0.
            && height > 0.
            && baseline >= 0.
            && baseline <= height
    }
}

/// Geometry can be cloned during wrapping; the frame's element is consumed once.
#[derive(Clone)]
pub(super) struct MeasuredInlineObject {
    pub metrics: InlineMetrics,
    content: Option<Rc<RefCell<Option<AnyElement>>>>,
    text: SharedString,
    font_size: Pixels,
    text_style: TextStyle,
}

impl MeasuredInlineObject {
    /// Must run before entering a GPUI measured-layout callback: native element
    /// layout itself uses the window's layout engine.
    pub fn measure(
        text: &str,
        presentation: Option<InlineElement>,
        style: &TextStyle,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let font_size = style.font_size.to_pixels(window.rem_size());
        let text: SharedString = text.replace(['\r', '\n'], " ").into();
        let line = window.text_system().shape_line(
            text.clone(),
            font_size,
            &[style.to_run(text.len())],
            None,
        );
        let height = style
            .line_height_in_pixels(window.rem_size())
            .max(line.ascent + line.descent);
        let fallback = InlineMetrics {
            size: size(line.width.max(px(1.)), height),
            baseline: (height - line.ascent - line.descent) / 2. + line.ascent,
        };
        let mut result = Self {
            metrics: fallback,
            content: None,
            text,
            font_size,
            text_style: style.clone(),
        };
        if let Some(presentation) = presentation {
            // The wrapper carries inherited marks into both layout and painting.
            let mut wrapper = div().child(presentation.element);
            wrapper.style().text = style.subtract(&Default::default());
            let mut element = wrapper.into_any_element();
            let measured = element.layout_as_root(
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                cx,
            );
            let metrics = InlineMetrics {
                size: measured,
                baseline: presentation.baseline.unwrap_or(
                    (measured.height - (fallback.size.height - fallback.baseline))
                        .max(Pixels::ZERO),
                ),
            };
            if metrics.is_valid() {
                result.metrics = metrics;
                result.content = Some(Rc::new(RefCell::new(Some(element))));
            }
        }
        result
    }

    pub fn fit_text(mut self, width: Option<Pixels>) -> Self {
        // GPUI has no general subtree scale. Only our own plain-text fallback
        // can be proportionally fitted; custom elements retain their real size.
        if self.content.is_none() {
            let scale = width.map_or(1., |width| {
                (width.max(Pixels::ZERO) / self.metrics.size.width).min(1.)
            });
            self.metrics.size = size(
                self.metrics.size.width * scale,
                self.metrics.size.height * scale,
            );
            self.metrics.baseline *= scale;
            self.font_size *= scale;
        }
        self
    }

    fn element(&self) -> (AnyElement, bool) {
        if let Some(content) = &self.content {
            return (
                content
                    .borrow_mut()
                    .take()
                    .expect("inline element painted once per frame"),
                true,
            );
        }
        let text = StyledText::new(self.text.clone())
            .with_runs(vec![self.text_style.to_run(self.text.len())]);
        (
            div()
                .w(self.metrics.size.width)
                .h(self.metrics.size.height)
                .text_size(self.font_size)
                .line_height(self.metrics.size.height)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(text)
                .into_any_element(),
            false,
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct InlineObjectSourcePosition {
    selected: std::sync::Weak<Mutex<Option<bool>>>,
    offset: usize,
}

impl InlineObjectSourcePosition {
    pub(super) fn boundary(selected: &Arc<Mutex<Option<bool>>>, from_start: bool) -> Self {
        Self { selected: Arc::downgrade(selected), offset: if from_start { 0 } else { super::inline_flow::ATOM_LEN } }
    }

    pub(super) fn offset_in(&self, selected: &Arc<Mutex<Option<bool>>>) -> Option<usize> {
        (self.selected.as_ptr() == Arc::as_ptr(selected)).then_some(self.offset)
    }
}

pub(super) struct InlineObjectSelectionProjection {
    selected: std::sync::Weak<Mutex<Option<bool>>>,
    bounds: Bounds<Pixels>,
    line_bounds: Bounds<Pixels>,
    rtl: bool,
    word_source: Option<(std::rc::Weak<dyn crate::text_selection::WordSelectionSource>, usize)>,
}

impl InlineObjectSelectionProjection {
    pub(super) fn source_caret(&self, position: &InlineObjectSourcePosition) -> Option<Bounds<Pixels>> {
        if !self.selected.ptr_eq(&position.selected) { return None; }
        self.selected.upgrade()?;
        self.caret_for_offset(position.offset)
    }

    fn caret_for_offset(&self, offset: usize) -> Option<Bounds<Pixels>> {
        if self.bounds.size.width <= Pixels::ZERO || self.bounds.size.height <= Pixels::ZERO { return None; }
        let position = match offset {
            0 if self.rtl => self.bounds.top_right(),
            0 => self.bounds.origin,
            offset if offset == '\u{fffc}'.len_utf8() && self.rtl => self.bounds.origin,
            offset if offset == '\u{fffc}'.len_utf8() => self.bounds.top_right(),
            _ => return None,
        };
        Some(caret_line_box(position, self.bounds.size.height))
    }

    pub(super) fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    pub(super) fn select_whole(&self, value: bool) {
        if let Some(selected) = self.selected.upgrade()
            && let Ok(mut selected) = selected.lock()
        {
            *selected = Some(value);
        }
    }

    /// The native resolved direction determines which half is source trailing.
    /// Use this same insertion rule for range projection and source comparison.
    pub(super) fn endpoint_offset(&self, position: gpui::Point<Pixels>) -> usize {
        let after = if position.y < self.bounds.top() {
            false
        } else if position.y >= self.bounds.bottom() {
            true
        } else if self.rtl {
            position.x <= self.bounds.center().x
        } else {
            position.x >= self.bounds.center().x
        };
        if after { "\u{fffc}".len() } else { 0 }
    }

    pub(super) fn source_position(&self, position: gpui::Point<Pixels>) -> Option<InlineObjectSourcePosition> {
        if position.y < self.bounds.top() || position.y > self.bounds.bottom() { return None; }
        Some(InlineObjectSourcePosition { selected: self.selected.clone(), offset: self.endpoint_offset(position) })
    }

    pub(super) fn unit_at(
        &self,
        position: gpui::Point<Pixels>,
        click_count: usize,
    ) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
        if self.bounds.size.width <= Pixels::ZERO
            || self.bounds.size.height <= Pixels::ZERO
            || !self.bounds.contains(&position)
        {
            return None;
        }
        let _selection = self.selected.upgrade()?;
        let bounds = match click_count {
            2 => self.bounds,
            3.. => self.line_bounds,
            _ => return None,
        };
        let (start, end) = if click_count == 2 && self.rtl {
            (bounds.top_right(), bounds.origin)
        } else {
            (bounds.origin, bounds.top_right())
        };
        Some((caret_line_box(start, bounds.size.height), caret_line_box(end, bounds.size.height)))
    }

    pub(super) fn word_caret(
        &self,
        source: &dyn crate::text_selection::WordSelectionSource,
        segment: usize,
        offset: usize,
    ) -> Option<Bounds<Pixels>> {
        let (owner, item) = self.word_source.as_ref()?;
        let source_pointer = source as *const dyn crate::text_selection::WordSelectionSource;
        if *item != segment || !std::ptr::addr_eq(owner.as_ptr(), source_pointer) {
            return None;
        }
        self.caret_for_offset(offset)
    }

    pub(super) fn project(&self, start: gpui::Point<Pixels>, end: gpui::Point<Pixels>) {
        if let Some(selected) = self.selected.upgrade()
            && let Ok(mut selected) = selected.lock()
        {
            *selected = Some(object_in_selection(self.bounds, start, end));
        }
    }

    pub(super) fn deselect(&self) {
        if let Some(selected) = self.selected.upgrade()
            && let Ok(mut selected) = selected.lock()
        {
            *selected = Some(false);
        }
    }
}

fn object_in_selection(
    bounds: Bounds<Pixels>,
    start: gpui::Point<Pixels>,
    end: gpui::Point<Pixels>,
) -> bool {
    point_in_text_selection(
        bounds.origin,
        bounds.size.width,
        start,
        end,
        bounds.size.height,
    )
}

/// Atomic selection wrapper that leaves child styling and interaction to GPUI.
pub(super) struct InlineObject {
    id: ElementId,
    accessibility_label: SharedString,
    size: Size<Pixels>,
    is_image: bool,
    selected: Arc<Mutex<Option<bool>>>,
    selection_bounds: Bounds<Pixels>,
    line_bounds: Bounds<Pixels>,
    rtl: bool,
    word_source: Option<(Rc<dyn crate::text_selection::WordSelectionSource>, usize)>,
    content: AnyElement,
    content_measured: bool,
    link: Option<super::node::LinkMark>,
    link_click_handler: Option<Arc<super::text_view::LinkClickHandlerFn>>,
}

impl InlineObject {
    pub(super) fn direction(mut self, rtl: bool) -> Self {
        self.rtl = rtl;
        self
    }

    pub(super) fn word_source(
        mut self,
        source: Option<(Rc<dyn crate::text_selection::WordSelectionSource>, usize)>,
    ) -> Self {
        self.word_source = source;
        self
    }

    pub fn link(
        mut self,
        link: Option<super::node::LinkMark>,
        handler: Option<Arc<super::text_view::LinkClickHandlerFn>>,
    ) -> Self {
        self.link = link;
        self.link_click_handler = handler;
        self
    }

    pub fn new(
        id: impl Into<ElementId>,
        accessibility_label: SharedString,
        object: MeasuredInlineObject,
        selected: Arc<Mutex<Option<bool>>>,
        selection_bounds: Bounds<Pixels>,
        line_bounds: Bounds<Pixels>,
    ) -> Self {
        let size = object.metrics.size;
        let (content, content_measured) = object.element();
        Self {
            id: id.into(),
            accessibility_label,
            size,
            is_image: false,
            selected,
            selection_bounds,
            line_bounds,
            rtl: false,
            word_source: None,
            content,
            content_measured,
            link: None,
            link_click_handler: None,
        }
    }

    /// The flow has already measured images. Keep their native element without
    /// shaping placeholder text or retaining a second measured-content owner.
    pub(super) fn image(
        id: impl Into<ElementId>,
        alt: SharedString,
        content: AnyElement,
        size: Size<Pixels>,
        selected: Arc<Mutex<Option<bool>>>,
        selection_bounds: Bounds<Pixels>,
        line_bounds: Bounds<Pixels>,
    ) -> Self {
        Self {
            id: id.into(),
            accessibility_label: alt,
            size,
            is_image: true,
            selected,
            selection_bounds,
            line_bounds,
            rtl: false,
            word_source: None,
            content,
            content_measured: false,
            link: None,
            link_click_handler: None,
        }
    }
}

impl IntoElement for InlineObject {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for InlineObject {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;
    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<Role> {
        Some(Role::GenericContainer)
    }

    fn write_a11y_info(&self, node: &mut gpui::accesskit::Node) {
        node.set_role(Role::GenericContainer);
        node.set_label(self.accessibility_label.as_ref());
        node.set_read_only();
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.request_layout(
                gpui::Style {
                    size: size(self.size.width.into(), self.size.height.into()),
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        if self.content_measured {
            self.content.prepaint_at(bounds.origin, window, cx);
        } else {
            self.content.prepaint_as_root(
                bounds.origin,
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::Definite(bounds.size.height),
                ),
                window,
                cx,
            );
        }
        if let Some(view) = GlobalState::global(cx).text_view_state() {
            let state = view.read(cx);
            if state.max_lines.is_some()
                && let Ok(mut spans) = state.line_spans.lock()
            {
                spans.push(LineSpan {
                    top: bounds.top(),
                    bottom: bounds.bottom(),
                    line_height: bounds.size.height,
                });
            }
        }
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let view = GlobalState::global(cx).text_view_state();
        let (selectable, selected, project_selection) =
            view.as_ref().map_or((false, false, false), |view| {
                let state = view.read(cx);
                if !state.is_selectable() {
                    return (false, false, false);
                }
                let all_selected = state.is_all_selected(cx);
                if !all_selected && state.selection_adapter.excludes_current_block(cx) {
                    return (true, false, true);
                }
                if (state.preserve_inline_selection || state.selection_adapter.has_open_selection(cx))
                    && !all_selected
                {
                    // An unprojected programmatic range still uses the paragraph's
                    // run-boundary inference. Painting must not explicitly exclude it.
                    return (
                        true,
                        self.selected.lock().is_ok_and(|selected| *selected == Some(true)),
                        false,
                    );
                }
                let selected = all_selected
                    || state.multi_click_selection().is_some_and(|s| {
                        s.line_bounds.map_or_else(
                            || bounds.contains(&s.pos),
                            |row| row.contains(&bounds.center()),
                        )
                    })
                    || state
                        .painted_selection_points(cx)
                        .is_some_and(|(start, end)| {
                            object_in_selection(self.selection_bounds, start, end)
                        });
                (true, selected, true)
            });
        if project_selection && let Ok(mut value) = self.selected.lock() {
            *value = Some(selected);
        }
        let selection_color = selected.then(|| {
            view.as_ref().unwrap().read(cx).text_view_style.selection()
        });
        if !self.is_image && let Some(color) = selection_color {
            window.paint_quad(gpui::fill(bounds, color));
        }
        if let Some(link) = self.link.clone() {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
            let link_hitbox = hitbox.clone();
            let link_view = view.clone();
            let handler = self.link_click_handler.clone();
            window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, window, cx| {
                if !phase.bubble()
                    || !link_hitbox.is_hovered(window)
                    || link_view
                        .as_ref()
                        .is_some_and(|view| view.read(cx).has_selection(cx))
                {
                    return;
                }
                crate::TextSelection::end(window, cx);
                cx.stop_propagation();
                let click = gpui::ClickEvent::Mouse(gpui::MouseClickEvent {
                    down: MouseDownEvent {
                        button: event.button,
                        position: event.position,
                        modifiers: event.modifiers,
                        click_count: event.click_count,
                        first_mouse: false,
                    },
                    up: event.clone(),
                });
                super::text_view::handle_link_click(&handler, link.url.clone(), click, window, cx);
            });
        }
        if selectable {
            view.as_ref()
                .unwrap()
                .read(cx)
                .selection_adapter
                .register_copy_projection(
                    super::selection_adapter::CopyProjection::InlineObject(
                        InlineObjectSelectionProjection {
                            selected: Arc::downgrade(&self.selected),
                            bounds: self.selection_bounds,
                            line_bounds: self.line_bounds,
                            rtl: self.rtl,
                            word_source: self.word_source.as_ref().map(|(source, item)| {
                                (Rc::downgrade(source), *item)
                            }),
                        },
                    ),
                    cx,
                );
            if selected {
                let (start, end) = if self.rtl {
                    (self.selection_bounds.top_right(), self.selection_bounds.origin)
                } else {
                    (self.selection_bounds.origin, self.selection_bounds.top_right())
                };
                view.as_ref().unwrap().read(cx).selection_adapter
                    .register_selection_edges(
                        caret_line_box(start, self.selection_bounds.size.height),
                        caret_line_box(end, self.selection_bounds.size.height),
                    );
            }
            if self.link.is_none() {
                window.set_cursor_style(CursorStyle::IBeam, hitbox);
            }
            let visible = bounds.intersect(&window.content_mask().bounds);
            if visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO {
                view.as_ref()
                    .unwrap()
                    .read(cx)
                    .selection_adapter
                    .register_inline(vec![visible]);
            }
            let hitbox = hitbox.clone();
            let is_image = self.is_image;
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble()
                    || !hitbox.is_hovered(window)
                    || event.button != MouseButton::Left
                    || !(2..=3).contains(&event.click_count)
                {
                    return;
                }
                if is_image && event.click_count == 2 && GlobalState::is_touch_press(cx) {
                    GlobalState::suppress_text_selection(cx);
                    return;
                }
                if let Some(view) = &view {
                    // The window selects the registered unit after bubbling.
                    // Discard local and offscreen ranges before it takes over.
                    view.update(cx, |state, cx| state.clear_selection(cx));
                }
            });
        }
        self.content.paint(window, cx);
        // Opaque image pixels would cover a background selection quad.
        if self.is_image && let Some(color) = selection_color {
            window.paint_quad(gpui::fill(bounds, color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::inline::test_draw::in_prepaint;
    use super::*;

    #[test]
    fn native_padding_and_fixed_size_are_measured_without_fake_scaling() {
        let mut app = gpui::TestApp::new();
        in_prepaint(&mut app, |window, cx| {
            let native = InlineElement::new(
                div()
                    .px(px(4.))
                    .py(px(2.))
                    .child(div().w(px(40.)).h(px(10.))),
            )
            .with_baseline(px(10.));
            let measured = MeasuredInlineObject::measure(
                "badge",
                Some(native),
                &TextStyle::default(),
                window,
                cx,
            );
            assert_eq!(measured.metrics.size, size(px(48.), px(14.)));
            assert_eq!(measured.metrics.baseline, px(10.));
            let narrow = measured.fit_text(Some(px(20.)));
            assert_eq!(narrow.metrics.size, size(px(48.), px(14.)));
        });
    }

    #[test]
    fn invalid_baseline_uses_text_fallback_and_zero_width_remains_finite() {
        let mut app = gpui::TestApp::new();
        in_prepaint(&mut app, |window, cx| {
            for baseline in [px(f32::NAN), px(-1.), px(30.)] {
                let native =
                    InlineElement::new(div().w(px(20.)).h(px(20.))).with_baseline(baseline);
                let measured = MeasuredInlineObject::measure(
                    "fallback",
                    Some(native),
                    &TextStyle::default(),
                    window,
                    cx,
                );
                assert!(measured.content.is_none());
                assert!(measured.metrics.is_valid());
                let zero = measured.fit_text(Some(px(0.)));
                assert_eq!(zero.metrics.size, size(px(0.), px(0.)));
                assert_eq!(zero.metrics.baseline, px(0.));
            }
        });
    }

    #[test]
    fn atomic_wrapper_does_not_mislabel_native_controls_as_images() {
        let mut app = gpui::TestApp::new();
        let mut window = app.open_window(|_, _| gpui::Empty);
        window.update(|_, window, cx| {
            let measured =
                MeasuredInlineObject::measure("member", None, &TextStyle::default(), window, cx);
            let object = InlineObject::new(
                "member",
                "Member profile".into(),
                measured,
                Arc::default(),
                Bounds::default(),
                Bounds::default(),
            );
            let mut accessible = gpui::accesskit::Node::new(Role::Unknown);
            object.write_a11y_info(&mut accessible);
            assert_eq!(accessible.role(), Role::GenericContainer);
            assert_eq!(accessible.label(), Some("Member profile"));
        });
    }
}
