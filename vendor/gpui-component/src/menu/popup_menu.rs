use crate::ThemeStyled as _;
use crate::actions::{Cancel, Confirm, SelectDown, SelectUp};
use crate::actions::{SelectLeft, SelectRight};
use crate::menu::menu_item::{MenuItemElement, MenuItemAppearance};
use crate::scroll::ScrollableElement;
use crate::{ActiveTheme, ElementExt, Icon, IconName, Sizable as _, StyledExt as _, h_flex, v_flex};
use crate::{Side, Size, kbd::Kbd};
use gpui::{
    Action, Anchor, AnimationExt, AnyElement, App, AppContext, Bounds, Context, DismissEvent, Edges, Entity,
    EntityId, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Pixels, Refineable, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, StyleRefinement, Task, WeakEntity, Window, anchored, deferred, div, prelude::FluentBuilder, px, rems,
};
use gpui::{ClickEvent, Half, MouseDownEvent, OwnedMenuItem, Point, Subscription};
use gpui_base::TestSupportExt as _;

use std::collections::HashSet;
use std::{rc::Rc, time::Duration};

const CONTEXT: &str = "PopupMenu";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(CONTEXT)),
    ]);
}

/// An menu item in a popup menu.
pub enum PopupMenuItem {
    /// A menu separator item.
    Separator,
    /// A non-interactive label item.
    Label(SharedString),
    /// A standard menu item.
    Item {
        icon: Option<Icon>,
        label: SharedString,
        disabled: bool,
        checked: bool,
        is_link: bool,
        action: Option<Box<dyn Action>>,
        // For link item
        handler: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
        appearance: Option<Rc<MenuItemAppearance>>,
    },
    /// A menu item with custom element render.
    ElementItem {
        icon: Option<Icon>,
        disabled: bool,
        checked: bool,
        action: Option<Box<dyn Action>>,
        render: Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>,
        handler: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
        appearance: Option<Rc<MenuItemAppearance>>,
    },
    /// A submenu item that opens another popup menu.
    Submenu {
        icon: Option<Icon>,
        label: SharedString,
        disabled: bool,
        menu: Entity<PopupMenu>,
        appearance: Option<Rc<MenuItemAppearance>>,
    },
}

/// Content-independent styling, reusable when a visible menu changes theme.
/// Geometry not present in these refinements is retained.
#[derive(Clone)]
pub struct PopupMenuAppearance {
    pub surface: StyleRefinement,
    pub separator: StyleRefinement,
    pub item: MenuItemAppearance,
    pub submenu_indicator: Option<Icon>,
}

impl FluentBuilder for PopupMenuItem {}
impl PopupMenuItem {
    /// Create a new menu item with the given label.
    #[inline]
    pub fn new(label: impl Into<SharedString>) -> Self {
        PopupMenuItem::Item {
            icon: None,
            label: label.into(),
            disabled: false,
            checked: false,
            action: None,
            is_link: false,
            handler: None,
            appearance: None,
        }
    }

    /// Create a new menu item with custom element render.
    #[inline]
    pub fn element<F, E>(builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        PopupMenuItem::ElementItem {
            icon: None,
            disabled: false,
            checked: false,
            action: None,
            render: Box::new(move |window, cx| builder(window, cx).into_any_element()),
            handler: None,
            appearance: None,
        }
    }

    /// Create a new submenu item that opens another popup menu.
    #[inline]
    pub fn submenu(label: impl Into<SharedString>, menu: Entity<PopupMenu>) -> Self {
        PopupMenuItem::Submenu {
            icon: None,
            label: label.into(),
            disabled: false,
            menu,
            appearance: None,
        }
    }

    /// Create a separator menu item.
    #[inline]
    pub fn separator() -> Self {
        PopupMenuItem::Separator
    }

    /// Creates a label menu item.
    #[inline]
    pub fn label(label: impl Into<SharedString>) -> Self {
        PopupMenuItem::Label(label.into())
    }

    /// Set the icon for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`], [`PopupMenuItem::ElementItem`] and [`PopupMenuItem::Submenu`].
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        match &mut self {
            PopupMenuItem::Item { icon: i, .. } => {
                *i = Some(icon.into());
            }
            PopupMenuItem::ElementItem { icon: i, .. } => {
                *i = Some(icon.into());
            }
            PopupMenuItem::Submenu { icon: i, .. } => {
                *i = Some(icon.into());
            }
            _ => {}
        }
        self
    }

    /// Override one command's appearance, such as a destructive toolbar item.
    pub fn appearance(mut self, value: MenuItemAppearance) -> Self {
        match &mut self {
            Self::Item { appearance, .. } | Self::ElementItem { appearance, .. } | Self::Submenu { appearance, .. } => *appearance = Some(Rc::new(value)),
            _ => {},
        }
        self
    }

    fn appearance_override(&self) -> Option<&Rc<MenuItemAppearance>> {
        match self {
            Self::Item { appearance, .. } | Self::ElementItem { appearance, .. } | Self::Submenu { appearance, .. } => appearance.as_ref(),
            _ => None,
        }
    }

    /// Set the action for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`] and [`PopupMenuItem::ElementItem`].
    pub fn action(mut self, action: Box<dyn Action>) -> Self {
        match &mut self {
            PopupMenuItem::Item { action: a, .. } => {
                *a = Some(action);
            }
            PopupMenuItem::ElementItem { action: a, .. } => {
                *a = Some(action);
            }
            _ => {}
        }
        self
    }

    /// Set the disabled state for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`], [`PopupMenuItem::ElementItem`] and [`PopupMenuItem::Submenu`].
    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            PopupMenuItem::Item { disabled: d, .. } => {
                *d = disabled;
            }
            PopupMenuItem::ElementItem { disabled: d, .. } => {
                *d = disabled;
            }
            PopupMenuItem::Submenu { disabled: d, .. } => {
                *d = disabled;
            }
            _ => {}
        }
        self
    }

    /// Set checked state for the menu item.
    ///
    /// NOTE: If `check_side` is [`Side::Left`], the icon will replace with a check icon.
    pub fn checked(mut self, checked: bool) -> Self {
        match &mut self {
            PopupMenuItem::Item { checked: c, .. } => {
                *c = checked;
            }
            PopupMenuItem::ElementItem { checked: c, .. } => {
                *c = checked;
            }
            _ => {}
        }
        self
    }

    /// Add a click handler for the menu item.
    ///
    /// Only works for [`PopupMenuItem::Item`] and [`PopupMenuItem::ElementItem`].
    pub fn on_click<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        match &mut self {
            PopupMenuItem::Item { handler: h, .. } => {
                *h = Some(Rc::new(handler));
            }
            PopupMenuItem::ElementItem { handler: h, .. } => {
                *h = Some(Rc::new(handler));
            }
            _ => {}
        }
        self
    }

    /// Create a link menu item.
    #[inline]
    pub fn link(label: impl Into<SharedString>, href: impl Into<String>) -> Self {
        let href = href.into();
        PopupMenuItem::Item {
            icon: None,
            label: label.into(),
            disabled: false,
            checked: false,
            action: None,
            is_link: true,
            handler: Some(Rc::new(move |_, _, cx| cx.open_url(&href))),
            appearance: None,
        }
    }

    #[inline]
    fn is_clickable(&self) -> bool {
        !matches!(self, PopupMenuItem::Separator)
            && matches!(
                self,
                PopupMenuItem::Item {
                    disabled: false,
                    ..
                } | PopupMenuItem::ElementItem {
                    disabled: false,
                    ..
                } | PopupMenuItem::Submenu {
                    disabled: false,
                    ..
                }
            )
    }

    #[inline]
    fn is_separator(&self) -> bool {
        matches!(self, PopupMenuItem::Separator)
    }

    fn has_left_icon(&self, check_side: Side) -> bool {
        match self {
            PopupMenuItem::Item { icon, checked, .. } => {
                icon.is_some() || (check_side.is_left() && *checked)
            }
            PopupMenuItem::ElementItem { icon, checked, .. } => {
                icon.is_some() || (check_side.is_left() && *checked)
            }
            PopupMenuItem::Submenu { icon, .. } => icon.is_some(),
            _ => false,
        }
    }

    #[inline]
    fn is_checked(&self) -> bool {
        match self {
            PopupMenuItem::Item { checked, .. } => *checked,
            PopupMenuItem::ElementItem { checked, .. } => *checked,
            _ => false,
        }
    }

    fn a11y_label(&self) -> Option<SharedString> {
        match self {
            PopupMenuItem::Item { label, .. }
            | PopupMenuItem::Label(label)
            | PopupMenuItem::Submenu { label, .. } => Some(label.clone()),
            PopupMenuItem::Separator | PopupMenuItem::ElementItem { .. } => None,
        }
    }
}

