use crate::{ActiveTheme, Disableable, StyledExt, h_flex};
use gpui::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Hsla, StyleRefinement, Styled, Window, accesskit::Toggled, prelude::FluentBuilder as _,
};
use gpui_base::{PremultipliedColor, TestSupportExt as _};
use smallvec::SmallVec;
use std::{rc::Rc, time::Duration};

/// Optional host appearance. Default Kit menus retain their theme and behavior.
#[derive(Clone)]
pub struct MenuItemAppearance {
    pub style: StyleRefinement,
    pub foreground: Hsla,
    pub hovered_foreground: Hsla,
    pub disabled_foreground: Hsla,
    pub background: Hsla,
    pub hovered_background: Hsla,
    pub pressed_background: Hsla,
    pub background_transition: Duration,
    pub foreground_transition: Duration,
    pub border_transition: Duration,
    pub press_offset: Option<(gpui::Pixels, Duration)>,
}

#[derive(Clone, Copy, Default)]
struct Interaction { hovered: bool, pressed: bool }

#[derive(IntoElement)]
pub(crate) struct MenuItemElement {
    id: ElementId,
    aria_label: Option<SharedString>,
    style: StyleRefinement,
    disabled: bool,
    selected: bool,
    checked: Option<bool>,
    appearance: Option<Rc<MenuItemAppearance>>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_hover: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
    children: SmallVec<[AnyElement; 2]>,
}

impl MenuItemElement {
    /// Create a new MenuItem with the given ID.
    pub(crate) fn new(id: impl Into<ElementId>) -> Self {
        let id: ElementId = id.into();
        Self {
            id: id.clone(),
            aria_label: None,
            style: StyleRefinement::default(),
            disabled: false,
            selected: false,
            checked: None,
            appearance: None,
            on_click: None,
            on_hover: None,
            children: SmallVec::new(),
        }
    }

    pub(crate) fn appearance(mut self, appearance: Rc<MenuItemAppearance>) -> Self {
        self.appearance = Some(appearance); self
    }

    /// Set ListItem as the selected item style.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Set whether the menu item is checked, for assistive technology, or
    /// `None` for an item that is not an option. The check mark itself is
    /// drawn by the menu.
    pub(crate) fn checked(mut self, checked: Option<bool>) -> Self {
        self.checked = checked;
        self
    }

    /// Set the accessible label for the menu item.
    pub(crate) fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// Set the disabled state of the MenuItem.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set a handler for when the MenuItem is clicked.
    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set a handler for when the mouse enters the MenuItem.
    #[allow(unused)]
    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Box::new(handler));
        self
    }
}

