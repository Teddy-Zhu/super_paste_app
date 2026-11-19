use gpui::{
    AnyElement, App, ClickEvent, Context, DismissEvent, Div, Edges, ElementId, Entity, EntityId, EventEmitter,
    FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, Length, ParentElement, Render,
    KeyDownEvent, Pixels, RenderOnce, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window,
    deferred, div, prelude::FluentBuilder, px, rems,
};
use gpui_base::TestSupportExt as _;
use rust_i18n::t;

use crate::ThemeStyled as _;
use crate::{
    ActiveTheme, Disableable, ElementExt as _, Icon, IconName, IndexPath, Sizable, Size,
    StyleSized, StyledExt,
    actions::{Cancel, Confirm},
    h_flex,
    input::{clear_button, input_style},
    list::List,
    searchable_list::{
        SearchableListChange, SearchableListDelegate, SearchableListItem, SearchableListState,
    },
    sizing::DROPDOWN_LIST_PADDING,
    v_flex,
};
use gpui_base::{GlobalState, Select as BaseSelect};

// MARK: Public re-exports for back-compat

/// Re-exported for backward compatibility. New code should prefer [`SearchableGroup`].
pub use crate::searchable_list::SearchableGroup as SelectGroup;
/// Re-exported for backward compatibility. New code should prefer [`SearchableListDelegate`].
pub use crate::searchable_list::SearchableListDelegate as SelectDelegate;
/// Re-exported for backward compatibility. New code should prefer [`SearchableListItem`].
pub use crate::searchable_list::SearchableListItem as SelectItem;
/// Re-exported for backward compatibility. New code should prefer [`SearchableListItemElement`].
pub use crate::searchable_list::SearchableListItemElement as SelectListItem;
/// Re-exported for backward compatibility.
pub use crate::searchable_list::SearchableVec;

#[derive(IntoElement)]
pub struct Caret {
    size: Size,
    color: Option<Hsla>,
}

impl Caret {
    /// Create a select caret sized for its trigger.
    pub fn new(size: Size) -> Self {
        Self { size, color: None }
    }

    /// Set the caret color.
    pub fn text_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Caret {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::new(IconName::ChevronDown)
            .with_size(match self.size {
                Size::XSmall => Size::XSmall,
                Size::Small => Size::Small,
                _ => Size::Medium,
            })
            .when_some(self.color, |this, color| this.text_color(color))
    }
}

/// Events emitted by [`SelectState`].
pub enum SelectEvent<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    Confirm(Option<<D::Item as SearchableListItem>::Value>),
}

/// Native trigger content and current interaction state. Custom renderers keep
/// the provided root's click handler and consume the selected title/trailing
/// action; selection, keyboard handling and popup ownership remain in Select.
pub struct SelectTrigger {
    pub id: EntityId,
    pub title: AnyElement,
    pub trailing: AnyElement,
    pub disabled: bool,
    pub hovered: bool,
    pub focused: bool,
    pub open: bool,
}

type TriggerBuilder = std::rc::Rc<
    dyn Fn(Stateful<Div>, SelectTrigger, &mut Window, &mut App) -> Stateful<Div>,
>;

/// Opt-in dropdown presentation. Data, cursor and confirmation still belong
/// to Select/List; ordinary controls retain the Kit defaults.
#[derive(Clone)]
pub struct SelectMenuStyle {
    pub surface: StyleRefinement,
    pub row_renderer: Option<crate::searchable_list::SearchableListRowRenderer>,
    pub variable_rows: bool,
    pub animated: bool,
    pub offset: Pixels,
    pub scrollbar: Option<gpui_base::ScrollbarStyles>,
    pub scrollbar_gutter: Option<Pixels>,
    /// Keep focus on the trigger and support prefix/first/last navigation.
    pub trigger_navigation: bool,
    /// Use available viewport height instead of the fixed menu height.
    pub viewport_fit: bool,
}

impl Default for SelectMenuStyle {
    fn default() -> Self {
        Self { surface: StyleRefinement::default(), row_renderer: None,
            variable_rows: false, animated: true, offset: px(6.0), scrollbar: None, scrollbar_gutter: None,
            trigger_navigation: false, viewport_fit: false }
    }
}

// MARK: SelectOptions (builder only — applied to SearchableListState during render)

struct SelectOptions {
    style: StyleRefinement,
    size: Size,
    icon: Option<Icon>,
    cleanable: bool,
    placeholder: Option<SharedString>,
    accessibility_label: Option<SharedString>,
    title_prefix: Option<SharedString>,
    search_placeholder: Option<SharedString>,
    menu_width: Length,
    menu_max_h: Length,
    scroll_bounce: Option<bool>,
    disabled: bool,
    appearance: bool,
    focus_ring_enabled: bool,
    intrinsic_size: bool,
    trigger_builder: Option<TriggerBuilder>,
    menu_style: Option<SelectMenuStyle>,
}

impl Default for SelectOptions {
    fn default() -> Self {
        Self {
            style: StyleRefinement::default(),
            size: Size::default(),
            icon: None,
            cleanable: false,
            placeholder: None,
            accessibility_label: None,
            title_prefix: None,
            menu_width: Length::Auto,
            menu_max_h: rems(20.).into(),
            scroll_bounce: None,
            disabled: false,
            appearance: true,
            focus_ring_enabled: true,
            intrinsic_size: false,
            trigger_builder: None,
            menu_style: None,
            search_placeholder: None,
        }
    }
}

// MARK: SelectState