pub struct PopupMenu {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) menu_items: Vec<PopupMenuItem>,
    /// The focus handle of Entity to handle actions.
    pub(crate) action_context: Option<FocusHandle>,
    /// The focus to restore on dismiss. Unlike `action_context`, this does not
    /// change where actions are dispatched: they still bubble from the menu's
    /// own focus path (through the trigger element's ancestors).
    pub(crate) previous_focus_handle: Option<FocusHandle>,
    /// A focus handle on the trigger's dispatch path, so shortcut hints can
    /// resolve against the key contexts the menu's actions bubble through on
    /// the very frame the menu opens. GPUI looks a handle up in the previously
    /// rendered frame, and the trigger was in it when the menu was not yet.
    pub(crate) trigger_focus_handle: Option<FocusHandle>,
    selected_index: Option<usize>,
    min_width: Option<Pixels>,
    max_width: Option<Pixels>,
    max_height: Option<Pixels>,
    bounds: Bounds<Pixels>,
    size: Size,
    item_height: Option<Pixels>,
    item_gap: Option<Pixels>,
    item_font_size: Option<Pixels>,
    icon_size: Option<Pixels>,
    icon_gap: Option<Pixels>,
    surface_style: StyleRefinement,
    separator_style: StyleRefinement,
    item_appearance: Option<Rc<MenuItemAppearance>>,
    content_padding: Option<Pixels>,
    submenu_indicator: Option<Icon>,
    pointer_submenus: Option<PointerSubmenuPlacement>,
    surface_bounds: Bounds<Pixels>,
    submenu_pointer_y: Option<Pixels>,
    entrance: Option<(Duration, Point<Pixels>)>,
    pointer_leave_delay: Option<Duration>,
    pointer_owner: Option<EntityId>,
    pointer_close: Option<Task<()>>,
    check_side: Side,
    /// Indexes of the items added as options that can be checked, so an
    /// unchecked one reports its state rather than reading as a command.
    checkable: HashSet<usize>,

    /// The parent menu of this menu, if this is a submenu
    parent_menu: Option<WeakEntity<Self>>,
    scrollable: bool,
    external_link_icon: bool,
    scroll_handle: ScrollHandle,
    // This will update on render
    submenu_anchor: (Anchor, Pixels),

    /// Paint priority for this menu layer. The top-level menu starts at 1 and
    /// each nested submenu increments it, so deeper levels are always drawn on
    /// top of shallower ones. This fixes background content (e.g. the
    /// underlying list) bleeding through multi-level submenus, which happens
    /// when nested `anchored` popovers share the same paint order.
    ///
    /// The top-level menu relies on its container (e.g. `Popover`,
    /// `ContextMenu`) to `deferred`-draw it, and each submenu is deferred once
    /// in `render_item` with `priority + 1`. Keeping a single deferred layer
    /// per level matters because GPUI caps nested deferred depth (see
    /// `prepaint_deferred_draws`).
    ///
    /// Deferring is also what lets a submenu open from a `scrollable` menu:
    /// GPUI paints a deferred draw at the window level with no inherited
    /// content mask, so the items container's `overflow_y_scroll` clip never
    /// reaches it, while the scroll offset is still baked into its anchor.
    priority: usize,

    _subscriptions: Vec<Subscription>,
}

