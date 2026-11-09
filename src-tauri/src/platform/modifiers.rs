use crate::events::emit_window;
use gtk::prelude::*;
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct ModifierEmitter {
    app: tauri::AppHandle,
    previous: Cell<Option<(bool, bool)>>,
    sequence: Cell<u64>,
}

impl ModifierEmitter {
    fn publish(&self, state: gdk::ModifierType, force: bool) {
        let current = (
            state.contains(gdk::ModifierType::CONTROL_MASK),
            state.intersects(gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK),
        );
        if !force && self.previous.get() == Some(current) {
            return;
        }
        let sequence = self.sequence.get() + 1;
        self.sequence.set(sequence);
        match emit_window(
            &self.app,
            "overlay",
            "overlay-modifiers-changed",
            vec![json!({ "control": current.0, "meta": current.1, "sequence": sequence })],
        ) {
            Ok(()) => self.previous.set(Some(current)),
            Err(error) => tracing::error!(%error, "overlay modifier delivery failed"),
        }
    }
}

fn current_state(keymap: &gdk::Keymap) -> gdk::ModifierType {
    let mut state = gdk::ModifierType::from_bits_truncate(keymap.modifier_state());
    keymap.add_virtual_modifiers(&mut state);
    state
}

/// Observe compositor/XKB modifier state independently of WebKit and its input method.
pub fn install(app: &tauri::AppHandle, window: &tauri::Window) -> Result<(), String> {
    let host = window.gtk_window().map_err(|error| error.to_string())?;
    let keymap = gdk::Keymap::for_display(&host.display()).ok_or("GDK 键盘映射不可用")?;
    let emitter = Rc::new(ModifierEmitter {
        app: app.clone(),
        previous: Cell::new(None),
        sequence: Cell::new(0),
    });
    let weak_host = host.downgrade();
    let changes = emitter.clone();
    let handler = keymap.connect_state_changed(move |keymap| {
        if let Some(host) = weak_host.upgrade()
            && host.is_visible()
        {
            changes.publish(current_state(keymap), false);
        }
    });

    let mapped = emitter.clone();
    let mapped_keymap = keymap.clone();
    host.connect_map(move |_| mapped.publish(current_state(&mapped_keymap), true));

    let focus = emitter.clone();
    let focus_keymap = keymap.clone();
    host.connect_focus_in_event(move |_, _| {
        focus.publish(current_state(&focus_keymap), true);
        glib::Propagation::Proceed
    });
    let blur = emitter.clone();
    host.connect_focus_out_event(move |host, _| {
        // X11 continues reporting modifier changes without focus. Wayland stops
        // delivering keyboard state on leave; refresh it on the next focus-in.
        if !host.display().is::<gdkx11::X11Display>() {
            blur.publish(gdk::ModifierType::empty(), true);
        }
        glib::Propagation::Proceed
    });
    host.connect_hide(move |_| emitter.publish(gdk::ModifierType::empty(), true));
    let handler = RefCell::new(Some(handler));
    host.connect_destroy(move |_| {
        if let Some(handler) = handler.borrow_mut().take() {
            keymap.disconnect(handler);
        }
    });
    Ok(())
}