/// State of the [`Select`] component.
///
/// Emits [`DismissEvent`] when an open menu closes, including after a selection is confirmed.
pub struct SelectState<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    pub(crate) state: SearchableListState<D>,

    // Select-specific fields
    searchable: bool,
    icon: Option<Icon>,
    title_prefix: Option<SharedString>,
    focus_ring_enabled: bool,
    intrinsic_size: bool,
    trigger_builder: Option<TriggerBuilder>,
    trigger_hovered: bool,
    menu_style: Option<SelectMenuStyle>,
    typeahead: String,
    typeahead_last: Option<std::time::Instant>,
    _menu_window_observers: Option<(gpui::Subscription, gpui::Subscription, gpui::Subscription)>,
    menu_accessibility_label: Option<SharedString>,
}

/// A Select element.
#[derive(IntoElement)]
pub struct Select<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    id: ElementId,
    state: Entity<SelectState<D>>,
    options: SelectOptions,
    empty: Option<Box<dyn Fn(&mut Window, &App) -> AnyElement + 'static>>,
}

#[derive(IntoElement)]
struct SelectMenuSurface<D: SearchableListDelegate + 'static>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    state: Entity<SelectState<D>>,
}

impl<D: SearchableListDelegate + 'static> RenderOnce for SelectMenuSurface<D>
where
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Deferred layout runs after the trigger has recorded this frame's bounds.
        self.state.update(cx, |state, cx| state.render_menu(window, cx))
    }
}

