use tauri::Emitter;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::error::AppError;
use crate::result::Result;

pub struct ShortcutManager;

impl ShortcutManager {
    /// 注册全局快捷键，若首选快捷键被占用则尝试备选方案。
    pub fn register_with_fallback(handle: &tauri::AppHandle, preferred: &str) -> Result<String> {
        if let Ok(shortcut) = Self::parse_shortcut(preferred) {
            if Self::try_register(handle, &shortcut).is_ok() {
                return Ok(preferred.to_string());
            }
        }

        let fallbacks = vec!["Ctrl+Shift+V", "Ctrl+Alt+V", "Alt+Shift+V"];
        for s in fallbacks {
            if let Ok(shortcut) = Self::parse_shortcut(s) {
                if Self::try_register(handle, &shortcut).is_ok() {
                    let _ = handle.emit_to(
                        tauri::EventTarget::App,
                        "shortcut:changed",
                        s,
                    );
                    tracing::warn!("首选快捷键被占用，已切换到: {}", s);
                    return Ok(s.to_string());
                }
            }
        }

        Err(AppError::config("所有快捷键均被占用"))
    }

    fn parse_shortcut(s: &str) -> Result<Shortcut> {
        let parts: Vec<&str> = s.split('+').collect();
        let mut modifiers = Modifiers::empty();
        let mut key_code = None;

        for part in parts {
            match part.trim() {
                "Ctrl" | "Control" => modifiers |= Modifiers::CONTROL,
                "Shift" => modifiers |= Modifiers::SHIFT,
                "Alt" => modifiers |= Modifiers::ALT,
                "CmdOrCtrl" => {
                    #[cfg(target_os = "macos")]
                    {
                        modifiers |= Modifiers::SUPER;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        modifiers |= Modifiers::CONTROL;
                    }
                }
                key => key_code = Some(Self::parse_key(key)?),
            }
        }

        Ok(Shortcut::new(
            Some(modifiers),
            key_code.ok_or_else(|| AppError::config("无效快捷键"))?,
        ))
    }

    fn parse_key(s: &str) -> Result<Code> {
        match s.to_uppercase().as_str() {
            "V" => Ok(Code::KeyV),
            "C" => Ok(Code::KeyC),
            "X" => Ok(Code::KeyX),
            "A" => Ok(Code::KeyA),
            _ => Err(AppError::config(format!("不支持的按键: {}", s))),
        }
    }

    fn try_register(handle: &tauri::AppHandle, shortcut: &Shortcut) -> Result<()> {
        handle
            .global_shortcut()
            .register(*shortcut)
            .map_err(|e| AppError::config(format!("注册快捷键失败: {}", e)))?;

        handle
            .global_shortcut()
            .on_shortcut(*shortcut, |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    let _ = app.emit_to(
                        tauri::EventTarget::App,
                        "toggle-overlay",
                        (),
                    );
                }
            })
            .map_err(|e| AppError::config(format!("绑定快捷键回调失败: {}", e)))?;

        Ok(())
    }
}
