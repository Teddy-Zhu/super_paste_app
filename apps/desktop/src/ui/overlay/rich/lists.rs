use gpui_kit::SharedString;
use super_paste_core::preview::rich::{ListStyle, MarkerText};

/// Counter semantics live in the controlled-document core. Shared literal
/// markers cross the UI boundary without a per-item string allocation.
pub(super) fn label(style: &ListStyle, ordinal: i64, rtl: bool) -> SharedString {
    match style.marker(ordinal, rtl) {
        MarkerText::Static(value) => value.into(),
        MarkerText::Shared(value) => value.into(),
        MarkerText::Generated(value) => value.into(),
    }
}
