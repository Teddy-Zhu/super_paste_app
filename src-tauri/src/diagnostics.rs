use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri_plugin_dialog::DialogExt;
use tracing_subscriber::{Layer, layer::Context, prelude::*};

#[derive(Clone)]
pub struct Diagnostics {
    events: Arc<Mutex<VecDeque<Value>>>,
}

impl Diagnostics {
    pub fn install() -> Result<Self, String> {
        let diagnostics = Self {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
        };
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .with(diagnostics.clone())
            .try_init()
            .map_err(|e| e.to_string())?;
        Ok(diagnostics)
    }

    pub fn recent_events(&self) -> Result<Value, String> {
        self.events
            .lock()
            .map(|events| json!(events.iter().collect::<Vec<_>>()))
            .map_err(|_| "diagnostic log lock poisoned".into())
    }

    pub fn memory() -> Result<Value, String> {
        let pid = sysinfo::get_current_pid().map_err(|e| e.to_string())?;
        let mut system =
            sysinfo::System::new().map_err(|error| format!("无法初始化进程诊断: {error}"))?;
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);
        let process = system.process(pid).ok_or("无法读取应用进程状态")?;
        Ok(
            json!({"residentBytes":process.memory(),"virtualBytes":process.virtual_memory(),"pid":pid.as_u32()}),
        )
    }

    pub async fn export(app: tauri::AppHandle, payload: Value) -> Result<Option<PathBuf>, String> {
        let file = tokio::task::spawn_blocking(move || {
            app.dialog()
                .file()
                .add_filter("诊断信息", &["json"])
                .set_file_name(format!(
                    "super-paste-diagnostics-{}.json",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ))
                .blocking_save_file()
        })
        .await
        .map_err(|e| e.to_string())?;
        let Some(file) = file else { return Ok(None) };
        let path = file.into_path().map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?;
        let output = path.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            use std::io::Write;
            let parent = output.parent().ok_or("导出路径无效")?;
            let mut temporary =
                tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            temporary.write_all(&bytes).map_err(|e| e.to_string())?;
            temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            temporary
                .persist(&output)
                .map_err(|e| e.error.to_string())?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(Some(path))
    }
}

impl<S: tracing::Subscriber> Layer<S> for Diagnostics {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let metadata = event.metadata();
        // Export event locations, never request bodies, credentials, clipboard text, or arbitrary formatted fields.
        let entry = json!({"timestamp":crate::events::now_ms(),"level":metadata.level().as_str(),"target":metadata.target(),"source":metadata.file(),"line":metadata.line()});
        if let Ok(mut events) = self.events.lock() {
            if events.len() == 1000 {
                events.pop_front();
            }
            events.push_back(entry);
        }
    }
}