impl PopupMenu {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            action_context: None,
            previous_focus_handle: None,
            trigger_focus_handle: None,
            parent_menu: None,
            menu_items: Vec::new(),
            selected_index: None,
            min_width: None,
            max_width: None,
            max_height: None,
            check_side: Side::Left,
            checkable: HashSet::new(),
            bounds: Bounds::default(),
            scrollable: false,
            scroll_handle: ScrollHandle::default(),
            external_link_icon: true,
            size: Size::default(),
            item_height: None,
            item_gap: None,
            item_font_size: None,
            icon_size: None,
            icon_gap: None,
            surface_style: StyleRefinement::default(),
            separator_style: StyleRefinement::default(),
            item_appearance: None,
            content_padding: None,
            submenu_indicator: None,
            pointer_submenus: None,
            surface_bounds: Bounds::default(),
            submenu_pointer_y: None,
            entrance: None,
            pointer_leave_delay: None,
            pointer_owner: None,
            pointer_close: None,
            submenu_anchor: (Anchor::TopLeft, Pixels::ZERO),
            priority: gpui_base::POPUP_PRIORITY,
            _subscriptions: vec![],
        }
    }

    pub fn build(
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(Self, &mut Window, &mut Context<PopupMenu>) -> Self,
    ) -> Entity<Self> {
        cx.new(|cx| f(Self::new(cx), window, cx))
    }

    /// Preserve native menu behavior while allowing a host's compact geometry.
    pub fn item_height(mut self, height: Pixels) -> Self {
        self.item_height = Some(height.max(px(1.)));
        self
    }

    pub fn item_gap(mut self, gap: Pixels) -> Self {
        self.item_gap = Some(gap.max(px(0.)));
        self
    }

    pub fn item_font_size(mut self, size: Pixels) -> Self {
        self.item_font_size = Some(size.max(px(1.)));
        self
    }

    pub fn icon_size(mut self, size: Pixels) -> Self {
        self.icon_size = Some(size.max(px(1.)));
        self
    }

    pub fn icon_gap(mut self, gap: Pixels) -> Self {
        self.icon_gap = Some(gap.max(px(0.)));
        self
    }

    pub fn separator_style(mut self, style: StyleRefinement) -> Self {
        self.separator_style = style;
        self
    }

    pub fn item_appearance(mut self, appearance: MenuItemAppearance) -> Self {
        self.item_appearance = Some(Rc::new(appearance)); self
    }

    pub fn appearance(mut self, appearance: PopupMenuAppearance) -> Self {
        self.apply_appearance(appearance); self
    }

    /// Change a visible menu's styling without rebuilding content, focus,
    /// selection, scroll position, entrance animation or pointer-close state.
    pub fn set_appearance(&mut self, appearance: PopupMenuAppearance, cx: &mut Context<Self>) {
        self.apply_appearance(appearance);
        cx.notify();
    }

    fn apply_appearance(&mut self, appearance: PopupMenuAppearance) {
        self.surface_style.refine(&appearance.surface);
        self.separator_style.refine(&appearance.separator);
        self.item_appearance = Some(Rc::new(appearance.item));
        self.submenu_indicator = appearance.submenu_indicator;
    }

    /// Update one authored row override while retaining its action and state.
    pub fn set_item_appearance(&mut self, index: usize, appearance: MenuItemAppearance, cx: &mut Context<Self>) {
        let Some(item) = self.menu_items.get_mut(index) else { return; };
        match item {
            PopupMenuItem::Item { appearance: value, .. }
            | PopupMenuItem::ElementItem { appearance: value, .. }
            | PopupMenuItem::Submenu { appearance: value, .. } => *value = Some(Rc::new(appearance)),
            _ => return,
        }
        cx.notify();
    }

    /// Read child identities for host-driven styling without rebuilding panes.
    pub fn submenus(&self) -> impl Iterator<Item = &Entity<PopupMenu>> {
        self.menu_items.iter().filter_map(|item| match item {
            PopupMenuItem::Submenu { menu, .. } => Some(menu),
            _ => None,
        })
    }

    pub fn content_padding(mut self, padding: Pixels) -> Self {
        self.content_padding = Some(padding.max(px(0.))); self
    }

    pub fn submenu_indicator(mut self, icon: Icon) -> Self {
        self.submenu_indicator = Some(icon); self
    }

    /// Use the menu surface edge and the pointer's entry position, instead of
    /// Kit's row-relative placement. Keyboard/click opening uses surface top.
    pub fn pointer_submenus(mut self, overlap: Pixels, viewport_margin: Pixels, pointer_offset: Pixels) -> Self {
        self.pointer_submenus = Some(PointerSubmenuPlacement { overlap, viewport_margin, pointer_offset }); self
    }

    /// Estimate a fixed-width context menu using one slot per authored item.
    /// Separators intentionally keep a slot: hosts can reproduce their source
    /// menu's positioning without retaining or measuring another layout tree.
    pub fn estimated_size(&self, window: &Window) -> gpui::Size<Pixels> {
        let border = &self.surface_style.border_widths;
        let rem = window.rem_size();
        let px = |length: Option<gpui::AbsoluteLength>| length.unwrap_or_default().to_pixels(rem);
        gpui::size(self.min_width.unwrap_or(rem * 8.).min(self.max_width()) + px(border.left) + px(border.right),
            self.item_height.unwrap_or(gpui::px(26.)) * self.menu_items.len() as f32
                + self.content_padding.unwrap_or(rem * 0.25) * 2. + px(border.top) + px(border.bottom))
    }

    pub fn entrance(mut self, duration: Duration, offset: Point<Pixels>) -> Self {
        self.entrance = Some((duration, offset));
        self
    }

    /// Set the focus handle of Entity to handle actions.
    ///
    /// When the menu is dismissed or before an action is triggered, the focus will be returned to this handle.
    ///
    /// Then the action will be dispatched to this handle.
    /// Submenus without their own action context use the nearest parent's
    /// explicit context for actions and shortcut hints.
    pub fn action_context(mut self, handle: FocusHandle) -> Self {
        self.action_context = Some(handle);
        self
    }

    pub(crate) fn set_action_context(
        &mut self,
        action_context: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        self.action_context = action_context.clone();

        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                menu.update(cx, |menu, cx| {
                    menu.set_action_context(action_context.clone(), cx);
                });
            }
        }
    }

    /// Set the focus to restore when the menu is dismissed, without changing
    /// where actions are dispatched.
    pub(crate) fn set_previous_focus(
        &mut self,
        handle: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        self.previous_focus_handle = handle.clone();

        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                menu.update(cx, |menu, cx| {
                    menu.set_previous_focus(handle.clone(), cx);
                });
            }
        }
    }

    /// Set the focus handle on the trigger's dispatch path that shortcut hints
    /// resolve against, without changing focus or where actions are dispatched.
    pub(crate) fn set_trigger_focus(
        &mut self,
        handle: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        self.trigger_focus_handle = handle.clone();

        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                menu.update(cx, |menu, cx| {
                    menu.set_trigger_focus(handle.clone(), cx);
                });
            }
        }
    }

    /// Set min width of the popup menu, default is 120px
    pub fn min_w(mut self, width: impl Into<Pixels>) -> Self {
        self.min_width = Some(width.into());
        self
    }

    /// Set max width of the popup menu, default is 500px
    pub fn max_w(mut self, width: impl Into<Pixels>) -> Self {
        self.max_width = Some(width.into());
        self
    }

    /// Set max height of the popup menu, default is half of the window height
    pub fn max_h(mut self, height: impl Into<Pixels>) -> Self {
        self.max_height = Some(height.into());
        self
    }

    /// Set the menu to be scrollable to show vertical scrollbar.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Set the side to show check icon, default is `Side::Left`.
    pub fn check_side(mut self, side: Side) -> Self {
        self.check_side = side;
        self
    }

    /// Set the menu to show external link icon, default is true.
    pub fn external_link_icon(mut self, visible: bool) -> Self {
        self.external_link_icon = visible;
        self
    }

    /// Add Menu Item
    pub fn menu(self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.menu_with_disabled(label, action, false)
    }

    /// Add Menu Item with enable state
    pub fn menu_with_enable(
        mut self,
        label: impl Into<SharedString>,
        action: Box<dyn Action>,
        enable: bool,
    ) -> Self {
        self.add_menu_item(label, None, action, !enable, false);
        self
    }

    /// Add Menu Item with disabled state
    pub fn menu_with_disabled(
        mut self,
        label: impl Into<SharedString>,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.add_menu_item(label, None, action, disabled, false);
        self
    }

    /// Add label
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.menu_items.push(PopupMenuItem::label(label.into()));
        self
    }

    /// Add Menu to open link
    pub fn link(self, label: impl Into<SharedString>, href: impl Into<String>) -> Self {
        self.link_with_disabled(label, href, false)
    }

    /// Add Menu to open link with disabled state
    pub fn link_with_disabled(
        mut self,
        label: impl Into<SharedString>,
        href: impl Into<String>,
        disabled: bool,
    ) -> Self {
        let href = href.into();
        self.menu_items
            .push(PopupMenuItem::link(label, href).disabled(disabled));
        self
    }

    /// Add Menu to open link
    pub fn link_with_icon(
        self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        href: impl Into<String>,
    ) -> Self {
        self.link_with_icon_and_disabled(label, icon, href, false)
    }

    /// Add Menu to open link with icon and disabled state
    fn link_with_icon_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        href: impl Into<String>,
        disabled: bool,
    ) -> Self {
        let href = href.into();
        self.menu_items.push(
            PopupMenuItem::link(label, href)
                .icon(icon)
                .disabled(disabled),
        );
        self
    }

    /// Add Menu Item with Icon.
    pub fn menu_with_icon(
        self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
    ) -> Self {
        self.menu_with_icon_and_disabled(label, icon, action, false)
    }

    /// Add Menu Item with Icon and disabled state
    pub fn menu_with_icon_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.add_menu_item(label, Some(icon.into()), action, disabled, false);
        self
    }

    /// Add Menu Item with check icon
    pub fn menu_with_check(
        self,
        label: impl Into<SharedString>,
        checked: bool,
        action: Box<dyn Action>,
    ) -> Self {
        self.menu_with_check_and_disabled(label, checked, action, false)
    }

    /// Add Menu Item with check icon and disabled state
    pub fn menu_with_check_and_disabled(
        mut self,
        label: impl Into<SharedString>,
        checked: bool,
        action: Box<dyn Action>,
        disabled: bool,
    ) -> Self {
        self.checkable.insert(self.menu_items.len());
        self.add_menu_item(label, None, action, disabled, checked);
        self
    }

    /// Add Menu Item with custom element render.
    pub fn menu_element<F, E>(self, action: Box<dyn Action>, builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_check(false, action, builder)
    }

    /// Add Menu Item with custom element render with disabled state.
    pub fn menu_element_with_disabled<F, E>(
        self,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_check_and_disabled(false, action, disabled, builder)
    }

    /// Add Menu Item with custom element render with icon.
    pub fn menu_element_with_icon<F, E>(
        self,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_icon_and_disabled(icon, action, false, builder)
    }

    /// Add Menu Item with custom element render with check state
    pub fn menu_element_with_check<F, E>(
        self,
        checked: bool,
        action: Box<dyn Action>,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_element_with_check_and_disabled(checked, action, false, builder)
    }

    /// Add Menu Item with custom element render with icon and disabled state
    fn menu_element_with_icon_and_disabled<F, E>(
        mut self,
        icon: impl Into<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.menu_items.push(
            PopupMenuItem::element(builder)
                .action(action)
                .icon(icon)
                .disabled(disabled),
        );
        self
    }

    /// Add Menu Item with custom element render with check state and disabled state
    fn menu_element_with_check_and_disabled<F, E>(
        mut self,
        checked: bool,
        action: Box<dyn Action>,
        disabled: bool,
        builder: F,
    ) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.checkable.insert(self.menu_items.len());
        self.menu_items.push(
            PopupMenuItem::element(builder)
                .action(action)
                .checked(checked)
                .disabled(disabled),
        );
        self
    }

    /// Add a separator Menu Item
    pub fn separator(mut self) -> Self {
        if self.menu_items.is_empty() {
            return self;
        }

        if let Some(PopupMenuItem::Separator) = self.menu_items.last() {
            return self;
        }

        self.menu_items.push(PopupMenuItem::separator());
        self
    }

    /// Add a Submenu
    pub fn submenu(
        self,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.submenu_with_icon(None, label, window, cx, f)
    }

    /// Add a Submenu item with icon
    pub fn submenu_with_icon(
        mut self,
        icon: Option<Icon>,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        let submenu = PopupMenu::build(window, cx, f);
        let parent_menu = cx.entity().downgrade();
        let parent_priority = self.priority;
        submenu.update(cx, |view, _| {
            view.parent_menu = Some(parent_menu);
            view.priority = parent_priority + 1;
        });

        self.menu_items.push(
            PopupMenuItem::submenu(label, submenu).when_some(icon, |this, icon| this.icon(icon)),
        );
        self
    }

    /// Add menu item.
    pub fn item(mut self, item: impl Into<PopupMenuItem>) -> Self {
        let item: PopupMenuItem = item.into();
        self.menu_items.push(item);
        self
    }

    /// A toggle option must expose both its checked and unchecked state,
    /// including when the host supplies its own icon and click handler.
    pub fn option(mut self, item: impl Into<PopupMenuItem>) -> Self {
        self.checkable.insert(self.menu_items.len());
        self.menu_items.push(item.into());
        self
    }

    /// Replace all menu items by re-running a builder on this menu, keeping its
    /// identity (focus, parent menu, layer priority).
    ///
    /// For menus whose content arrives asynchronously after the menu is shown,
    /// e.g. swapping a "loading…" placeholder for the loaded items:
    ///
    /// ```ignore
    /// cx.spawn_in(window, async move |menu, cx| {
    ///     let items = fetch_items().await;
    ///     _ = menu.update_in(cx, |menu, window, cx| {
    ///         menu.rebuild(window, cx, |menu, _, _| {
    ///             items.into_iter().fold(menu, |menu, item| {
    ///                 menu.menu(item.label, Box::new(item.action))
    ///             })
    ///         });
    ///     });
    /// })
    /// .detach();
    /// ```
    pub fn rebuild(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(Self, &mut Window, &mut Context<Self>) -> Self,
    ) {
        let mut menu = std::mem::replace(self, Self::new(cx));
        menu.menu_items.clear();
        menu.checkable.clear();
        menu.submenu_pointer_y = None;
        menu.selected_index = None;
        *self = f(menu, window, cx);
        cx.notify();
    }

    /// Update an asynchronous action's availability without replacing the menu,
    /// moving other rows, or discarding an open submenu and its focus.
    pub fn set_item_disabled(&mut self, index: usize, disabled: bool, cx: &mut Context<Self>) {
        let Some(item) = self.menu_items.get_mut(index) else { return; };
        let value = match item {
            PopupMenuItem::Item { disabled, .. } | PopupMenuItem::ElementItem { disabled, .. } | PopupMenuItem::Submenu { disabled, .. } => disabled,
            _ => return,
        };
        if *value == disabled { return; }
        *value = disabled;
        if disabled && self.selected_index == Some(index) { self.selected_index = None; }
        cx.notify();
    }

    /// Dismiss the complete menu chain after the pointer leaves every pane.
    /// Entering any child cancels the one root-owned timer.
    pub fn dismiss_on_pointer_leave(mut self, delay: Duration) -> Self {
        self.pointer_leave_delay = Some(delay); self
    }

    fn pointer_changed(&mut self, owner: EntityId, inside: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = &self.parent_menu {
            let _ = parent.update(cx, |parent, cx| parent.pointer_changed(owner, inside, window, cx));
            return;
        }
        let Some(delay) = self.pointer_leave_delay else { return; };
        if inside {
            self.pointer_owner = Some(owner); self.pointer_close = None;
        } else if self.pointer_owner == Some(owner) {
            self.pointer_owner = None;
            self.pointer_close = Some(cx.spawn_in(window, async move |weak, cx| {
                cx.background_executor().timer(delay).await;
                let _ = weak.update_in(cx, |menu, window, cx| {
                    menu.pointer_close = None;
                    if menu.pointer_owner.is_none() { menu.dismiss(&Cancel, window, cx); }
                });
            }));
        }
    }

    fn contains_active_pane(&self, owner: EntityId, cx: &App) -> bool {
        self.active_submenu().is_some_and(|menu| {
            menu.entity_id() == owner || menu.read(cx).contains_active_pane(owner, cx)
        })
    }

    fn add_menu_item(
        &mut self,
        label: impl Into<SharedString>,
        icon: Option<Icon>,
        action: Box<dyn Action>,
        disabled: bool,
        checked: bool,
    ) -> &mut Self {
        self.menu_items.push(
            PopupMenuItem::new(label)
                .when_some(icon, |item, icon| item.icon(icon))
                .disabled(disabled)
                .checked(checked)
                .action(action),
        );
        self
    }

    pub(super) fn with_menu_items<I>(
        mut self,
        items: impl IntoIterator<Item = I>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        I: Into<OwnedMenuItem>,
    {
        for item in items {
            match item.into() {
                OwnedMenuItem::Action {
                    name,
                    action,
                    checked,
                    disabled,
                    ..
                } => {
                    // Every application menu action has a check state, so an
                    // unchecked one is not known to be an option.
                    self = self.item(
                        PopupMenuItem::new(name)
                            .action(action.boxed_clone())
                            .disabled(disabled)
                            .checked(checked),
                    )
                }
                OwnedMenuItem::Separator => {
                    self = self.separator();
                }
                OwnedMenuItem::Submenu(submenu) => {
                    self = self.submenu(submenu.name, window, cx, move |menu, window, cx| {
                        menu.with_menu_items(submenu.items.clone(), window, cx)
                    })
                }
                OwnedMenuItem::SystemMenu(_) => {}
            }
        }

        if self.menu_items.len() > 20 {
            self.scrollable = true;
        }

        self
    }

    pub(crate) fn active_submenu(&self) -> Option<Entity<PopupMenu>> {
        if let Some(ix) = self.selected_index {
            if let Some(item) = self.menu_items.get(ix) {
                return match item {
                    PopupMenuItem::Submenu { menu, disabled: false, .. } => Some(menu.clone()),
                    _ => None,
                };
            }
        }

        None
    }

    pub fn is_empty(&self) -> bool {
        self.menu_items.is_empty()
    }

    fn clickable_menu_items(&self) -> impl Iterator<Item = (usize, &PopupMenuItem)> {
        self.menu_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_clickable())
    }

    fn on_click(
        &mut self,
        ix: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.prevent_default();
        self.selected_index = Some(ix);
        self.submenu_pointer_y = None;
        self.activate(event, window, cx);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(&ClickEvent::default(), window, cx);
    }

    fn activate(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_index {
            Some(index) => {
                let item = self.menu_items.get(index);
                if !item.is_some_and(PopupMenuItem::is_clickable) {
                    return;
                }
                match item {
                    Some(PopupMenuItem::Item {
                        handler, action, ..
                    }) => {
                        if let Some(handler) = handler {
                            handler(event, window, cx);
                        } else if let Some(action) = action.as_ref() {
                            self.dispatch_confirm_action(action, window, cx);
                        }

                        self.dismiss(&Cancel, window, cx)
                    }
                    Some(PopupMenuItem::ElementItem {
                        handler, action, ..
                    }) => {
                        if let Some(handler) = handler {
                            handler(event, window, cx);
                        } else if let Some(action) = action.as_ref() {
                            self.dispatch_confirm_action(action, window, cx);
                        }
                        self.dismiss(&Cancel, window, cx)
                    }
                    Some(PopupMenuItem::Submenu {
                        disabled: false, ..
                    }) => {
                        self._select_submenu(window, cx);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn dispatch_confirm_action(
        &self,
        action: &Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(context) = self.resolved_action_context(cx) {
            context.focus(window, cx);
        }

        window.dispatch_action(action.boxed_clone(), cx);
    }

    fn resolved_action_context(&self, cx: &App) -> Option<FocusHandle> {
        self.action_context.clone().or_else(|| {
            let parent = self.parent_menu.as_ref()?.upgrade()?;
            parent.read(cx).resolved_action_context(cx)
        })
    }

    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let placement_changed = self.submenu_pointer_y.take().is_some();
        if self.selected_index != Some(ix) {
            self.selected_index = Some(ix);
            self.scroll_handle.scroll_to_item(ix);
            cx.notify();
        } else if placement_changed {
            cx.notify();
        }
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let ix = self.selected_index.unwrap_or(0);

        if let Some((prev_ix, _)) = self
            .menu_items
            .iter()
            .enumerate()
            .rev()
            .find(|(i, item)| *i < ix && item.is_clickable())
        {
            self.set_selected_index(prev_ix, cx);
            return;
        }

        let last_clickable_ix = self.clickable_menu_items().last().map(|(ix, _)| ix);
        if let Some(ix) = last_clickable_ix {
            self.set_selected_index(ix, cx);
        }
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(ix) = self.selected_index else {
            let first_clickable_ix = self.clickable_menu_items().next().map(|(ix, _)| ix);
            if let Some(ix) = first_clickable_ix {
                self.set_selected_index(ix, cx);
            }
            return;
        };

        if let Some((next_ix, _)) = self
            .menu_items
            .iter()
            .enumerate()
            .find(|(i, item)| *i > ix && item.is_clickable())
        {
            self.set_selected_index(next_ix, cx);
            return;
        }

        let first_clickable_ix = self.clickable_menu_items().next().map(|(ix, _)| ix);
        if let Some(ix) = first_clickable_ix {
            self.set_selected_index(ix, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        let handled = if matches!(self.submenu_anchor.0, Anchor::TopLeft | Anchor::BottomLeft) {
            self._unselect_submenu(window, cx)
        } else {
            self._select_submenu(window, cx)
        };

        if self.parent_side(cx).is_left() {
            self._focus_parent_menu(window, cx);
        }

        if handled {
            return;
        }

        // For parent AppMenuBar to handle.
        if self.parent_menu.is_none() {
            cx.propagate();
        }
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        let handled = if matches!(self.submenu_anchor.0, Anchor::TopLeft | Anchor::BottomLeft) {
            self._select_submenu(window, cx)
        } else {
            self._unselect_submenu(window, cx)
        };

        if self.parent_side(cx).is_right() {
            self._focus_parent_menu(window, cx);
        }

        if handled {
            return;
        }

        // For parent AppMenuBar to handle.
        if self.parent_menu.is_none() {
            cx.propagate();
        }
    }

    fn _select_submenu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.submenu_pointer_y = None;
        if let Some(active_submenu) = self.active_submenu() {
            // Focus the submenu, so that can be handle the action.
            active_submenu.update(cx, |view, cx| {
                let first_clickable_ix = view.clickable_menu_items().next().map(|(ix, _)| ix);
                if let Some(ix) = first_clickable_ix {
                    view.set_selected_index(ix, cx);
                } else {
                    view.selected_index = None;
                    cx.notify();
                }
                view.focus_handle.focus(window, cx);
            });
            cx.notify();
            return true;
        }

        return false;
    }

    fn _unselect_submenu(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(active_submenu) = self.active_submenu() {
            active_submenu.update(cx, |view, cx| {
                view.selected_index = None;
                cx.notify();
            });
            return true;
        }

        return false;
    }

    fn _focus_parent_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(parent) = self.parent_menu.as_ref() else {
            return;
        };
        let Some(parent) = parent.upgrade() else {
            return;
        };

        self.selected_index = None;
        parent.update(cx, |view, cx| {
            view.focus_handle.focus(window, cx);
            cx.notify();
        });
    }

    fn parent_side(&self, cx: &App) -> Side {
        let Some(parent) = self.parent_menu.as_ref() else {
            return Side::Left;
        };

        let Some(parent) = parent.upgrade() else {
            return Side::Left;
        };

        match parent.read(cx).submenu_anchor.0 {
            Anchor::TopLeft | Anchor::BottomLeft => Side::Left,
            Anchor::TopRight | Anchor::BottomRight => Side::Right,
            // Center anchors are not used for submenu positioning, but we must cover them.
            _ => Side::Left,
        }
    }

    /// Dismiss the menu and the entire parent chain.
    ///
    /// The submenu is closed together with its parent, same as macOS menus.
    fn dismiss(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.pointer_close = None;
        self.pointer_owner = None;
        self.selected_index = None;
        cx.emit(DismissEvent);

        // Focus back to the previous focused handle, unless the item's click
        // handler has already moved focus elsewhere (e.g. opened a dialog and
        // focused its input) -- stealing focus back would break that.
        let focus_moved_away =
            window.focused(cx).is_some() && !self.focus_handle.contains_focused(window, cx);
        if !focus_moved_away {
            if let Some(handle) = self
                .previous_focus_handle
                .as_ref()
                .or(self.action_context.as_ref())
            {
                window.focus(handle, cx);
            }
        }

        let Some(parent_menu) = self.parent_menu.clone() else {
            return;
        };

        // Dismiss parent menu, when this menu is dismissed
        _ = parent_menu.update(cx, |view, cx| {
            view.dismiss(&Cancel, window, cx);
        });
    }

    fn handle_dismiss(
        &mut self,
        position: &Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Do not dismiss, if click inside the parent menu
        if let Some(parent) = self.parent_menu.as_ref() {
            if let Some(parent) = parent.upgrade() {
                let parent = parent.read(cx);
                let bounds = if parent.pointer_submenus.is_some() { parent.surface_bounds } else { parent.bounds };
                if bounds.contains(position) {
                    return;
                }
            }
        }

        // Do not dismiss, if there have an active submenu, the click may be
        // inside the submenu, let the submenu to handle it.
        //
        // Otherwise the submenu will be dismissed before its item's `on_click`.
        if self.active_submenu().is_some() {
            return;
        }

        self.dismiss(&Cancel, window, cx);
    }

    fn on_mouse_down_out(
        &mut self,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_dismiss(&e.position, window, cx);
    }

    fn render_key_binding(
        &self,
        action: Option<Box<dyn Action>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Kbd> {
        let action = action?;

        // Each handle names a dispatch path GPUI can resolve in the previously
        // rendered frame: the explicit action target, the trigger the menu
        // opened from (already rendered when the menu first draws), the focus
        // the menu interrupted, then the menu's own path once it has been
        // rendered. A binding registered without a key context applies on
        // every path, so it is the last resort rather than the window's
        // leftover context stack, which only holds the path of whatever
        // element happened to paint last.
        let action_context = self.resolved_action_context(cx);
        [
            action_context.as_ref(),
            self.trigger_focus_handle.as_ref(),
            self.previous_focus_handle.as_ref(),
            Some(&self.focus_handle),
        ]
        .into_iter()
        .flatten()
        .find_map(|handle| Kbd::binding_for_action_in(action.as_ref(), handle, window))
        .or_else(|| Kbd::global_binding_for_action(action.as_ref(), window))
        .map(|this| {
            this.p_0()
                .flex_nowrap()
                .border_0()
                .bg(gpui::transparent_white())
        })
    }

    fn render_icon(
        &self,
        has_icon: bool,
        checked: bool,
        icon: Option<Icon>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        if !has_icon {
            return None;
        }

        let icon = if let Some(icon) = icon {
            icon.clone()
        } else if checked {
            Icon::new(IconName::Check)
        } else {
            Icon::empty()
        };

        Some(match self.icon_size { Some(size) => icon.with_size(size), None => icon.xsmall() })
    }

    #[inline]
    fn max_width(&self) -> Pixels {
        self.max_width.unwrap_or(px(500.))
    }

    /// Calculate the anchor corner and left offset for child submenu
    fn update_submenu_menu_anchor(&mut self, window: &Window, cx: &App) {
        let bounds = self.bounds;
        // Open the submenu on the left only when, placed at this menu's right
        // edge, it would run past the window's. Its width is the one it was
        // last drawn at, or this menu's own until it has been drawn.
        let submenu_width = self
            .selected_index
            .and_then(|ix| match self.menu_items.get(ix) {
                Some(PopupMenuItem::Submenu { menu, .. }) => Some(menu.read(cx).bounds.size.width),
                _ => None,
            })
            .filter(|width| *width > Pixels::ZERO)
            .unwrap_or(bounds.size.width);
        let (anchor, left) = if bounds.right() + submenu_width > window.viewport_size().width {
            (Anchor::TopRight, -px(16.))
        } else {
            (Anchor::TopLeft, bounds.size.width - px(8.))
        };

        let is_bottom_pos = bounds.origin.y + bounds.size.height > window.bounds().size.height;
        self.submenu_anchor = if is_bottom_pos {
            (anchor.other_side_along(gpui::Axis::Vertical), left)
        } else {
            (anchor, left)
        };
    }

    fn render_item(
        &self,
        ix: usize,
        item: &PopupMenuItem,
        options: RenderOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MenuItemElement {
        let has_left_icon = options.has_left_icon;
        let is_left_check = options.check_side.is_left() && item.is_checked();
        let right_check_icon = if options.check_side.is_right() && item.is_checked() {
            Some(Icon::new(IconName::Check).xsmall())
        } else {
            None
        };

        let selected = self.selected_index == Some(ix);
        const EDGE_PADDING: Pixels = px(4.);
        const INNER_PADDING: Pixels = px(8.);

        let is_submenu = matches!(item, PopupMenuItem::Submenu { .. });
        let appearance = item.appearance_override().or(self.item_appearance.as_ref()).filter(|_| !item.is_separator());
        let custom_appearance = appearance.is_some();

        let (item_height, radius) = match self.size {
            Size::Small => (px(20.), options.radius.half()),
            _ => (px(26.), options.radius),
        };
        let item_height = self.item_height.unwrap_or(item_height);

        let this = MenuItemElement::new(ix)
            .relative()
            .text_sm()
            .when_some(self.item_font_size, |item, size| item.text_size(size))
            .py_0()
            .px(INNER_PADDING)
            .rounded(radius)
            .items_center()
            .selected(selected)
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                if *hovered {
                    this.submenu_pointer_y = (is_submenu && !window.last_input_was_keyboard()).then(|| window.mouse_position().y);
                    // Pointing elsewhere closes the open submenu; take back
                    // the focus the keyboard may have moved into it.
                    if this.selected_index != Some(ix)
                        && this.active_submenu().is_some_and(|submenu| {
                            submenu.focus_handle(cx).contains_focused(window, cx)
                        })
                    {
                        this.focus_handle.focus(window, cx);
                    }
                    this.selected_index = Some(ix);
                } else if !is_submenu
                    && this.selected_index == Some(ix)
                    // A key press ends hover under a still pointer; keep the
                    // highlight so the next arrow key moves on from this item.
                    && !window.last_input_was_keyboard()
                {
                    // TODO: Better handle the submenu unselection when hover out
                    this.selected_index = None;
                }

                cx.notify();
            }))
            .when_some(item.a11y_label(), |this, label| this.aria_label(label))
            .checked(
                (item.is_checked() || self.checkable.contains(&ix)).then(|| item.is_checked()),
            )
            .when_some(appearance, |this, appearance| this.appearance(appearance.clone()));

        match item {
            PopupMenuItem::Separator => this
                .h_auto()
                .p_0()
                .my_0p5()
                .mx_neg_1()
                .border_b(px(2.))
                .border_color(cx.theme().border)
                .disabled(true)
                .refine_style(&self.separator_style),
            PopupMenuItem::Label(label) => this.disabled(true).cursor_default().child(
                h_flex()
                    .cursor_default()
                    .items_center()
                    .gap_x(self.icon_gap.unwrap_or(px(4.)))
                    .children(self.render_icon(has_left_icon, false, None, window, cx))
                    .child(div().flex_1().child(label.clone())),
            ),
            PopupMenuItem::ElementItem {
                render,
                icon,
                disabled,
                ..
            } => this
                .when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, event, window, cx| {
                        this.on_click(ix, event, window, cx)
                    }))
                })
                .disabled(*disabled)
                .child(
                    h_flex()
                        .flex_1()
                        .min_h(if custom_appearance { px(0.) } else { item_height })
                        .items_center()
                        .gap_x(self.icon_gap.unwrap_or(px(4.)))
                        .children(self.render_icon(
                            has_left_icon,
                            is_left_check,
                            icon.clone(),
                            window,
                            cx,
                        ))
                        .child((render)(window, cx))
                        .children(right_check_icon.map(|icon| icon.ml_3())),
                ),
            PopupMenuItem::Item {
                icon,
                label,
                action,
                disabled,
                is_link,
                ..
            } => {
                let show_link_icon = *is_link && self.external_link_icon;
                let action = action.as_ref().map(|action| action.boxed_clone());
                let key = self.render_key_binding(action, window, cx);

                this.when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, event, window, cx| {
                        this.on_click(ix, event, window, cx)
                    }))
                })
                .disabled(*disabled)
                .h(item_height)
                .gap_x(self.icon_gap.unwrap_or(px(4.)))
                .children(self.render_icon(
                    has_left_icon,
                    is_left_check,
                    icon.clone(),
                    window,
                    cx,
                ))
                .child(
                    h_flex()
                        .when(custom_appearance, |this| this.flex_1().min_w_0().overflow_hidden())
                        .when(!custom_appearance, |this| this.w_full())
                        .gap_3()
                        .items_center()
                        .justify_between()
                        .when(!show_link_icon, |this| this.child(Self::render_item_label(label, custom_appearance)))
                        .children(right_check_icon)
                        .when(show_link_icon, |this| {
                            this.child(
                                h_flex()
                                    .when(custom_appearance, |this| this.flex_1().min_w_0())
                                    .when(!custom_appearance, |this| this.w_full())
                                    .justify_between()
                                    .gap_1p5()
                                    .child(Self::render_item_label(label, custom_appearance))
                                    .child(
                                        Icon::new(IconName::ExternalLink)
                                            .xsmall()
                                            .text_color(cx.theme().muted_foreground),
                                    ),
                            )
                        })
                        .children(key),
                )
            }
            PopupMenuItem::Submenu {
                icon,
                label,
                menu,
                disabled,
                ..
            } => this
                .when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, event, window, cx| {
                        this.on_click(ix, event, window, cx)
                    }))
                })
                .selected(selected)
                .disabled(*disabled)
                .items_start()
                .when(custom_appearance, |this| this.items_center())
                .child(
                    h_flex()
                        .min_h(if custom_appearance { px(0.) } else { item_height })
                        .w_full()
                        .when(custom_appearance, |this| this.min_w_0())
                        .when(!custom_appearance, |this| this.h_full())
                        .items_center()
                        .gap_x(self.icon_gap.unwrap_or(px(4.)))
                        .children(self.render_icon(
                            has_left_icon,
                            false,
                            icon.clone(),
                            window,
                            cx,
                        ))
                        .child(
                            h_flex()
                                .flex_1()
                                .when(custom_appearance, |this| this.min_w_0())
                                .gap_2()
                                .when(self.submenu_indicator.is_some(), |this| this.gap_x(px(4.)))
                                .items_center()
                                .justify_between()
                                .child(div().flex_1().when(custom_appearance, |this| this.min_w_0().text_ellipsis()).child(label.clone()))
                                .child(match &self.submenu_indicator {
                                    Some(icon) => div().size(px(12.)).flex_shrink_0().flex().items_center().justify_center().child(icon.clone()).into_any_element(),
                                    None => Icon::new(IconName::ChevronRight).xsmall().text_color(cx.theme().muted_foreground).into_any_element(),
                                }),
                        ),
                )
                .when(selected && item.is_clickable(), |this| {
                    this.child({
                        if let Some(placement) = self.pointer_submenus {
                            let size = menu.read(cx).estimated_size(window);
                            let bounds = self.surface_bounds;
                            let viewport = window.viewport_size();
                            let mut x = bounds.right() - placement.overlap;
                            if x + size.width > viewport.width - placement.viewport_margin {
                                x = bounds.left() - size.width + placement.overlap;
                            }
                            let y = self.submenu_pointer_y.map_or(bounds.top(), |y| y - placement.pointer_offset)
                                .min(viewport.height - size.height - placement.viewport_margin);
                            deferred(anchored().position(gpui::point(x, y)).anchor(Anchor::TopLeft).keep_position()
                                .child(div().id("submenu").test_support().occlude().child(menu.clone())))
                                .with_priority(self.priority + 1).into_any_element()
                        } else {
                            let (anchor, left) = self.submenu_anchor;
                            let is_bottom_pos =
                                matches!(anchor, Anchor::BottomLeft | Anchor::BottomRight);
                            deferred(
                                anchored()
                                    .anchor(anchor)
                                    .child(
                                        div()
                                            .id("submenu")
                                            .test_support()
                                            .occlude()
                                            .when(is_bottom_pos, |this| this.bottom_0())
                                            .when(!is_bottom_pos, |this| this.top_neg_1())
                                            .left(left)
                                            .child(menu.clone()),
                                    )
                                    .snap_to_window_with_margin(Edges::all(EDGE_PADDING)),
                            )
                            .with_priority(self.priority + 1).into_any_element()
                        }
                    })
                }),
        }
    }

    fn render_item_label(label: &SharedString, truncate: bool) -> AnyElement {
        if truncate { div().flex_1().min_w_0().text_ellipsis().child(label.clone()).into_any_element() }
        else { label.clone().into_any_element() }
    }
}

