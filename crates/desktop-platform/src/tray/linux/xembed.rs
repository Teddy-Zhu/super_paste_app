//! GTK owns the legacy XEmbed protocol and native menus. It is initialized only
//! after the SNI watcher becomes unavailable, on one owned GTK event thread.
#![allow(unsafe_code)]

use super::super::*;
use gtk::{gdk, gdk_pixbuf, glib, prelude::*};
use glib::translate::*;
use std::{cell::Cell, rc::Rc, sync::Mutex, thread::JoinHandle};

#[derive(Clone, Copy)]
struct Desired { visible: bool, state: TrayState, stopped: bool }
enum Worker {
    Dormant { icon: Icon, action: ActionHandler },
    Running(JoinHandle<Result<()>>),
    Failed(Error),
    Stopped,
}

pub(super) struct Fallback {
    desired: watch::Sender<Desired>,
    worker: Mutex<Worker>,
}

impl Fallback {
    pub fn new(state: TrayState, icon: Icon, action: ActionHandler) -> Arc<Self> {
        let (desired, _) = watch::channel(Desired { visible: false, state, stopped: false });
        Arc::new(Self { desired, worker: Mutex::new(Worker::Dormant { icon, action }) })
    }

    pub fn visible(&self, visible: bool) {
        let Ok(mut worker) = self.worker.lock() else {
            tracing::error!("XEmbed 托盘线程状态锁损坏"); return;
        };
        if matches!(*worker, Worker::Stopped) { return; }
        self.desired.send_modify(|desired| desired.visible = visible);
        if !visible || !matches!(*worker, Worker::Dormant { .. }) { return; }
        let Worker::Dormant { icon, action } = std::mem::replace(&mut *worker, Worker::Stopped) else { unreachable!() };
        let desired = self.desired.subscribe();
        *worker = match std::thread::Builder::new().name("super-paste-xembed".into()).spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| run(icon, action, desired)))
                .unwrap_or_else(|_| Err(Error::Task("XEmbed 托盘线程发生异常".into())));
            if let Err(error) = &result { tracing::error!(%error, "XEmbed 托盘后端停止"); }
            result
        }) {
            Ok(worker) => Worker::Running(worker),
            Err(error) => {
                let error = Error::Platform(format!("启动 XEmbed 托盘线程失败：{error}").into());
                tracing::error!(%error);
                Worker::Failed(error)
            },
        };
    }

    pub fn refresh(&self, state: TrayState) {
        // A single latest-value slot also supplies the GTK task's wakeup. No
        // unbounded command queue or periodic GLib/Tokio polling is needed.
        self.desired.send_modify(|desired| desired.state = state);
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.desired.send_modify(|desired| { desired.visible = false; desired.stopped = true; });
        let worker = {
            let mut worker = self.worker.lock().map_err(|_| Error::Task("XEmbed 托盘线程状态锁损坏".into()))?;
            std::mem::replace(&mut *worker, Worker::Stopped)
        };
        match worker {
            Worker::Running(worker) => tokio::task::spawn_blocking(move || worker.join()
                .unwrap_or_else(|_| Err(Error::Task("XEmbed 托盘线程未正常结束".into())))).await?,
            Worker::Failed(error) => Err(error),
            _ => Ok(()),
        }
    }
}

impl Drop for Fallback {
    fn drop(&mut self) { self.desired.send_modify(|desired| { desired.visible = false; desired.stopped = true; }); }
}

