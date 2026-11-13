pub mod active_window;
pub(crate) mod active_events;
#[cfg(target_os = "linux")]
pub mod autostart;
#[cfg(target_os = "linux")]
mod kwin;
#[cfg(target_os = "linux")]
mod layer_shell;
#[cfg(target_os = "linux")]
mod modifiers;
pub mod native_frame;
pub mod ort_runtime;
pub mod power;
pub mod shortcut;
pub(crate) mod source_icon;
#[cfg(target_os = "linux")]
mod source_window_x11;
pub mod windows;