impl FluentBuilder for PopupMenu {}

impl Styled for PopupMenu {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.surface_style
    }
}
impl EventEmitter<DismissEvent> for PopupMenu {}
impl Focusable for PopupMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[derive(Clone, Copy)]
struct RenderOptions {
    has_left_icon: bool,
    check_side: Side,
    radius: Pixels,
}

#[derive(Clone, Copy)]
struct PointerSubmenuPlacement { overlap: Pixels, viewport_margin: Pixels, pointer_offset: Pixels }

impl Render for PopupMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_submenu_menu_anchor(window, cx);

        // Submenus attached via the public `item()` + `PopupMenuItem::submenu()`
        // path (from contexts that only have the menu value, e.g. a table
        // delegate's `context_menu`) have no parent wired at construction time.
        // Wire them here so the dismiss chain, click-outside checks and keyboard
        // navigation treat them the same as `submenu()`-built children.
        let parent = cx.entity().downgrade();
        let parent_priority = self.priority;
        for item in &self.menu_items {
            if let PopupMenuItem::Submenu { menu, .. } = item {
                if menu.read(cx).parent_menu.is_none() {
                    menu.update(cx, |menu, _| {
                        menu.parent_menu = Some(parent.clone());
                        menu.priority = parent_priority + 1;
                    });
                }
            }
        }

        // An unmounted pane cannot send a final leave callback. Reconcile a
        // keyboard switch/rebuild against the visible chain without retaining
        // old panes or a separate map of hover state.
        if self.parent_menu.is_none() {
            if let Some(owner) = self.pointer_owner {
                if owner != cx.entity_id() && !self.contains_active_pane(owner, cx) {
                    self.pointer_changed(owner, false, window, cx);
                }
            }
        }

        let view = cx.entity().clone();
        let surface_view = view.clone();
        let items_count = self.menu_items.len();

        let max_height = self.max_height.unwrap_or_else(|| {
            let window_half_height = window.window_bounds().get_bounds().size.height * 0.5;
            window_half_height.min(px(450.))
        });

        let has_left_icon = self
            .menu_items
            .iter()
            .any(|item| item.has_left_icon(self.check_side));

        let max_width = self.max_width();
        let options = RenderOptions {
            has_left_icon,
            check_side: self.check_side,
            radius: cx.theme().radius.min(px(8.)),
        };
        let rem = window.rem_size();

        let root = v_flex()
            .id("popup-menu")
            .test_support()
            .role(Role::Menu)
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .hover_listener_mode(gpui::HoverListenerMode::PointerPresence)
            .on_hover(cx.listener(|menu, inside, window, cx| {
                menu.pointer_changed(cx.entity_id(), *inside, window, cx);
            }))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_mouse_down_out(cx.listener(Self::on_mouse_down_out))
            .popover_style(cx)
            .text_color(cx.theme().popover_foreground)
            .refine_style(&self.surface_style)
            .relative()
            .occlude()
            .when(self.pointer_submenus.is_some(), |mut root| {
                let border = root.style().border_widths.clone();
                let resolve = |length: Option<gpui::AbsoluteLength>| length.unwrap_or_default().to_pixels(rem);
                let (left, right, top, bottom) = (resolve(border.left), resolve(border.right), resolve(border.top), resolve(border.bottom));
                root.on_prepaint(move |bounds, _, cx| {
                    // The observation canvas fills the padding box. Recover
                    // the surface's border box for the authored 2px overlap.
                    let bounds = Bounds { origin: bounds.origin - gpui::point(left, top),
                        size: gpui::size(bounds.size.width + left + right, bounds.size.height + top + bottom) };
                    surface_view.update(cx, |menu, _| menu.surface_bounds = bounds);
                })
            })
            .child(
                v_flex()
                    .id("items")
                    .p_1()
                    .when_some(self.content_padding, |items, padding| items.p(padding))
                    .gap_y_0p5()
                    .when_some(self.item_gap, |items, gap| items.gap_y(gap))
                    .min_w(rems(8.))
                    .when_some(self.min_width, |this, min_width| this.min_w(min_width))
                    .max_w(max_width)
                    .when(self.scrollable, |this| {
                        this.max_h(max_height)
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle)
                    })
                    .children(
                        self.menu_items
                            .iter()
                            .enumerate()
                            // Ignore last separator
                            .filter(|(ix, item)| !(*ix + 1 == items_count && item.is_separator()))
                            .map(|(ix, item)| self.render_item(ix, item, options, window, cx)),
                    )
                    .on_prepaint(move |bounds, _, cx| {
                        view.update(cx, |menu, cx| {
                            let width_changed = menu.bounds.size.width != bounds.size.width;
                            menu.bounds = bounds;
                            if width_changed {
                                if let Some(parent) = menu
                                    .parent_menu
                                    .as_ref()
                                    .and_then(|parent| parent.upgrade())
                                {
                                    parent.update(cx, |_, cx| cx.notify());
                                }
                            }
                        });
                    }),
            )
            .when(self.scrollable, |this| {
                this.vertical_scrollbar(&self.scroll_handle)
            });
        if let Some((duration, offset)) = self.entrance.filter(|_| !cx.reduce_motion()) {
            root.with_animation(("popup-entrance", cx.entity_id()), gpui::Animation::new(duration)
                .with_easing(|progress| gpui_base::Easing::Ease.sample(progress)), move |root, progress| {
                    root.opacity(progress).top(offset.y * (1. - progress)).left(offset.x * (1. - progress))
                }).into_any_element()
        } else { root.into_any_element() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn custom_menu_item_preserves_mouse_event(cx: &mut gpui::TestAppContext) {
        use gpui::{Modifiers, MouseButton, MouseUpEvent};
        use std::cell::RefCell;

        cx.update(crate::init);
        let events = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let events = events.clone();
            move |_, cx| {
                PopupMenu::new(cx).item(
                    PopupMenuItem::element(|_, _| {
                        div()
                            .debug_selector(|| "custom-menu-content".into())
                            .w(px(100.))
                            .h(px(20.))
                            .child("Custom")
                    })
                    .on_click(move |event, _, _| events.borrow_mut().push(event.clone())),
                )
            }
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let position = cx.debug_bounds("custom-menu-content").unwrap().center();
        let modifiers = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        cx.simulate_mouse_down(position, MouseButton::Left, modifiers);
        cx.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers,
            click_count: 2,
        });
        cx.run_until_parked();
        let events = events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mouse_position(), Some(position));
        assert_eq!(events[0].modifiers(), modifiers);
        let ClickEvent::Mouse(event) = &events[0] else {
            panic!("expected the original mouse click");
        };
        assert_eq!(event.up.click_count, 2);
    }

    #[gpui::test]
    fn popup_menu_item_a11y_label_uses_visible_label(cx: &mut gpui::TestAppContext) {
        let submenu = cx.update(|cx| cx.new(|cx| PopupMenu::new(cx)));

        assert_eq!(PopupMenuItem::new("Open").a11y_label(), Some("Open".into()));
        assert_eq!(
            PopupMenuItem::link("Docs", "https://example.com").a11y_label(),
            Some("Docs".into())
        );
        assert_eq!(
            PopupMenuItem::label("Recent files").a11y_label(),
            Some("Recent files".into())
        );
        assert_eq!(
            PopupMenuItem::submenu("More", submenu).a11y_label(),
            Some("More".into())
        );
        assert_eq!(PopupMenuItem::separator().a11y_label(), None);
        assert_eq!(PopupMenuItem::element(|_, _| div()).a11y_label(), None);
    }
}