impl<D> SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    /// Create a new Select state.
    pub fn new(
        delegate: D,
        selected_index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.entity().downgrade();
        let weak_confirm = weak.clone();
        let weak_cancel = weak.clone();
        let weak_empty = weak;

        let selected_indices = selected_index.into_iter().collect::<Vec<_>>();

        let state = SearchableListState::new(
            delegate,
            selected_indices,
            // on_confirm — commit the selection
            move |selected_index, _secondary, window, cx| {
                cx.defer_in(window, {
                    let weak_confirm = weak_confirm.clone();
                    move |list_state, window, cx| {
                        let mut selection = weak_confirm
                            .upgrade()
                            .map(|e| e.read(cx).state.selection.clone())
                            .unwrap_or_default();

                        let changes = {
                            let mut changes: Vec<SearchableListChange> = selection
                                .iter()
                                .map(|(ix, _)| SearchableListChange::Deselect { index: *ix })
                                .collect();

                            if let Some(ix) = selected_index {
                                changes.push(SearchableListChange::Select { index: ix });
                            }

                            changes
                        };

                        // on_will_change is called directly — entity-handle access would
                        // re-enter the ListState lock that defer_in holds for this callback.
                        list_state
                            .delegate_mut()
                            .delegate
                            .on_will_change(&mut selection, &changes);

                        let confirmed = weak_confirm.update(cx, |this, cx| {
                            this.state.selection = selection;

                            let final_value =
                                this.state.selection.first().map(|(_, i)| i.value().clone());

                            cx.emit(SelectEvent::Confirm(final_value.clone()));
                            cx.notify();
                            this.set_open(false, cx);
                            this.focus(window, cx);

                            (this.state.selection.clone(), final_value)
                        });

                        // Clear the query through list_state directly — an entity-handle
                        // update here would re-enter the ListState lock held above.
                        // The committed index pointed into the filtered view, so resolve
                        // the confirmed value in the restored full list; otherwise the
                        // cursor and the mark would disagree on the next open.
                        if let Ok((mut new_selection, final_value)) = confirmed {
                            if !list_state.query_input.read(cx).value().is_empty() {
                                list_state.set_query("", window, cx);
                            }

                            if let Some(ix) = final_value
                                .as_ref()
                                .and_then(|value| list_state.delegate().delegate.position(value))
                            {
                                list_state.set_selected_index(Some(ix), window, cx);
                                if let Some((slot, _)) = new_selection.first_mut() {
                                    *slot = ix;
                                }
                                _ = weak_confirm.update(cx, |this, cx| {
                                    if let Some((slot, _)) = this.state.selection.first_mut() {
                                        *slot = ix;
                                    }
                                    cx.notify();
                                });
                            }

                            list_state
                                .delegate_mut()
                                .update_selection_snapshot(new_selection.clone());
                            list_state
                                .delegate_mut()
                                .delegate
                                .on_confirm(&new_selection);
                        }
                    }
                });
            },
            // on_cancel — clear the query, restore cursor to committed index, close
            move |_final_selected_index, window, cx| {
                cx.defer_in(window, {
                    let weak_cancel = weak_cancel.clone();
                    move |list_state, window, cx| {
                        let committed_ix = weak_cancel
                            .upgrade()
                            .and_then(|e| e.read(cx).state.selection.first().map(|(ix, _)| *ix));

                        if !list_state.query_input.read(cx).value().is_empty() {
                            list_state.set_query("", window, cx);
                        }
                        list_state.set_selected_index(committed_ix, window, cx);

                        _ = weak_cancel.update(cx, |this, cx| {
                            this.set_open(false, cx);
                            this.focus(window, cx);
                        });
                    }
                });
            },
            // on_render_empty
            move |window, cx| {
                if let Some(empty) = weak_empty
                    .upgrade()
                    .and_then(|e| e.read(cx).state.empty.as_ref().map(|f| f(window, cx)))
                {
                    empty
                } else {
                    h_flex()
                        .justify_center()
                        .py_6()
                        .text_color(cx.theme().muted_foreground.opacity(0.6))
                        .child(Icon::new(IconName::Inbox).size(px(28.)))
                        .into_any_element()
                }
            },
            Self::on_blur,
            window,
            cx,
        );

        Self {
            state,
            searchable: false,
            icon: None,
            title_prefix: None,
            focus_ring_enabled: true,
            intrinsic_size: false,
            trigger_builder: None,
            trigger_hovered: false,
            menu_style: None,
            typeahead: String::new(),
            typeahead_last: None,
            _menu_window_observers: None,
            menu_accessibility_label: None,
        }
    }

    /// Sets whether the dropdown menu is searchable, default is `false`.
    ///
    /// When `true`, a search input appears at the top of the dropdown menu.
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    /// Set the selected index for the select.
    pub fn set_selected_index(
        &mut self,
        selected_index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.list.update(cx, |list, cx| {
            list._set_selected_index(selected_index, window, cx);
        });

        let item = selected_index
            .and_then(|ix| self.state.list.read(cx).delegate().delegate.item(ix))
            .map(|i| i.clone());

        self.state.selection = match (selected_index, item) {
            (Some(ix), Some(item)) => vec![(ix, item)],
            _ => vec![],
        };
        self.state.sync_snapshot(cx);
    }

    /// Set selected value for the select.
    ///
    /// Looks up the position from the delegate and sets the selected index accordingly.
    /// Passes `None` when the value is not found.
    ///
    /// The delegate looks the value up in its matched items, so an active search query is
    /// cleared first to get an index into the full item list.
    pub fn set_selected_value(
        &mut self,
        selected_value: &<D::Item as SearchableListItem>::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.clear_query(window, cx);

        let selected_index = self
            .state
            .list
            .read(cx)
            .delegate()
            .delegate
            .position(selected_value);

        self.set_selected_index(selected_index, window, cx);
    }

    /// Replace the delegate (item data) for the select state.
    pub fn set_items(&mut self, items: D, window: &mut Window, cx: &mut Context<Self>)
    where
        D: SearchableListDelegate + 'static,
    {
        let trigger_navigation = self.trigger_navigation();
        let mut selection = self.state.selection.clone();
        self.state.list.update(cx, |list, cx| {
            let cursor = if trigger_navigation {
                list.selected_index().filter(|index| items.item(*index)
                    .is_some_and(|item| items.is_item_enabled(*index, item, cx)))
            } else {
                list.selected_index().and_then(|ix| list.delegate().delegate.item(ix))
                    .and_then(|item| items.position(item.value()))
            };
            // A value the new items no longer have is dropped, as
            // `set_selected_value` does, rather than kept at an index that
            // now points at another item.
            selection.retain_mut(|(ix, item)| {
                let Some(position) = items.position(item.value()) else {
                    return false;
                };
                let Some(replacement) = items.item(position) else {
                    return false;
                };
                *ix = position;
                *item = replacement.clone();
                true
            });
            list.delegate_mut().delegate = items;
            list.set_selected_index(cursor, window, cx);
            list.delegate_mut()
                .update_selection_snapshot(selection.clone());
            cx.notify();
        });
        self.state.selection = selection;
        if trigger_navigation && self.state.open {
            let first = self.enabled_indices(cx).next();
            if first.is_none() { self.dismiss_menu(false, window, cx); }
            else {
                let active = self.selected_index(cx).or(first);
                self.set_active(active, window, cx);
            }
        }
        cx.notify();
    }

    /// Get the current selected index.
    pub fn selected_index(&self, cx: &App) -> Option<IndexPath> {
        self.state.list.read(cx).selected_index()
    }

    /// Get the current selected value.
    pub fn selected_value(&self) -> Option<&<D::Item as SearchableListItem>::Value> {
        self.state.selection.first().map(|(_, i)| i.value())
    }

    /// Focus the select trigger input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.focus_handle.focus(window, cx);
    }

    fn trigger_navigation(&self) -> bool {
        !self.searchable && self.menu_style.as_ref().is_some_and(|style| style.trigger_navigation)
    }

    fn enabled_indices<'a>(&'a self, cx: &'a App) -> impl DoubleEndedIterator<Item = IndexPath> + 'a {
        let delegate = &self.state.list.read(cx).delegate().delegate;
        (0..delegate.sections_count(cx)).flat_map(move |section| {
            (0..delegate.items_count(section)).map(move |row| IndexPath { section, row, ..Default::default() })
        }).filter(move |index| delegate.item(*index).is_some_and(|item| delegate.is_item_enabled(*index, item, cx)))
    }

    fn set_active(&mut self, index: Option<IndexPath>, window: &mut Window, cx: &mut Context<Self>) {
        self.state.list.update(cx, |list, cx| {
            list.set_selected_index(index, window, cx);
            list.scroll_to_selected_item(window, cx);
            cx.notify();
        });
        cx.notify();
    }

    fn open_trigger_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.disabled || self.state.open { return; }
        let delegate = &self.state.list.read(cx).delegate().delegate;
        if !(0..delegate.sections_count(cx)).any(|section| delegate.items_count(section) > 0) { return; }
        let committed = self.state.selection.first().map(|(index, _)| *index);
        let initial = committed.filter(|index| self.enabled_indices(cx).any(|candidate| candidate == *index))
            .or_else(|| self.enabled_indices(cx).next());
        self.set_active(initial, window, cx);
        self._menu_window_observers = Some((
            cx.observe_window_activation(window, |this, window, cx| {
                if this.state.open && !window.is_window_active() { this.dismiss_menu(false, window, cx); }
            }),
            cx.observe_window_visibility(window, |this, visibility, window, cx| {
                if this.state.open && visibility == gpui::WindowVisibility::Hidden { this.dismiss_menu(false, window, cx); }
            }),
            cx.observe_window_scroll(window, |this, event, window, cx| {
                if this.state.open && !event.is_from(this.state.list.read(cx).scroll_handle().base_handle()) {
                    this.dismiss_menu(false, window, cx);
                }
            }),
        ));
        self.set_open(true, cx);
        self.focus(window, cx);
    }

    fn dismiss_menu(&mut self, return_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_query_and_restore_cursor(window, cx);
        self.set_open(false, cx);
        if return_focus { self.focus(window, cx); }
    }

    fn move_active(&mut self, direction: i8, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.selected_index(cx);
        let next = if direction < 0 {
            self.enabled_indices(cx).rev().find(|index| cursor.is_none_or(|cursor| (index.section, index.row) < (cursor.section, cursor.row)))
                .or_else(|| self.enabled_indices(cx).next_back())
        } else {
            self.enabled_indices(cx).find(|index| cursor.is_none_or(|cursor| (index.section, index.row) > (cursor.section, cursor.row)))
                .or_else(|| self.enabled_indices(cx).next())
        };
        self.set_active(next, window, cx);
    }

    fn confirm_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.list.update(cx, |list, cx| list.on_action_confirm(&Confirm { secondary: false }, window, cx));
    }

    fn trigger_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.trigger_navigation() || self.state.disabled { return; }
        let key = &event.keystroke;
        let handled = match key.key.as_str() {
            "up" | "down" => {
                let direction = if key.key == "up" { -1 } else { 1 };
                if self.state.open { self.move_active(direction, window, cx); }
                else {
                    let selected = self.state.selection.first().map(|(index, _)| *index);
                    let had_selection = self.enabled_indices(cx).any(|index| Some(index) == selected);
                    self.open_trigger_menu(window, cx);
                    if self.state.open {
                        if had_selection { self.move_active(direction, window, cx); }
                        else if direction < 0 {
                            let last = self.enabled_indices(cx).next_back();
                            self.set_active(last, window, cx);
                        }
                    }
                }
                true
            }
            "home" | "end" if self.state.open => {
                let index = if key.key == "home" { self.enabled_indices(cx).next() }
                    else { self.enabled_indices(cx).next_back() };
                self.set_active(index, window, cx);
                true
            }
            "escape" if self.state.open => { self.dismiss_menu(true, window, cx); true }
            "tab" if self.state.open => {
                // Let the window's existing Tab traversal continue from the trigger.
                self.dismiss_menu(false, window, cx);
                false
            }
            "enter" | "space" => {
                if self.state.open { self.confirm_active(window, cx); }
                else { self.open_trigger_menu(window, cx); }
                true
            }
            _ => {
                let text = key.key_char.as_deref().unwrap_or(&key.key);
                if key.modifiers.control || key.modifiers.platform || key.modifiers.alt
                    || text.encode_utf16().count() != 1 || text.chars().any(char::is_control) { return; }
                let now = std::time::Instant::now();
                if self.typeahead_last.is_none_or(|last| now.duration_since(last) >= std::time::Duration::from_millis(700)) {
                    self.typeahead.clear();
                }
                self.typeahead.push_str(&text.to_lowercase());
                self.typeahead_last = Some(now);
                let cursor = self.state.open.then(|| self.selected_index(cx)).flatten();
                let delegate = &self.state.list.read(cx).delegate().delegate;
                let after = |index: &IndexPath| cursor.is_none_or(|cursor| (index.section, index.row) > (cursor.section, cursor.row));
                let found = self.enabled_indices(cx).filter(after)
                    .chain(self.enabled_indices(cx).filter(|index| !after(index)))
                    .find(|index| delegate.item(*index).is_some_and(|item| item.title().to_lowercase().starts_with(&self.typeahead)));
                if let Some(index) = found {
                    self.set_active(Some(index), window, cx);
                    if !self.state.open { self.confirm_active(window, cx); }
                }
                true
            }
        };
        if handled { window.prevent_default(); cx.stop_propagation(); }
    }

    fn on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.list.read(cx).is_focused(window, cx)
            || self.state.focus_handle.is_focused(window)
        {
            return;
        }

        self.clear_query_and_restore_cursor(window, cx);
        self.set_open(false, cx);
        cx.notify();
    }

    fn toggle_menu(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();

        if self.trigger_navigation() {
            if self.state.open { self.dismiss_menu(false, window, cx); }
            else { self.open_trigger_menu(window, cx); }
            return;
        }

        self.set_open(!self.state.open, cx);

        if self.state.open {
            self.state.list.focus_handle(cx).focus(window, cx);
        } else {
            self.clear_query_and_restore_cursor(window, cx);
        }

        cx.notify();
    }

    fn escape(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.open {
            cx.propagate();
            return;
        }

        cx.stop_propagation();
        self.clear_query_and_restore_cursor(window, cx);
        self.set_open(false, cx);
        self.focus(window, cx);
        cx.notify();
    }

    /// Drop the search query and move the cursor back to the committed
    /// selection, so the next open shows every item. Call on every menu
    /// close that does not go through the confirm/cancel callbacks.
    fn clear_query_and_restore_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.clear_query(window, cx);

        let committed_ix = self.state.selection.first().map(|(ix, _)| *ix);
        self.state.list.update(cx, |list, cx| {
            if list.selected_index() != committed_ix {
                list.set_selected_index(committed_ix, window, cx);
            }
        });
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.state.open == open { return; }
        let dismissed = self.state.open && !open;
        if open && !self.state.open && self.menu_style.as_ref().is_some_and(|style| style.variable_rows) {
            self.state.list.update(cx, |list, cx| list.reset_flow_scroll_to_selection(cx));
        }
        self.state.open = open;
        if !open { self._menu_window_observers = None; }
        self.state.deferred_context = open.then(|| GlobalState::register_deferred_popover(cx));

        if dismissed {
            cx.emit(DismissEvent);
        }
        cx.notify();
    }

    fn clean(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.set_selected_index(None, window, cx);
        cx.emit(SelectEvent::Confirm(None));
    }

    fn render_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.state.open { return div().into_any_element(); }
        let bounds = self.state.bounds;
        let menu_style = self.menu_style.clone().unwrap_or_default();
        let space = if menu_style.viewport_fit {
            let Some(space) = gpui_base::Positioner::dropdown_space(bounds, window, menu_style.offset, px(8.0), px(180.0)) else {
                self.dismiss_menu(false, window, cx);
                return div().into_any_element();
            };
            Some(space)
        } else { None };
        let max_height: Length = space.map(|space| {
            let border = crate::styled::border_widths(&menu_style.surface, window.rem_size());
            (space.height - border.top - border.bottom).max(px(0.)).into()
        }).unwrap_or(self.state.menu_max_h);
        let surface = v_flex().occlude()
            .map(|root| match self.state.menu_width {
                Length::Auto => root.w(bounds.size.width),
                Length::Definite(width) => root.w(width),
            })
            .popover_style(cx).refine_style(&menu_style.surface)
            .when_some(space, |root, space| root.max_w(space.width).max_h(space.height))
            .child(List::new(&self.state.list)
                .when_some(self.menu_accessibility_label.clone(), |list, label| list.accessibility_label(label))
                .when_some(self.state.search_placeholder.clone(), |list, placeholder| list.search_placeholder(placeholder))
                .with_size(self.state.size).max_h(max_height).variable_rows(menu_style.variable_rows)
                .when_some(menu_style.scrollbar.clone(), |list, styles| list.flow_scrollbar_styles(styles))
                .when_some(menu_style.scrollbar_gutter, |list, width| list.flow_scrollbar_gutter(width))
                .when(self.trigger_navigation(), |list| list.active_descendant_owner(&self.state.focus_handle))
                .when_some(self.state.scroll_bounce, |list, on| list.scroll_bounce(on))
                .paddings(Edges::all(DROPDOWN_LIST_PADDING)))
            .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                if this.trigger_navigation() {
                    if !this.state.bounds.contains(&event.position) { this.dismiss_menu(false, window, cx); }
                } else { this.escape(&Cancel, window, cx); }
            }));
        let popup = if menu_style.animated {
            crate::popover::dropdown_popup(("select-popup", cx.entity_id()), bounds, surface, cx)
        } else {
            gpui_base::Positioner::side(bounds).placement(gpui_base::Placement::Bottom)
                .align(gpui_base::Align::Start).offset(menu_style.offset).margin(px(8.0)).child(surface)
        };
        popup.when(menu_style.viewport_fit, |popup| popup.offset(menu_style.offset).margin(px(8.0)).dropdown_flip(px(180.0)))
            .into_any_element()
    }

    fn display_title(&mut self, _: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let default_title = div().text_color(cx.theme().muted_foreground).child(
            self.state
                .placeholder
                .clone()
                .unwrap_or_else(|| t!("Select.placeholder").into()),
        );

        let render_title = |item: &D::Item| {
            if let Some(element) = item.display_title() { element }
            else if let Some(prefix) = self.title_prefix.as_ref() {
                format!("{}{}", prefix, item.title()).into_any_element()
            } else { item.title().into_any_element() }
        };
        let title = if self.menu_style.is_some() {
            // A hover/keyboard cursor is not a committed value, and a search
            // result index cannot address the committed full-list item.
            self.state.selection.first().map(|(_, item)| render_title(item))
        } else {
            self.selected_index(cx).and_then(|index| {
                self.state.list.read(cx).delegate().delegate.item(index).map(render_title)
            })
        };
        let Some(title) = title else { return default_title; };

        div()
            .when(self.state.disabled && (self.state.appearance || self.trigger_builder.is_none()), |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .child(title)
    }

    fn accessibility_value(&self) -> SharedString {
        let Some((_, item)) = self.state.selection.first() else {
            return self
                .state
                .placeholder
                .clone()
                .unwrap_or_else(|| t!("Select.placeholder").into());
        };

        if let Some(prefix) = self.title_prefix.as_ref() {
            format!("{}{}", prefix, item.title()).into()
        } else {
            item.title()
        }
    }
}

