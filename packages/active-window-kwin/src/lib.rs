use napi::{Env, Error, Result, Task, bindgen_prelude::AsyncTask};
use napi_derive::napi;

#[cfg(target_os = "linux")]
mod kwin;

#[napi(object)]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[napi(object)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub application: String,
    pub window_class: String,
    pub pid: u32,
    pub path: Option<String>,
    pub bounds: WindowBounds,
}

pub struct ActiveWindowTask;

impl Task for ActiveWindowTask {
    type Output = Option<WindowInfo>;
    type JsValue = Option<WindowInfo>;

    fn compute(&mut self) -> Result<Self::Output> {
        #[cfg(target_os = "linux")]
        {
            kwin::get_active_window().map_err(Error::from_reason)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(Error::from_reason("KWin is only available on Linux"))
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

// D-Bus calls and /proc access run on the worker pool, never the JS event loop.
#[napi]
pub fn get_active_window() -> AsyncTask<ActiveWindowTask> {
    AsyncTask::new(ActiveWindowTask)
}
