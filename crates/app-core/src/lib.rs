//! Background business services. This crate must not depend on a GUI or window handle.
pub mod error;
pub mod configuration;
pub mod credentials;
pub mod clipboard;
pub mod events;
pub mod images;
pub mod qr;
pub mod preview;
pub mod ocr;
mod resources;
pub mod storage;
pub mod tags;
pub mod statistics;
pub mod calendar;
pub mod sync;
pub mod mcp;

pub use error::{Error, Result};

pub fn now_ms() -> Result<i64> {
    let duration = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Task("系统时间早于 Unix 起点".into()))?;
    i64::try_from(duration.as_millis()).map_err(|_| Error::Task("系统时间超出支持范围".into()))
}