impl<D> Render for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let searchable = self.searchable;
        let is_focused = self.state.focus_handle.is_focused(window);
        let show_clean = self.state.cleanable && self.selected_index(cx).is_some();
        let allow_open = !self.state.disabled && (!self.state.open || self.trigger_navigation());
        let outline_visible = self.state.open || (is_focused && !self.state.disabled);

        let (bg, fg) = input_style(self.state.disabled, cx);

        self.state.list.update(cx, |list, cx| {
            list.set_searchable(searchable, cx);
            list.delegate_mut().size = self.state.size;
            list.delegate_mut().row_renderer = self.menu_style.as_ref().and_then(|style| style.row_renderer.clone());
        });

        let title = self.display_title(window, cx).into_any_element();
        let trailing = if show_clean {
            clear_button(cx).map(|button| {
                if self.state.disabled { button.disabled(true) }
                else { button.on_click(cx.listener(Self::clean)) }
            }).into_any_element()
        } else {
            match self.icon.clone() {
                Some(icon) => icon.when(self.trigger_builder.is_none(), |icon| {
                    icon.xsmall().text_color(cx.theme().muted_foreground)
                }).into_any_element(),
                None => Caret::new(self.state.size).text_color(cx.theme().muted_foreground).into_any_element(),
            }
        };
        let trigger = div()
            .id("input")
            .relative()
            .flex()
            .items_center()
            .justify_between()
            .border_1()
            .border_color(cx.theme().transparent)
            .when(self.state.appearance, |this| {
                this.bg(bg)
                    .text_color(fg)
                    .when(self.state.disabled, |this| this.opacity(0.5))
                    .border_color(cx.theme().input)
                    .rounded(cx.theme().radius)
            })
            .input_size(self.state.size)
            .input_text_size(self.state.size)
            .refine_style(&self.state.style)
            .when(outline_visible && self.state.appearance, |this| {
                this.border_1().border_color(cx.theme().ring)
            })
            .when(
                outline_visible && self.state.appearance && self.focus_ring_enabled,
                |this| this.focus_ring_style(window, cx),
            )
            .when(self.trigger_builder.is_some(), |this| {
                this.on_hover(cx.listener(|this, hovered, _, cx| {
                    this.trigger_hovered = *hovered;
                    cx.notify();
                }))
            })
            .when(allow_open, |this| {
                this.on_click(cx.listener(Self::toggle_menu))
            });
        let input = if let Some(builder) = self.trigger_builder.clone() {
            builder(
                trigger,
                SelectTrigger {
                    id: cx.entity_id(),
                    title,
                    trailing,
                    disabled: self.state.disabled,
                    hovered: self.trigger_hovered && !self.state.disabled,
                    focused: is_focused && !self.state.disabled,
                    open: self.state.open,
                },
                window,
                cx,
            )
        } else {
            trigger
                .child(
                    h_flex()
                        .id("inner")
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .items_center()
                        .justify_between()
                        .gap_1()
                        .child(
                            div().id("title").flex_1().min_w_0()
                                .overflow_hidden().whitespace_nowrap().truncate().child(title),
                        )
                        .child(trailing),
                )
        };

        div().when(!self.intrinsic_size, |root| root.size_full()).relative().child(
            div()
                .relative()
                .on_prepaint({
                    let state = cx.entity();
                    move |bounds, _, cx| state.update(cx, |r, _| r.state.bounds = bounds)
                })
                .child(input.test_support())
                .when(self.state.open, |root| root.child(
                    deferred(SelectMenuSurface { state: cx.entity() })
                        .with_late_layout(window.viewport_size().map(gpui::AvailableSpace::Definite))
                        .with_priority(gpui_base::POPUP_PRIORITY)
                )),
        )
    }
}