fn run(icon: Icon, action: ActionHandler, mut desired: watch::Receiver<Desired>) -> Result<()> {
    if desired.borrow().stopped { return Ok(()); }
    // GTK/GDK use the global default GLib context. GPUI does not pump it. Keep
    // its ownership on this thread through initialization, callbacks and drop.
    let context = glib::MainContext::default();
    let _guard = context.acquire().map_err(|error| Error::Platform(format!("无法取得 GTK 事件上下文：{error}").into()))?;
    if gtk::is_initialized() {
        return Err(Error::Platform("GTK 已由另一线程初始化，无法接管 XEmbed 托盘".into()));
    }
    // Restrict GTK rather than mutating process environment after threads have
    // started. This cannot select a Wayland display behind the XEmbed owner.
    gdk::set_allowed_backends("x11");
    gtk::init().map_err(|error| Error::Platform(format!("初始化 XEmbed 托盘失败：{error}").into()))?;
    let display = gdk::Display::default().ok_or_else(|| Error::Platform("GTK 没有可用的 X11 显示连接".into()))?;
    if !display.type_().name().contains("X11") {
        return Err(Error::Platform("XEmbed 托盘要求 X11 显示后端".into()));
    }
    let initial = *desired.borrow_and_update();
    if initial.stopped { return Ok(()); }
    let tray = NativeTray::new(icon, action, initial.state);
    let main_loop = glib::MainLoop::new(Some(&context), false);
    let finished = main_loop.clone();
    let task = context.spawn_local(async move {
        let result = AssertUnwindSafe(async move {
            loop {
                let projection = *desired.borrow_and_update();
                if projection.stopped { break; }
                tray.project(projection);
                if desired.changed().await.is_err() { break; }
            }
            // Destroy menu/icon on their GTK thread before releasing the loop.
            drop(tray);
        }).catch_unwind().await.map_err(|_| Error::Task("XEmbed 状态投影任务发生异常".into()));
        finished.quit();
        result
    });
    main_loop.run();
    // Observe an unexpected async callback panic rather than reporting success.
    context.block_on(task).map_err(|error| Error::Task(format!("XEmbed 事件任务异常：{error}").into()))??;
    Ok(())
}

// gtk-rs intentionally omits this deprecated class from the safe API; it is
// precisely the legacy XEmbed class we need. Keep the binding and popup FFI
// here; GLib still owns references, signal values and callback destruction.
glib::wrapper! {
    struct StatusIcon(Object<gtk::ffi::GtkStatusIcon, gtk::ffi::GtkStatusIconClass>);
    match fn { type_ => || gtk::ffi::gtk_status_icon_get_type(), }
}

struct NativeTray {
    icon: StatusIcon,
    menu: gtk::Menu,
    checks: [(gtk::CheckMenuItem, glib::SignalHandlerId); 5],
}

impl NativeTray {
    fn new(icon: Icon, action: ActionHandler, state: TrayState) -> Self {
        let bytes = glib::Bytes::from_owned(icon.rgba);
        let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(&bytes, gdk_pixbuf::Colorspace::Rgb, true, 8,
            icon.width as i32, icon.height as i32, icon.width as i32 * 4);
        let icon: StatusIcon = glib::Object::builder().property("visible", false)
            .property("pixbuf", &pixbuf).property("title", TITLE).property("tooltip-text", TOOLTIP).build();
        // SAFETY: this owned object has the GtkStatusIcon runtime type, and
        // the nul-terminated application name outlives this synchronous call.
        unsafe { gtk::ffi::gtk_status_icon_set_name(icon.as_ptr(), c"super-paste".as_ptr()); }
        let menu = gtk::Menu::new();
        menu.append(&item("设置", TrayAction::ShowSettings { activation_token: None }, &action));
        menu.append(&item("粘贴面板", TrayAction::ToggleOverlay { activation_token: None }, &action));
        let checks = [
            check("跟随系统", state.color_mode == ColorMode::System, TrayAction::SetColorMode(ColorMode::System), &action),
            check("亮色模式", state.color_mode == ColorMode::Light, TrayAction::SetColorMode(ColorMode::Light), &action),
            check("暗黑模式", state.color_mode == ColorMode::Dark, TrayAction::SetColorMode(ColorMode::Dark), &action),
            check("开机自启", state.auto_start, TrayAction::ToggleAutoStart, &action),
            check("自动 OCR", state.enable_local_ocr, TrayAction::ToggleLocalOcr, &action),
        ];
        let colors = gtk::Menu::new();
        for (item, _) in &checks[..3] { colors.append(item); }
        let theme = gtk::MenuItem::with_label("颜色模式"); theme.set_submenu(Some(&colors)); menu.append(&theme);
        menu.append(&gtk::SeparatorMenuItem::new());
        for (item, _) in &checks[3..] { menu.append(item); }
        menu.append(&gtk::SeparatorMenuItem::new());
        menu.append(&item("重启应用", TrayAction::Restart, &action));
        menu.append(&item("退出", TrayAction::Quit, &action));
        menu.show_all();

        let popup = menu.clone(); let weak_icon = icon.downgrade();
        icon.connect_local("popup-menu", false, move |values| {
            if let Some(icon) = weak_icon.upgrade() {
                let button = values[1].get::<u32>().expect("GtkStatusIcon popup button type");
                let time = values[2].get::<u32>().expect("GtkStatusIcon popup time type");
                popup_at_icon(&popup, &icon, button, time);
            }
            None
        });
        // Suppress GTK's pointer activation and emit on release, as the old
        // application does. Keep activate for keyboard/accessibility clients.
        let skip_release = Rc::new(Cell::new(false));
        let released = skip_release.clone(); let click = action.clone();
        icon.connect_local("button-release-event", false, move |values| {
            let event = values[1].get::<gdk::Event>().expect("GtkStatusIcon event type")
                .downcast::<gdk::EventButton>().expect("GtkStatusIcon button event type");
            let handled = event.button() == 1;
            if handled && !released.replace(false) { emit(&click, TrayAction::ToggleOverlay { activation_token: None }); }
            Some(handled.to_value())
        });
        let double = action.clone();
        icon.connect_local("button-press-event", false, move |values| {
            let event = values[1].get::<gdk::Event>().expect("GtkStatusIcon event type")
                .downcast::<gdk::EventButton>().expect("GtkStatusIcon button event type");
            let handled = event.button() == 1;
            if handled && event.event_type() == gdk::EventType::ButtonPress { skip_release.set(false); }
            if handled && event.event_type() == gdk::EventType::DoubleButtonPress {
                skip_release.set(true); emit(&double, TrayAction::ShowSettings { activation_token: None });
            }
            Some(handled.to_value())
        });
        icon.connect_local("activate", false, move |_| { emit(&action, TrayAction::ToggleOverlay { activation_token: None }); None });
        icon.connect_notify_local(Some("embedded"), |icon, _| {
            if icon.property::<bool>("embedded") { tracing::info!("Linux XEmbed 托盘宿主已连接"); }
            else { tracing::warn!("Linux XEmbed 托盘宿主不可用，等待连接"); }
        });
        Self { icon, menu, checks }
    }

