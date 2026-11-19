use crate::{
    AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Size, TransformationMatrix, Window,
};

/// Scales a subtree around its left center without changing its layout allocation.
/// Drawing, clipping, hit testing and text input share the same transformation.
pub fn scaled(child: impl IntoElement, scale: Size<f32>) -> Scaled {
    assert!(scale.width.is_finite() && scale.height.is_finite() && scale.width >= 0.0 && scale.height >= 0.0,
        "element scale must be finite and non-negative");
    Scaled { child: child.into_any_element(), scale }
}

/// An axis-scaled element. Deferred popups retain their own unscaled layout.
pub struct Scaled { child: AnyElement, scale: Size<f32> }

impl Element for Scaled {
    type RequestLayoutState = ();
    type PrepaintState = TransformationMatrix;

    fn id(&self) -> Option<crate::ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, _id: Option<&GlobalElementId>, _inspector: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(&mut self, _id: Option<&GlobalElementId>, _inspector: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _layout: &mut (), window: &mut Window, cx: &mut App) -> TransformationMatrix {
        let origin = bounds.origin;
        let center_y = bounds.center().y;
        let local = TransformationMatrix {
            rotation_scale: [[self.scale.width, 0.0], [0.0, self.scale.height]],
            translation: [origin.x.0 * (1.0 - self.scale.width), center_y.0 * (1.0 - self.scale.height)],
        };
        let transform = window.element_transformation.compose(local);
        window.with_element_transformation(transform, |window| self.child.prepaint(window, cx));
        transform
    }

    fn paint(&mut self, _id: Option<&GlobalElementId>, _inspector: Option<&InspectorElementId>, _bounds: Bounds<Pixels>, _layout: &mut (), transform: &mut TransformationMatrix, window: &mut Window, cx: &mut App) {
        window.with_element_transformation(*transform, |window| { self.child.paint(window, cx); });
    }
}

impl IntoElement for Scaled {
    type Element = Self;
    fn into_element(self) -> Self { self }
}
