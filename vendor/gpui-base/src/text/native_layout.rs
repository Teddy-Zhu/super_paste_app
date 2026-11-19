use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use gpui::{App, Bounds, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, StyledText, TextLayout, TextStyle, WeakEntity, Window};

use super::{TextViewState, TextViewStyle};

#[derive(Clone, Copy, Debug)]
pub(super) enum NativeLayoutFailure {
    UnsupportedSource,
    UnsafeClusterBoundary,
    ShapingWorkLimit,
}

/// One document/font generation. Retained nodes can report only to their
/// own generation; deferred notifications never mutate a replacement session.
#[derive(Clone)]
pub(super) struct NativeLayoutGuard {
    inner: Arc<NativeLayoutState>,
}

struct NativeLayoutState {
    failed: AtomicBool,
    owner: WeakEntity<TextViewState>,
}

impl PartialEq for NativeLayoutGuard {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.inner, &other.inner) }
}

impl NativeLayoutGuard {
    pub fn failed(&self) -> bool { self.inner.failed.load(Ordering::Acquire) }

    pub fn paint_foreground(&self, layout: &TextLayout, window: &mut Window, cx: &mut App) -> bool {
        if self.failed() { return false; }
        if let Err(error) = layout.try_paint_foreground(window, cx) {
            if !self.failed() {
                tracing::warn!(%error, "native document text painting failed");
            }
            // Retained elements report only to their own document generation.
            self.fail(cx);
            return false;
        }
        true
    }

    pub fn fail(&self, cx: &mut App) {
        if !self.inner.failed.swap(true, Ordering::AcqRel) {
            let owner = self.inner.owner.clone();
            // Intrinsic layout can run while the owning view is borrowed.
            cx.defer(move |cx| { let _ = owner.update(cx, |_, cx| cx.notify()); });
        }
    }
}

/// Authored list markers share the document's failure feedback, including
/// empty list items with no body flow to report a drawing failure.
pub(super) struct NativeText {
    text: StyledText,
    guard: NativeLayoutGuard,
}

impl NativeText {
    pub fn new(text: SharedString, guard: NativeLayoutGuard) -> Self {
        Self { text: StyledText::new(text), guard }
    }
}

impl IntoElement for NativeText {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for NativeText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, id: Option<&GlobalElementId>, inspector: Option<&InspectorElementId>,
        window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        self.text.request_layout(id, inspector, window, cx)
    }

    fn prepaint(&mut self, id: Option<&GlobalElementId>, inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>, state: &mut (), window: &mut Window, cx: &mut App) {
        self.text.prepaint(id, inspector, bounds, state, window, cx);
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        _: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        if self.guard.failed() { return; }
        let layout = self.text.layout();
        _ = layout.paint_background(window, cx);
        self.guard.paint_foreground(layout, window, cx);
    }
}

#[derive(PartialEq)]
pub(super) struct NativeLayoutKey {
    pub revision: usize,
    pub width: Pixels,
    pub typography: (TextStyle, Pixels),
    pub font_generation: usize,
    pub style: Arc<TextViewStyle>,
    pub mono_family: SharedString,
}

#[derive(Default)]
pub(super) struct NativeLayoutSession {
    current: Option<(NativeLayoutKey, NativeLayoutGuard)>,
    failure_presented: bool,
}

impl NativeLayoutSession {
    pub fn prepare(&mut self, key: NativeLayoutKey, owner: &Entity<TextViewState>) -> NativeLayoutGuard {
        if self.current.as_ref().is_none_or(|(previous, _)| *previous != key) {
            self.current = Some((key, NativeLayoutGuard {
                inner: Arc::new(NativeLayoutState {
                    failed: AtomicBool::new(false), owner: owner.downgrade(),
                }),
            }));
            self.failure_presented = false;
        }
        self.current.as_ref().expect("prepared native layout session").1.clone()
    }

    pub fn retry_failure(&mut self) {
        if self.current.as_ref().is_some_and(|(_, guard)| guard.failed()) {
            *self = Self::default();
        }
    }

    pub fn take_new_failure(&mut self) -> bool {
        if !self.failure_presented && self.current.as_ref().is_some_and(|(_, guard)| guard.failed()) {
            self.failure_presented = true;
            true
        } else { false }
    }
}
