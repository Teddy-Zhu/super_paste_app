use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::app::state::AppState;
use crate::error::AppError;
use crate::result::Result;
use crate::system::overlay;

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
        let code = match s.to_uppercase().as_str() {
            "A" => Code::KeyA,
            "B" => Code::KeyB,
            "C" => Code::KeyC,
            "D" => Code::KeyD,
            "E" => Code::KeyE,
            "F" => Code::KeyF,
            "G" => Code::KeyG,
            "H" => Code::KeyH,
            "I" => Code::KeyI,
            "J" => Code::KeyJ,
            "K" => Code::KeyK,
            "L" => Code::KeyL,
            "M" => Code::KeyM,
            "N" => Code::KeyN,
            "O" => Code::KeyO,
            "P" => Code::KeyP,
            "Q" => Code::KeyQ,
            "R" => Code::KeyR,
            "S" => Code::KeyS,
            "T" => Code::KeyT,
            "U" => Code::KeyU,
            "V" => Code::KeyV,
            "W" => Code::KeyW,
            "X" => Code::KeyX,
            "Y" => Code::KeyY,
            "Z" => Code::KeyZ,
            "0" => Code::Digit0,
            "1" => Code::Digit1,
            "2" => Code::Digit2,
            "3" => Code::Digit3,
            "4" => Code::Digit4,
            "5" => Code::Digit5,
            "6" => Code::Digit6,
            "7" => Code::Digit7,
            "8" => Code::Digit8,
            "9" => Code::Digit9,
            "F1" => Code::F1,
            "F2" => Code::F2,
            "F3" => Code::F3,
            "F4" => Code::F4,
            "F5" => Code::F5,
            "F6" => Code::F6,
            "F7" => Code::F7,
            "F8" => Code::F8,
            "F9" => Code::F9,
            "F10" => Code::F10,
            "F11" => Code::F11,
            "F12" => Code::F12,
            "SPACE" => Code::Space,
            "ENTER" | "RETURN" => Code::Enter,
            "TAB" => Code::Tab,
            _ => return Err(AppError::config(format!("不支持的按键: {}", s))),
        };
        Ok(code)
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
                    // 直接调用共享 toggle 逻辑，不再走 emit 事件往返
                    let position = app
                        .try_state::<AppState>()
                        .map(|s| s.config.get().overlay_position)
                        .unwrap_or_else(|| "bottom".to_string());
                    if let Err(e) = overlay::toggle_overlay(app, &position) {
                        tracing::warn!("快捷键切换 Overlay 失败: {:?}", e);
                    }
                }
            })
            .map_err(|e| AppError::config(format!("绑定快捷键回调失败: {}", e)))?;

        Ok(())
    }
}
