use super::*;
use ksni::{TrayMethods, menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu}};
mod xembed;

pub(super) struct Backend { handle: ksni::Handle<LinuxTray>, fallback: Option<Arc<xembed::Fallback>> }
impl Backend {
    pub async fn create(_main: MainThread, state: TrayState, icon: Icon, action: ActionHandler) -> Result<Self> {
        let argb = icon.rgba.chunks_exact(4).flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]]).collect();
        let (width, height) = (icon.width as i32, icon.height as i32);
        // GPUI prefers Wayland when WAYLAND_DISPLAY is present. Do not create
        // a second, unusable XWayland tray on that session's missing SNI host.
        let populated = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let fallback = (populated("DISPLAY") && !populated("WAYLAND_DISPLAY"))
            .then(|| xembed::Fallback::new(state, icon, action.clone()));
        let tray = LinuxTray { state, action, token: None, fallback: fallback.clone(),
            icon: ksni::Icon { width, height, data: argb } };
        // Autostart can precede the desktop watcher. ksni owns reconnecting without polling.
        let handle = match tray.assume_sni_available(true).spawn().await {
            Ok(handle) => handle,
            Err(error) => {
                if let Some(fallback) = &fallback && let Err(release) = fallback.shutdown().await {
                    tracing::error!(%release, "释放未启动的托盘后端失败");
                }
                return Err(Error::Platform(format!("创建 Linux 托盘失败：{error}").into()));
            },
        };
        Ok(Self { handle, fallback })
    }
    pub async fn refresh(&mut self, state: TrayState) -> Result<()> {
        self.handle.update(move |tray| tray.state = state).await.ok_or_else(|| Error::Platform("Linux 托盘连接已结束".into()))?;
        if let Some(fallback) = &self.fallback { fallback.refresh(state); }
        Ok(())
    }
    pub async fn shutdown(self) -> Result<()> {
        self.handle.shutdown().await;
        if let Some(fallback) = self.fallback { fallback.shutdown().await?; }
        Ok(())
    }
}
struct LinuxTray { state: TrayState, action: ActionHandler, icon: ksni::Icon, token: Option<String>, fallback: Option<Arc<xembed::Fallback>> }
impl LinuxTray {
    fn emit(&mut self, mut action: TrayAction) {
        let token = self.token.take();
        match &mut action {
            TrayAction::ShowSettings { activation_token } | TrayAction::ToggleOverlay { activation_token } => *activation_token = token,
            _ => {},
        }
        (self.action)(action);
    }
    fn item(label: &str, action: TrayAction) -> MenuItem<Self> {
        StandardItem::<Self> { label: label.into(), activate: Box::new(move |tray| tray.emit(action.clone())), ..Default::default() }.into()
    }
    fn check(label: &str, checked: bool, action: TrayAction) -> MenuItem<Self> {
        CheckmarkItem::<Self> { label: label.into(), checked, activate: Box::new(move |tray| tray.emit(action.clone())), ..Default::default() }.into()
    }
}
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String { "com.v2dawn.superpaste.rs".into() }
    fn title(&self) -> String { TITLE.into() }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> { vec![self.icon.clone()] }
    fn tool_tip(&self) -> ksni::ToolTip { ksni::ToolTip { title: TOOLTIP.into(), ..Default::default() } }
    fn provide_xdg_activation_token(&mut self, token: String) { self.token = (!token.is_empty() && token.len() <= 4096).then_some(token); }
    fn activate(&mut self, _x: i32, _y: i32) { self.emit(TrayAction::ToggleOverlay { activation_token: None }); }
    fn watcher_online(&self) {
        if let Some(fallback) = &self.fallback { fallback.visible(false); }
        tracing::info!("Linux SNI 托盘宿主已连接");
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        if let Some(fallback) = &self.fallback { fallback.visible(true); }
        tracing::warn!(?reason, "Linux SNI 托盘宿主不可用，等待连接"); true
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            Self::item("设置", TrayAction::ShowSettings { activation_token: None }),
            Self::item("粘贴面板", TrayAction::ToggleOverlay { activation_token: None }),
            SubMenu { label: "颜色模式".into(), submenu: vec![
                Self::check("跟随系统", self.state.color_mode == ColorMode::System, TrayAction::SetColorMode(ColorMode::System)),
                Self::check("亮色模式", self.state.color_mode == ColorMode::Light, TrayAction::SetColorMode(ColorMode::Light)),
                Self::check("暗黑模式", self.state.color_mode == ColorMode::Dark, TrayAction::SetColorMode(ColorMode::Dark)),
            ], ..Default::default() }.into(),
            MenuItem::Separator,
            Self::check("开机自启", self.state.auto_start, TrayAction::ToggleAutoStart),
            Self::check("自动 OCR", self.state.enable_local_ocr, TrayAction::ToggleLocalOcr),
            MenuItem::Separator,
            Self::item("重启应用", TrayAction::Restart),
            Self::item("退出", TrayAction::Quit),
        ]
    }
}