    fn project(&self, desired: Desired) {
        let values = [desired.state.color_mode == ColorMode::System, desired.state.color_mode == ColorMode::Light,
            desired.state.color_mode == ColorMode::Dark, desired.state.auto_start, desired.state.enable_local_ocr];
        // gtk_check_menu_item_set_active can emit activate. Projection and
        // rollback must never submit another application mutation.
        for ((item, signal), checked) in self.checks.iter().zip(values) {
            item.block_signal(signal); item.set_active(checked); item.unblock_signal(signal);
        }
        if !desired.visible { self.menu.popdown(); }
        self.icon.set_property("visible", desired.visible);
    }
}

impl Drop for NativeTray {
    fn drop(&mut self) { self.menu.popdown(); self.icon.set_property("visible", false); }
}

fn emit(handler: &ActionHandler, action: TrayAction) {
    // A native signal trampoline must not unwind through GLib's C frames.
    if std::panic::catch_unwind(AssertUnwindSafe(|| handler(action))).is_err() {
        tracing::error!("XEmbed 托盘动作回调发生异常");
    }
}
fn item(label: &str, action: TrayAction, handler: &ActionHandler) -> gtk::MenuItem {
    let item = gtk::MenuItem::with_label(label); let handler = handler.clone();
    item.connect_activate(move |_| emit(&handler, action.clone())); item
}
fn check(label: &str, checked: bool, action: TrayAction, handler: &ActionHandler) -> (gtk::CheckMenuItem, glib::SignalHandlerId) {
    let item = gtk::CheckMenuItem::with_label(label); item.set_active(checked); let handler = handler.clone();
    let signal = item.connect_activate(move |_| emit(&handler, action.clone())); (item, signal)
}

fn popup_at_icon(menu: &gtk::Menu, icon: &StatusIcon, button: u32, time: u32) {
    unsafe extern "C" fn position(menu: *mut gtk::ffi::GtkMenu, x: *mut i32, y: *mut i32,
        push_in: *mut glib::ffi::gboolean, data: glib::ffi::gpointer) {
        // SAFETY: GTK invokes this synchronously/on its loop with our live
        // status-icon pointer; the owner closes the menu before dropping it.
        unsafe { gtk::ffi::gtk_status_icon_position_menu(menu, x, y, push_in, data.cast()); }
    }
    // SAFETY: both GTK objects and the positioning callback stay on this GTK
    // thread. NativeTray owns the icon for the complete popup lifetime.
    unsafe { gtk::ffi::gtk_menu_popup(menu.to_glib_none().0, std::ptr::null_mut(), std::ptr::null_mut(),
        Some(position), icon.as_ptr().cast(), button, time); }
}
