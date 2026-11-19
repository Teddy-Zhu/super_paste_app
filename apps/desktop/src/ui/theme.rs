use gpui_kit::*;
use gpui_kit::component::{Theme, ThemeMode};
use super_paste_format::ColorMode;
use super::palette::Palette;

struct ThemeState { applied: Option<bool> }
impl Global for ThemeState {}

pub fn initialize(cx: &mut App) {
    let fonts = cx.text_system().all_font_names();
    let preferred = |names: &[&str]| -> Option<SharedString> {
        names.iter().find_map(|name| fonts.iter().find(|font| font.eq_ignore_ascii_case(name)).cloned().map(Into::into))
    };
    let font = if cfg!(target_os = "macos") { None }
        else if cfg!(target_os = "linux") { preferred(&["Noto Sans CJK SC", "Sarasa UI SC", "Noto Sans"]) }
        else { preferred(&["Segoe UI", "Microsoft YaHei UI", "Noto Sans"]) };
    let mono = preferred(&["SF Mono", "Monaco", "Menlo", "Ubuntu Mono"]).unwrap_or_else(|| {
        // Match the reference CSS stack's browser fallback, rather than Kit's
        // editor-oriented default. Linux resolves the generic via fontconfig.
        if cfg!(target_os = "windows") { "Courier New".into() }
        else if cfg!(target_os = "macos") { "Menlo".into() }
        else { "monospace".into() }
    });
    Theme::update(cx, |theme| {
        if let Some(font) = font { theme.font_family = font; }
        theme.mono_font_family = mono;
        theme.font_size = px(13.0);
    });
    cx.set_global(ThemeState { applied: None });
    apply(ColorMode::System, cx.window_appearance(), cx);
}

pub fn apply(mode: ColorMode, appearance: WindowAppearance, cx: &mut App) {
    let dark = Palette::uses_dark(mode, appearance);
    if cx.global::<ThemeState>().applied == Some(dark) { return; }
    let target = if dark { ThemeMode::Dark } else { ThemeMode::Light };
    if Theme::global(cx).mode != target { Theme::change(target, None, cx); }
    let p = Palette::new(dark);
    Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        c.background = p.bg_primary;
        c.foreground = p.text_primary;
        c.border = p.border_medium;
        c.input = p.control_border;
        c.ring = p.primary;
        c.caret = p.primary;
        c.primary = p.primary;
        c.primary_hover = p.primary_hover;
        c.primary_active = p.primary_active;
        c.primary_foreground = p.text_inverse;
        c.button_primary = p.primary;
        c.button_primary_hover = p.primary_hover;
        c.button_primary_active = p.primary_active;
        c.button_primary_foreground = p.text_inverse;
        c.button = p.bg_primary;
        c.button_foreground = p.text_primary;
        c.button_hover = p.control_hover;
        c.button_active = p.control_active;
        c.secondary = p.bg_secondary;
        c.secondary_hover = p.control_hover;
        c.secondary_active = p.control_active;
        c.secondary_foreground = p.text_primary;
        c.button_secondary = p.bg_secondary;
        c.button_secondary_hover = p.control_hover;
        c.button_secondary_active = p.control_active;
        c.button_secondary_foreground = p.text_primary;
        c.muted = p.bg_secondary;
        c.muted_foreground = p.text_secondary;
        c.popover = p.bg_elevated;
        c.popover_foreground = p.text_primary;
        c.accent = p.selection;
        c.accent_foreground = p.primary;
        c.selection = p.selection;
        c.list = p.bg_primary;
        c.list_hover = p.control_hover;
        c.list_active = p.selection;
        c.list_active_border = p.border_medium;
        c.sidebar = p.sidebar;
        c.sidebar_foreground = p.text_secondary;
        c.sidebar_border = p.border_medium;
        c.sidebar_primary = p.primary;
        c.sidebar_primary_foreground = p.text_inverse;
        c.sidebar_accent = p.selection;
        c.sidebar_accent_foreground = p.primary;
        c.link = p.primary;
        c.link_hover = p.primary_hover;
        c.link_active = p.primary_active;
        c.danger = p.danger;
        c.danger_hover = p.danger;
        c.danger_active = p.danger;
        c.danger_foreground = p.text_inverse;
        c.button_danger = p.danger_bg;
        c.button_danger_hover = p.danger_bg;
        c.button_danger_active = p.danger_border;
        c.button_danger_foreground = p.danger;
        c.success = p.success;
        c.success_foreground = p.success_text;
        c.warning = p.warning;
        c.warning_foreground = p.warning_text;
        c.info = p.info;
        c.info_foreground = p.text_inverse;
        c.scrollbar_thumb = p.scrollbar;
        c.scrollbar_thumb_hover = p.scrollbar_hover;
        c.switch = p.toggle_off;
        c.switch_thumb = p.bg_primary;
        c.progress_bar = p.primary;
        c.slider_bar = p.primary;
        c.slider_thumb = rgb(0xffffff).into();
        c.tab = p.bg_secondary;
        c.tab_active = p.selection;
        c.tab_active_foreground = p.primary;
        c.tab_foreground = p.text_secondary;
        c.tab_bar = p.bg_secondary;
    });
    cx.global_mut::<ThemeState>().applied = Some(dark);
}