impl<D> Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    pub fn new(state: &Entity<SelectState<D>>) -> Self {
        Self {
            id: ("select", state.entity_id()).into(),
            state: state.clone(),
            options: SelectOptions::default(),
            empty: None,
        }
    }

    /// Sets an explicit identity for the select root.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    /// Let the trigger's native content and constraints size the wrapper,
    /// instead of filling the parent's dimensions. The default stays full size.
    pub fn intrinsic_size(mut self, intrinsic: bool) -> Self {
        self.options.intrinsic_size = intrinsic;
        self
    }

    /// Customize the wired trigger while retaining native Select behavior.
    pub fn trigger(
        mut self,
        builder: impl Fn(Stateful<Div>, SelectTrigger, &mut Window, &mut App) -> Stateful<Div> + 'static,
    ) -> Self {
        self.options.trigger_builder = Some(std::rc::Rc::new(builder));
        self
    }

    /// Set the width of the dropdown menu, default: `Length::Auto`.
    pub fn menu_width(mut self, width: impl Into<Length>) -> Self {
        self.options.menu_width = width.into();
        self
    }

    pub fn menu_style(mut self, style: SelectMenuStyle) -> Self {
        self.options.menu_style = Some(style);
        self
    }

    /// Set the max height of the dropdown menu, default: 20rem.
    pub fn menu_max_h(mut self, max_h: impl Into<Length>) -> Self {
        self.options.menu_max_h = max_h.into();
        self
    }

    /// Set the placeholder shown when no value is selected.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.placeholder = Some(placeholder.into());
        self
    }

    /// Set the name a screen reader announces for the select.
    ///
    /// The placeholder and selected value are not used as the accessible name,
    /// because they describe the current value rather than the control itself.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.options.accessibility_label = Some(label.into());
        self
    }

    /// Override the trailing icon, replacing the default chevron.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.options.icon = Some(icon.into());
        self
    }

    /// Set a label prefix shown before the selected title in the trigger.
    ///
    /// e.g. `title_prefix("Country: ")` → "Country: United States"
    pub fn title_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.options.title_prefix = Some(prefix.into());
        self
    }

    /// Show a clear button when a value is selected.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.options.cleanable = cleanable;
        self
    }

    /// Set the placeholder text for the search input.
    pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.options.search_placeholder = Some(placeholder.into());
        self
    }

    /// Bounce the dropdown list at its vertical boundaries. On by default on iOS,
    /// Android and macOS, off elsewhere; reduced motion disables displacement.
    pub fn scroll_bounce(mut self, enabled: bool) -> Self {
        self.options.scroll_bounce = Some(enabled);
        self
    }

    /// Set the disabled state.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.options.disabled = disabled;
        self
    }

    /// Set a custom closure that renders the empty-state element.
    pub fn empty<E: IntoElement + 'static>(
        mut self,
        builder: impl Fn(&mut Window, &App) -> E + 'static,
    ) -> Self {
        self.empty = Some(Box::new(move |window, cx| {
            builder(window, cx).into_any_element()
        }));
        self
    }

    /// Control whether the trigger shows a border and background (`true` by default).
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.options.appearance = appearance;
        self
    }
}