impl Disableable for MenuItemElement {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for MenuItemElement {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for MenuItemElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for MenuItemElement {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Element-scoped state expires when the item is no longer rendered;
        // there is no global hover map or timer for each row.
        let custom = self.appearance.as_ref().map(|appearance| {
            window.with_id(self.id.clone(), |window| {
                let interaction = window.use_keyed_state("menu-pointer", cx, |_, _| Interaction::default());
                let current = *interaction.read(cx);
                let highlighted = if window.last_input_was_keyboard() { self.selected } else { current.hovered };
                let background = if self.disabled { appearance.background }
                    else if current.pressed && current.hovered { appearance.pressed_background }
                    else if highlighted { appearance.hovered_background } else { appearance.background };
                let background = gpui_base::transition("menu-background", PremultipliedColor(background.into()),
                    gpui_base::Transition::new(appearance.background_transition).easing(gpui_base::Easing::Ease), window, cx).0;
                let foreground = if self.disabled { appearance.disabled_foreground }
                    else if highlighted { appearance.hovered_foreground } else { appearance.foreground };
                let foreground = if appearance.foreground_transition.is_zero() { foreground.into() }
                    else { gpui_base::transition("menu-foreground", PremultipliedColor(foreground.into()),
                        gpui_base::Transition::new(appearance.foreground_transition).easing(gpui_base::Easing::Ease), window, cx).0 };
                let border = if appearance.border_transition.is_zero() { None } else {
                    appearance.style.border_color.map(|color| gpui_base::transition("menu-border", PremultipliedColor(color.into()),
                        gpui_base::Transition::new(appearance.border_transition).easing(gpui_base::Easing::Ease), window, cx).0)
                };
                let offset = appearance.press_offset.map(|(offset, duration)| gpui_base::transition("menu-press",
                    if !self.disabled && current.pressed { offset } else { gpui::px(0.) },
                    gpui_base::Transition::new(duration).easing(gpui_base::Easing::Ease), window, cx));
                (interaction, background, foreground, border, offset)
            })
        });
        let custom_appearance = self.appearance.is_some();
        let pointer = custom.as_ref().map(|(interaction, ..)| interaction.clone());
        let mut on_hover = self.on_hover;
        let custom_hover = if custom_appearance { on_hover.take() } else { None };
        h_flex()
            .id(self.id)
            .test_support()
            .role(Role::MenuItem)
            .when_some(self.aria_label, |this, label| this.aria_label(label))
            .aria_selected(self.selected)
            .when_some(self.checked, |this, checked| {
                this.aria_toggled(if checked {
                    Toggled::True
                } else {
                    Toggled::False
                })
            })
            .gap_x_1()
            .py_1()
            .px_2()
            .text_base()
            .text_color(cx.theme().foreground)
            .relative()
            .items_center()
            .justify_between()
            .refine_style(&self.style)
            .when_some(custom, |this, (interaction, background, foreground, border, offset)| {
                let appearance = self.appearance.as_ref().expect("custom menu appearance");
                let hovered = interaction.clone(); let up = interaction.clone();
                let disabled = self.disabled;
                this.refine_style(&appearance.style).bg(background).text_color(foreground)
                    .when_some(border, |this, border| this.border_color(border))
                    .when_some(offset, |this, offset| this.top(offset))
                    .hover_listener_mode(gpui::HoverListenerMode::PointerPresence)
                    .cursor(if disabled { gpui::CursorStyle::OperationNotAllowed } else { gpui::CursorStyle::PointingHand })
                    .on_hover(move |inside, window, cx| {
                        hovered.update(cx, |state, cx| {
                            if state.hovered != *inside { state.hovered = *inside; cx.notify(); }
                        });
                        if let Some(on_hover) = &custom_hover { on_hover(inside, window, cx); }
                    })
                    // Clear during capture: a submenu's click handler stops
                    // bubbling while retaining this row for the next frame.
                    .capture_any_mouse_up(move |event, _, cx| {
                        if event.button == MouseButton::Left { up.update(cx, |state, cx| {
                            if state.pressed { state.pressed = false; cx.notify(); }
                        }); }
                    })
                    .on_mouse_up_out(MouseButton::Left, move |_, _, cx| interaction.update(cx, |state, cx| {
                        if state.pressed { state.pressed = false; cx.notify(); }
                    }))
            })
            .when_some(on_hover, |this, on_hover| {
                this.on_hover(move |hovered, window, cx| (on_hover)(hovered, window, cx))
            })
            .when(!self.disabled, |this| {
                // Not `group_hover` on the item's own group: GPUI styles an
                // element before registering its group, and that fallback keeps
                // the pointer's item lit after a key moves the highlight away.
                this.when(!custom_appearance, |this| this.hover(|this| {
                    this.bg(cx.theme().tokens.accent)
                        .text_color(cx.theme().accent_foreground)
                })
                .when(self.selected, |this| {
                    this.bg(cx.theme().tokens.accent)
                        .text_color(cx.theme().accent_foreground)
                }))
                .when_some(self.on_click, |this, on_click| {
                    this.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        // The down state and propagation boundary share one
                        // listener; bubble listeners run in reverse order.
                        if let Some(pointer) = &pointer {
                            pointer.update(cx, |state, cx| { state.pressed = true; cx.notify(); });
                        }
                        cx.stop_propagation();
                    })
                    .on_click(on_click)
                })
            })
            .when(self.disabled && !custom_appearance, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .children(self.children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn aria_label_sets_accessible_name(_cx: &mut gpui::TestAppContext) {
        let item = MenuItemElement::new("open").aria_label("Open");

        assert_eq!(item.aria_label, Some("Open".into()));
    }
}
