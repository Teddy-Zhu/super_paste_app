mod commands;
mod config;
mod core;
pub mod credentials;
mod diagnostics;
mod events;
mod mcp;
mod ocr;
mod platform;
mod runtime;
mod storage;
mod sync;
mod tray;

use runtime::{Runtime, RuntimeHolder};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct StartupActivation {
    requested: AtomicBool,
    overlay_requested: AtomicBool,
    ready: AtomicBool,
}

fn activate(app: &tauri::AppHandle) {
    let startup = app.state::<StartupActivation>();
    startup.requested.store(true, Ordering::Release);
    if startup.ready.load(Ordering::Acquire)
        && let Ok(runtime) = app.state::<RuntimeHolder>().get()
    {
        startup.requested.store(false, Ordering::Release);
        tauri::async_runtime::spawn(async move {
            if let Err(error) = runtime.windows.open_main().await {
                tracing::error!(%error,"application activation failed");
            }
        });
    }
}

fn toggle_overlay(app: &tauri::AppHandle) {
    let startup = app.state::<StartupActivation>();
    startup.overlay_requested.fetch_xor(true, Ordering::AcqRel);
    if startup.ready.load(Ordering::Acquire)
        && startup.overlay_requested.swap(false, Ordering::AcqRel)
        && let Ok(runtime) = app.state::<RuntimeHolder>().get()
    {
        tauri::async_runtime::spawn(async move {
            if let Err(error) = runtime.windows.toggle(None).await {
                tracing::error!(%error,"command line overlay activation failed");
            }
        });
    }
}

pub fn run() {
    let diagnostics = diagnostics::Diagnostics::install().expect("无法初始化应用日志");
    let holder = RuntimeHolder::default();
    let autostart = tauri_plugin_autostart::Builder::new()
        .app_name("Super Paste")
        .args(["--background"]);
    #[cfg(target_os = "macos")]
    let autostart = autostart.macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent);
    let app=tauri::Builder::default()
        .manage(holder.clone())
        .manage(StartupActivation::default())
        .plugin(tauri_plugin_single_instance::init(|app,args,_cwd|{if args.iter().any(|arg|arg=="--toggle-overlay"){toggle_overlay(app);}else if !args.iter().any(|arg|arg=="--background"){activate(app);}}))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(autostart.build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(|app,_,event|{
            if event.state==tauri_plugin_global_shortcut::ShortcutState::Pressed&&let Ok(runtime)=app.state::<RuntimeHolder>().get(){
                tauri::async_runtime::spawn(async move{if let Err(error)=runtime.windows.toggle(None).await{tracing::error!(%error,"global shortcut activation failed");}});
            }
        }).build())
        .invoke_handler(commands::handler())
        .setup(move|app|{
            #[cfg(target_os="macos")]
            app.handle().set_activation_policy(tauri::ActivationPolicy::Accessory)?;
            let handle=app.handle().clone();
            tauri::async_runtime::spawn(async move{
                match Runtime::initialize(handle.clone(),holder,diagnostics).await{
                    Ok(runtime)=>{
                        let activation=handle.state::<StartupActivation>();
                        activation.ready.store(true,Ordering::Release);
                        if activation.requested.swap(false,Ordering::AcqRel)&&let Err(error)=runtime.windows.open_main().await{tracing::error!(%error,"deferred activation failed");}
                        if activation.overlay_requested.swap(false,Ordering::AcqRel)&&let Err(error)=runtime.windows.toggle(None).await{tracing::error!(%error,"deferred overlay activation failed");}
                        let stop=runtime.stop.clone();
                        tauri::async_runtime::spawn(async move{tokio::select!{
                            _=stop.cancelled()=>{},
                            result=tokio::signal::ctrl_c()=>match result{Ok(())=>runtime.shutdown(false).await,Err(error)=>tracing::warn!(%error,"interrupt handler unavailable")},
                        }});
                    },
                    Err(error)=>{
                        tracing::error!(%error,"application initialization failed");
                        let exiting=handle.clone();
                        handle.dialog().message(format!("无法启动 Super Paste：\n{error}")).title("Super Paste").kind(tauri_plugin_dialog::MessageDialogKind::Error).show(move|_|exiting.exit(1));
                    },
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("无法创建 Tauri 应用");
    app.run(|handle, event| match event {
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            if let Ok(runtime) = handle.state::<RuntimeHolder>().get() {
                if !runtime.is_quitting() && code != Some(1) {
                    api.prevent_exit();
                    tauri::async_runtime::spawn(async move {
                        runtime.shutdown(false).await;
                    });
                }
            } else if code.is_none() {
                api.prevent_exit();
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => activate(handle),
        _ => {}
    });
}