impl<D> Sizable for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.options.size = size.into();
        self
    }
}

impl<D> crate::FocusableExt for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn focus_ring(mut self, enabled: bool) -> Self {
        self.options.focus_ring_enabled = enabled;
        self
    }

    fn is_focus_ring_enabled(&self) -> bool {
        self.options.focus_ring_enabled
    }
}

impl<D> EventEmitter<SelectEvent<D>> for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
}

impl<D> EventEmitter<DismissEvent> for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
}

impl<D> Focusable for SelectState<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.state.open {
            self.state.list.focus_handle(cx)
        } else {
            self.state.focus_handle.clone()
        }
    }
}

impl<D> Styled for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.options.style
    }
}

impl<D> RenderOnce for Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: PartialEq + Clone,
{
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.options.disabled;
        let accessibility_label = self.options.accessibility_label.clone();
        let focus_handle = self.state.read(cx).state.focus_handle.clone();
        let empty = self.empty;
        let opts = self.options;
        let intrinsic_size = opts.intrinsic_size;

        self.state.update(cx, |this, cx| {
            if disabled && this.state.open {
                this.clear_query_and_restore_cursor(window, cx);
                this.set_open(false, cx);
            }
            this.state.style = opts.style;
            this.state.size = opts.size;
            this.state.cleanable = opts.cleanable;
            this.state.placeholder = opts.placeholder;
            this.state.search_placeholder = opts.search_placeholder;
            this.state.menu_width = opts.menu_width;
            this.state.menu_max_h = opts.menu_max_h;
            this.state.scroll_bounce = opts.scroll_bounce;
            this.state.disabled = opts.disabled;
            this.state.appearance = opts.appearance;
            this.focus_ring_enabled = opts.focus_ring_enabled;
            this.intrinsic_size = opts.intrinsic_size;
            if opts.trigger_builder.is_none() { this.trigger_hovered = false; }
            this.trigger_builder = opts.trigger_builder;
            this.menu_style = opts.menu_style;
            this.menu_accessibility_label = opts.accessibility_label;
            this.icon = opts.icon;
            this.title_prefix = opts.title_prefix;

            if let Some(empty) = empty {
                this.state.empty = Some(empty);
            }
        });

