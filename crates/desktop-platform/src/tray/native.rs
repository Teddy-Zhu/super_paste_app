use super::*;
use std::{cell::RefCell, sync::{Mutex, Once, Weak}};
use tokio::sync::oneshot;
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu}};

const TRAY_ID: &str = "com.v2dawn.superpaste.rs.tray";
const PREFIX: &str = "superpaste-rs-tray-";
thread_local! { static NATIVE: RefCell<Option<NativeTray>> = const { RefCell::new(None) }; }
// The libraries install global handlers once. Dispatch through a weak current owner so
// shutdown/recreation neither retains the application nor routes to a stale instance.
static HANDLERS: Once = Once::new();
static TARGET: Mutex<Option<Weak<Dispatch>>> = Mutex::new(None);
struct Dispatch { action: ActionHandler }
struct NativeTray { _icon: TrayIcon, checks: [CheckMenuItem; 5], _dispatch: Arc<Dispatch> }

pub(super) struct Backend { main: MainThread }
impl Backend {
    pub async fn create(main: MainThread, state: TrayState, icon: Icon, action: ActionHandler) -> Result<Self> {
        on_main(&main, move || NATIVE.with_borrow_mut(|slot| {
            if slot.is_some() { return Err(Error::Conflict("原生托盘已经存在".into())); }
            *slot = Some(NativeTray::create(state, icon, action)?);
            Ok(())
        })).await?;
        Ok(Self { main })
    }
    pub async fn refresh(&mut self, state: TrayState) -> Result<()> {
        on_main(&self.main, move || NATIVE.with_borrow(|slot| {
            let tray = slot.as_ref().ok_or_else(|| Error::Platform("原生托盘不存在".into()))?;
            let values = [state.color_mode == ColorMode::System, state.color_mode == ColorMode::Light,
                state.color_mode == ColorMode::Dark, state.auto_start, state.enable_local_ocr];
            for (item, checked) in tray.checks.iter().zip(values) { item.set_checked(checked); }
            Ok(())
        })).await
    }
    pub async fn shutdown(self) -> Result<()> {
        on_main(&self.main, || { NATIVE.with_borrow_mut(|slot| drop(slot.take())); Ok(()) }).await
    }
}
async fn on_main(main: &MainThread, job: impl FnOnce() -> Result<()> + Send + 'static) -> Result<()> {
    let (sent, response) = oneshot::channel();
    main.run_on_main_thread_wait(move || { let _ = sent.send(job()); }).await?;
    response.await.map_err(|_| Error::Task("托盘主线程操作未完成".into()))?
}
fn emit(action: TrayAction) {
    let target = TARGET.lock().unwrap_or_else(|poison| poison.into_inner()).as_ref().and_then(Weak::upgrade);
    if let Some(target) = target { (target.action)(action); }
}
fn menu_action(id: &str) -> Option<TrayAction> {
    Some(match id.strip_prefix(PREFIX)? {
        "settings" => TrayAction::ShowSettings { activation_token: None },
        "overlay" => TrayAction::ToggleOverlay { activation_token: None },
        "system" => TrayAction::SetColorMode(ColorMode::System),
        "light" => TrayAction::SetColorMode(ColorMode::Light),
        "dark" => TrayAction::SetColorMode(ColorMode::Dark),
        "autostart" => TrayAction::ToggleAutoStart,
        "ocr" => TrayAction::ToggleLocalOcr,
        "restart" => TrayAction::Restart,
        "quit" => TrayAction::Quit,
        _ => return None,
    })
}
impl NativeTray {
    fn create(state: TrayState, icon: Icon, action: ActionHandler) -> Result<Self> {
        HANDLERS.call_once(|| {
            MenuEvent::set_event_handler(Some(|event: MenuEvent| {
                if let Some(action) = menu_action(event.id().as_ref()) { emit(action); }
            }));
            TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
                if event.id().as_ref() != TRAY_ID { return; }
                match event {
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =>
                        emit(TrayAction::ToggleOverlay { activation_token: None }),
                    TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } =>
                        emit(TrayAction::ShowSettings { activation_token: None }),
                    _ => {},
                }
            }));
        });
        let id = |suffix: &str| format!("{PREFIX}{suffix}");
        let check = |suffix: &str, label: &str, checked: bool| CheckMenuItem::with_id(id(suffix), label, true, checked, None);
        let checks = [
            check("system", "跟随系统", state.color_mode == ColorMode::System),
            check("light", "亮色模式", state.color_mode == ColorMode::Light),
            check("dark", "暗黑模式", state.color_mode == ColorMode::Dark),
            check("autostart", "开机自启", state.auto_start), check("ocr", "自动 OCR", state.enable_local_ocr),
        ];
        let settings = MenuItem::with_id(id("settings"), "设置", true, None);
        let overlay = MenuItem::with_id(id("overlay"), "粘贴面板", true, None);
        let theme = Submenu::with_items("颜色模式", true, &[&checks[0], &checks[1], &checks[2]])
            .map_err(native_error)?;
        let restart = MenuItem::with_id(id("restart"), "重启应用", true, None);
        let quit = MenuItem::with_id(id("quit"), "退出", true, None);
        let menu = Menu::with_items(&[&settings, &overlay, &theme, &PredefinedMenuItem::separator(),
            &checks[3], &checks[4], &PredefinedMenuItem::separator(), &restart, &quit]).map_err(native_error)?;
        let icon = tray_icon::Icon::from_rgba(icon.rgba, icon.width, icon.height).map_err(native_error)?;
        let builder = TrayIconBuilder::new().with_id(TRAY_ID).with_menu(Box::new(menu))
            .with_tooltip(TOOLTIP).with_menu_on_left_click(false);
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(icon);
        #[cfg(target_os = "windows")]
        let builder = builder.with_icon(icon);
        let dispatch = Arc::new(Dispatch { action });
        *TARGET.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(Arc::downgrade(&dispatch));
        let tray = builder.build().map_err(native_error)?;
        Ok(Self { _icon: tray, checks, _dispatch: dispatch })
    }
}
fn native_error(error: impl std::fmt::Display) -> Error { Error::Platform(format!("原生托盘操作失败：{error}").into()) }
