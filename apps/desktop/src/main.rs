#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
extern crate gpui_kit as gpui;

mod assets;
mod diagnostics;
mod effects;
mod immediate_configuration;
mod host;
mod services;
mod ui;

use gpui_kit::*;
use services::Services;
use super_paste_core::Error;
use super_paste_platform::main_thread::MainThread;
use super_paste_platform::{instance::{Activation, SingleInstance, InstanceRole, LaunchRequest}, paths::AppPaths};
use tokio::sync::{oneshot, watch};
use tracing_subscriber::prelude::*;

fn main() {
    if let Err(error) = run() {
        tracing::error!(%error, "应用停止");
        eprintln!("Super Paste: {error}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    if !super_paste_platform::restart::accept_handoff()? { return Ok(()); }
    let log = diagnostics::EventLog::new();
    tracing_subscriber::registry().with(tracing_subscriber::filter::LevelFilter::INFO)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr)).with(log.layer()).try_init()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all()
        .worker_threads(2).thread_name("super-paste")
        .thread_keep_alive(std::time::Duration::from_secs(10)).build()?;
    let handle = runtime.handle().clone();
    let paths = AppPaths::discover()?;
    let launch = LaunchRequest::from_environment()?;
    let InstanceRole::Primary { mut instance, activations, sender } = runtime.block_on(SingleInstance::acquire(&paths, launch.clone(), &handle))? else { return Ok(()); };
    let reopen_sender = sender.clone();
    let restart = super_paste_platform::restart::Restart::new();
    let desktop_restart = restart.clone();
    let (main_thread, main_jobs) = MainThread::channel();
    let foreground_jobs = main_jobs.clone();
    let (ready, ready_rx) = oneshot::channel();
    let (boot, boot_rx) = watch::channel(None);
    let boot_runtime = handle.clone();
    // The runtime owns startup; closing a window never cancels accepted I/O.
    let startup = runtime.spawn(async move {
        ready_rx.await.map_err(|_| Error::Task("桌面初始化已取消".into()))?;
        let result = Services::start(boot_runtime, main_thread, paths, sender, log).await;
        boot.send_replace(Some(result.clone()));
        result
    });
    let application = gpui_kit::application().with_assets(assets::Assets).with_quit_mode(QuitMode::Explicit);
    // On macOS this selects Accessory before AppKit launches; the graphical
    // platform stays available for the overlay and later settings activation.
    #[cfg(target_os = "macos")]
    let application = application.with_windowing(WindowingRequest::Headless);
    if cfg!(target_os = "macos") {
        application.on_reopen(move |_| {
            // The existing activation queue also retains requests that arrive
            // before service startup; no Desktop global is needed here.
            if let Err(error) = reopen_sender.try_send(Activation::ShowSettings) {
                tracing::warn!(%error, "应用重新激活失败");
            }
        });
    }
    application.run(move |cx| {
            gpui_kit::init(cx);
            ui::widgets::init(cx);
            ui::theme::initialize(cx);
            cx.spawn(async move |_| {
                while let Some(job) = foreground_jobs.recv().await { job(); }
            }).detach();
            host::install(handle, boot_rx, desktop_restart, launch, activations, cx);
            let _ = ready.send(());
        });
    // GPUI has a bounded native-quit hook. Finish the owned task even when that
    // hook's deadline expires, rather than abandoning a database/configuration commit.
    let shutdown = runtime.block_on(async {
        let shutdown = async {
            match startup.await { Ok(Ok(services)) => services.shutdown().await, Ok(Err(error)) => Err(error), Err(error) => Err(Error::from(error)) }
        };
        tokio::pin!(shutdown);
        loop {
            tokio::select! { biased;
                result = &mut shutdown => break result,
                job = main_jobs.recv() => match job { Some(job) => job(), None => break shutdown.await },
            }
        }
    });
    // The replacement waits for EOF. Release the old instance's listener and
    // lock before granting that handoff, after all accepted native work drains.
    let instance_shutdown = runtime.block_on(instance.shutdown());
    runtime.block_on(restart.release_after_shutdown());
    let errors = [shutdown, instance_shutdown].into_iter().filter_map(Result::err).map(|error| error.to_string()).collect::<Vec<_>>();
    if !errors.is_empty() { return Err(anyhow::anyhow!(errors.join("；"))); }
    Ok(())
}