        let is_open = self.state.read(cx).state.open;
        let accessibility_value = self.state.read(cx).accessibility_value();
        let trigger_navigation = self.state.read(cx).trigger_navigation();
        let popup_focus_handle = self.state.read(cx).state.list.focus_handle(cx);
        let content_focus_handle = if trigger_navigation { focus_handle.clone() }
            else { popup_focus_handle.clone() };
        let open_state = self.state.clone();
        let key_state = self.state.clone();

        BaseSelect::new(self.id)
            .open(is_open)
            .disabled(disabled)
            .when_some(accessibility_label, |this, label| {
                this.accessibility_label(label)
            })
            .focus_handle(&focus_handle)
            .content_focus_handle(&content_focus_handle)
            .when(trigger_navigation, |root| root.accessibility_controls(&popup_focus_handle))
            .accessibility_value(accessibility_value)
            .when(trigger_navigation, |root| root.capture_key_down(move |event, window, cx| {
                key_state.update(cx, |state, cx| state.trigger_key_down(event, window, cx));
            }))
            .on_open_change(move |open, window, cx| {
                open_state.update(cx, |state, cx| {
                    if open && state.trigger_navigation() {
                        state.open_trigger_menu(window, cx);
                        return;
                    }
                    if !open {
                        state.clear_query_and_restore_cursor(window, cx);
                    }
                    state.set_open(open, cx);
                });
            })
            .when(!intrinsic_size, |root| root.size_full())
            .child(self.state)
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, RenderOnce as _, TestAppContext};

    use crate::{
        IndexPath,
        searchable_list::{SearchableListDelegate as _, SearchableVec},
        select::{Select, SelectGroup, SelectState},
    };

    #[gpui::test]
    fn an_explicit_accessibility_label_does_not_replace_the_placeholder(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx));

            let plain = Select::new(&state).placeholder("Choose a language");
            assert_eq!(plain.options.accessibility_label, None);
            assert_eq!(
                plain.options.placeholder.as_deref(),
                Some("Choose a language")
            );

            let named = Select::new(&state)
                .placeholder("Choose a language")
                .accessibility_label("Programming language");
            assert_eq!(
                named.options.accessibility_label.as_deref(),
                Some("Programming language")
            );
            assert_eq!(
                named.options.placeholder.as_deref(),
                Some("Choose a language"),
                "an accessible name must not change what is drawn"
            );
        });
    }

    #[gpui::test]
    fn test_select_initial_selection_seeds_cursor(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| SelectState::new(items, Some(IndexPath::new(1)), window, cx));

            assert_eq!(
                state.read(cx).selected_index(cx),
                Some(IndexPath::new(1)),
                "initial cursor should be seeded on ListState so display_title can read it",
            );
            assert_eq!(state.read(cx).selected_value(), Some(&"Go"));
        });
    }

    #[gpui::test]
    fn test_select_initial_grouped_selection_seeds_cursor(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let mut groups: SearchableVec<SelectGroup<&'static str>> = SearchableVec::new(vec![]);
            groups.push(SelectGroup::new("A").items(["Apple", "Avocado"]));
            groups.push(SelectGroup::new("B").items(["Banana", "Blueberry", "Blackberry"]));

            let initial = IndexPath::new(1).section(1);
            let state = cx.new(|cx| SelectState::new(groups, Some(initial), window, cx));

            assert_eq!(state.read(cx).selected_index(cx), Some(initial));
            assert_eq!(state.read(cx).selected_value(), Some(&"Blueberry"));
        });
    }

    #[gpui::test]
    fn test_select_set_selected_value_clears_search_query(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(true));
            let list = state.read(cx).state.list.clone();

            list.update(cx, |list, cx| list.set_query("Rust", window, cx));
            assert_eq!(list.read(cx).delegate().delegate.items_count(0), 1);

            state.update(cx, |state, cx| {
                state.set_selected_value(&"Go", window, cx);
            });

            assert_eq!(state.read(cx).selected_value(), Some(&"Go"));
            assert_eq!(state.read(cx).selected_index(cx), Some(IndexPath::new(1)));
            assert_eq!(list.read(cx).query_input.read(cx).value(), "");
        });
    }

    #[gpui::test]
    fn test_select_set_selected_value_clears_grouped_search_query(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let mut groups: SearchableVec<SelectGroup<&'static str>> = SearchableVec::new(vec![]);
            groups.push(SelectGroup::new("A").items(["Apple", "Avocado"]));
            groups.push(SelectGroup::new("B").items(["Banana", "Blueberry"]));

            let state = cx.new(|cx| SelectState::new(groups, None, window, cx).searchable(true));
            let list = state.read(cx).state.list.clone();

            list.update(cx, |list, cx| list.set_query("Blue", window, cx));
            state.update(cx, |state, cx| {
                state.set_selected_value(&"Banana", window, cx);
            });

            assert_eq!(state.read(cx).selected_value(), Some(&"Banana"));
            assert_eq!(
                state.read(cx).selected_index(cx),
                Some(IndexPath::new(0).section(1)),
            );
        });
    }

    #[gpui::test]
    fn test_select_accessibility_value_tracks_placeholder_and_selection(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let window = cx.add_empty_window();
        window.update(|window, cx| {
            let items = SearchableVec::new(vec!["Rust", "Go"]);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(true));

            _ = Select::new(&state)
                .placeholder("Choose a language")
                .accessibility_label("Programming language")
                .render(window, cx);
            assert_eq!(state.read(cx).accessibility_value(), "Choose a language");

            state.update(cx, |state, cx| {
                state.set_selected_value(&"Rust", window, cx);
            });
            assert_eq!(state.read(cx).accessibility_value(), "Rust");

            let list = state.read(cx).state.list.clone();
            list.update(cx, |list, cx| list.set_query("Go", window, cx));
            assert_eq!(list.read(cx).delegate().delegate.items_count(0), 1);
            // Filtering changes the available rows, not the committed value.
            assert_eq!(state.read(cx).accessibility_value(), "Rust");

            _ = Select::new(&state)
                .placeholder("Choose a language")
                .title_prefix("Language: ")
                .render(window, cx);
            assert_eq!(state.read(cx).accessibility_value(), "Language: Rust");

            state.update(cx, |state, cx| state.set_selected_index(None, window, cx));
            assert_eq!(state.read(cx).accessibility_value(), "Choose a language");

            _ = Select::new(&state).render(window, cx);
            assert_eq!(
                state.read(cx).accessibility_value(),
                rust_i18n::t!("Select.placeholder").to_string(),
            );
        });
    }

    #[gpui::test]
    fn replacing_items_reconciles_the_cursor_and_selection(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let state = cx.new(|cx| {
                SelectState::new(
                    vec!["main", "topic", "release"],
                    Some(IndexPath::new(0)),
                    window,
                    cx,
                )
            });
            let list = state.read(cx).state.list.clone();
            list.update(cx, |list, cx| {
                list.set_selected_index(Some(IndexPath::new(1)), window, cx)
            });
            state.update(cx, |state, cx| {
                state.set_items(vec!["release", "main", "topic"], window, cx)
            });
            assert_eq!(state.read(cx).selected_index(cx), Some(IndexPath::new(2)));
            assert_eq!(state.read(cx).selected_value(), Some(&"main"));
            assert_eq!(state.read(cx).state.selection[0].0, IndexPath::new(1));
            assert_eq!(
                list.read(cx).delegate().selection_snapshot[0].0,
                IndexPath::new(1)
            );
            state.update(cx, |state, cx| {
                state.set_items(vec!["main", "replacement"], window, cx)
            });
            assert_eq!(state.read(cx).selected_index(cx), None);
            assert_eq!(state.read(cx).selected_value(), Some(&"main"));

            // A selected value the new items no longer have is dropped.
            state.update(cx, |state, cx| {
                state.set_items(vec!["replacement", "other"], window, cx)
            });
            assert_eq!(state.read(cx).selected_value(), None);
            assert!(state.read(cx).state.selection.is_empty());
            assert!(list.read(cx).delegate().selection_snapshot.is_empty());
        });
    }

    #[derive(Clone)]
    struct RemoteChoice {
        value: &'static str,
        title: &'static str,
        unavailable: bool,
    }

    impl super::SelectItem for RemoteChoice {
        type Value = &'static str;
        fn value(&self) -> &&'static str {
            &self.value
        }
        fn title(&self) -> gpui::SharedString {
            self.title.into()
        }
        fn disabled(&self) -> bool {
            self.unavailable
        }
    }

    #[gpui::test]
    fn disabled_items_cannot_be_confirmed(cx: &mut TestAppContext) {
        use crate::list::ListDelegate as _;
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        for unavailable in [true, false] {
            let state = cx.update(|window, cx| {
                let state = cx.new(|cx| {
                    SelectState::new(
                        vec![RemoteChoice {
                            value: "main",
                            title: "Main",
                            unavailable,
                        }],
                        Some(IndexPath::new(0)),
                        window,
                        cx,
                    )
                });
                state.update(cx, |state, cx| state.set_open(true, cx));
                let list = state.read(cx).state.list.clone();
                list.update(cx, |list, cx| {
                    list.delegate_mut().confirm(false, window, cx)
                });
                state
            });
            cx.run_until_parked();
            cx.update(|_, cx| assert_eq!(state.read(cx).state.open, unavailable));
        }
    }

    /// A page holding a select under retained views.
    #[cfg(feature = "gpui-fast")]
    mod retained {
        use std::{cell::Cell, rc::Rc};

        use gpui::{
            AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
            TestAppContext, Window, div, px,
        };

        use crate::{
            searchable_list::SearchableVec,
            select::{Select, SelectState},
        };

        /// A page that shows a select and what it selected, counting its
        /// renders.
        struct Page {
            state: Entity<SelectState<SearchableVec<&'static str>>>,
            renders: Rc<Cell<usize>>,
        }

        impl Render for Page {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.renders.set(self.renders.get() + 1);
                let selected = self.state.read(cx).selected_index(cx);
                div()
                    .w(px(240.))
                    .child(Select::new(&self.state))
                    .child(format!("{selected:?}"))
            }
        }

        /// Drawing a select again changes nothing in it: the page around it,
        /// which reads its selection, is not built again for it.
        #[gpui::test]
        fn drawing_a_select_again_leaves_the_page_around_it_retained(cx: &mut TestAppContext) {
            cx.update(crate::init);
            let renders = Rc::new(Cell::new(0));
            let (_, cx) = cx.add_window_view({
                let renders = renders.clone();
                move |window, cx| {
                    let items = SearchableVec::new(vec!["Rust", "Go", "C++"]);
                    Page {
                        state: cx.new(|cx| SelectState::new(items, None, window, cx)),
                        renders,
                    }
                }
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let rendered = renders.get();
            for _ in 0..3 {
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            assert_eq!(renders.get(), rendered, "the page was built again");
        }
    }
}
