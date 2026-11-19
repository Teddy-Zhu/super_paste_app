//! Native adapters. App-core owns business rules and never imports this crate.
pub mod clipboard;
pub mod credentials;
pub mod paths;
pub mod ort_runtime;
pub mod main_thread;
pub mod source;
pub mod source_icon;
pub mod geometry;
pub mod autostart;
pub mod restart;
pub mod instance;
pub mod shortcut;
pub mod access;
pub mod tray;
mod active_events;
mod latest;
#[cfg(target_os = "linux")]
mod source_window_x11;
#[cfg(target_os = "linux")]
mod geometry_x11;
#[cfg(target_os = "linux")]
mod kwin;
