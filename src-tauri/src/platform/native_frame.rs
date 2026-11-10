use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::Manager;
use tokio::sync::oneshot;

type Reply = Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>;

#[cfg(target_os = "linux")]
pub fn configure_overlay(window: &tauri::Window) -> Result<(), String> {
    use gtk::prelude::*;

    fn apply(host: &gtk::ApplicationWindow) {
        if let Some(surface) = host.window()
            && surface.is::<gdkx11::X11Window>()
        {
            // The renderer owns the slide animation. KWin must not scale the last
            // window texture after GTK unmaps it, including on repeated openings.
            gdk::property_change(
                &surface,
                &gdk::Atom::intern("_KDE_NET_WM_SKIP_CLOSE_ANIMATION"),
                &gdk::Atom::intern("CARDINAL"),
                32,
                gdk::PropMode::Replace,
                gdk::ChangeData::ULongs(&[1]),
            );
            surface.display().flush();
        }
    }

    let host = window.gtk_window().map_err(|e| e.to_string())?;
    host.connect_realize(apply);
    apply(&host);
    Ok(())
}

fn reply(sender: &Reply, result: Result<(), String>) {
    if let Ok(mut sender) = sender.lock()
        && let Some(sender) = sender.take()
    {
        let _ = sender.send(result);
    }
}

/// Snapshot completion commits the prepared DOM before the staged window is revealed.
/// Snapshot alpha is not a readiness signal: browser canvases and popovers can be opaque.
pub async fn wait_frame(
    app: &tauri::AppHandle,
    label: &str,
    timeout: Duration,
) -> Result<(), String> {
    let view = app.get_webview(label).ok_or("浮层 WebView 已关闭")?;
    let (sender, receiver) = oneshot::channel();
    let sender: Reply = Arc::new(Mutex::new(Some(sender)));
    view.with_webview(move |native| {
        #[cfg(target_os = "linux")]
        {
            use webkit2gtk::WebViewExt;
            native.inner().snapshot(
                webkit2gtk::SnapshotRegion::Visible,
                webkit2gtk::SnapshotOptions::TRANSPARENT_BACKGROUND,
                None::<&gio::Cancellable>,
                move |surface| {
                    let result = surface.map(|_| ()).map_err(|e| e.to_string());
                    reply(&sender, result);
                },
            );
        }
        #[cfg(target_os = "windows")]
        unsafe {
            use webview2_com::{
                CapturePreviewCompletedHandler,
                Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            };
            use windows::Win32::{
                Foundation::HGLOBAL, System::Com::StructuredStorage::CreateStreamOnHGlobal,
            };
            let failed = sender.clone();
            let result = (|| -> windows::core::Result<()> {
                let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
                let target = stream.clone();
                let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                    // Retain the output stream until WebView2 completes its frame capture.
                    let _stream = &target;
                    reply(&sender, result.map_err(|e| e.to_string()));
                    Ok(())
                }));
                native.controller().CoreWebView2()?.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &handler,
                )
            })();
            if let Err(error) = result {
                reply(&failed, Err(error.to_string()));
            }
        }
        #[cfg(target_os = "macos")]
        unsafe {
            use objc2::MainThreadMarker;
            use objc2_app_kit::NSImage;
            use objc2_foundation::NSError;
            use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
            let Some(marker) = MainThreadMarker::new() else {
                reply(&sender, Err("WKWebView 快照不在主线程".into()));
                return;
            };
            let view = &*native.inner().cast::<WKWebView>();
            let configuration = WKSnapshotConfiguration::new(marker);
            configuration.setAfterScreenUpdates(true);
            let callback = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let result = if !error.is_null() {
                    Err(format!("WKWebView 帧快照失败: {:?}", &*error))
                } else if !image.is_null() {
                    Ok(())
                } else {
                    Err("WKWebView 返回空帧快照".into())
                };
                reply(&sender, result);
            });
            view.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &callback);
        }
    })
    .map_err(|e| e.to_string())?;
    tokio::time::timeout(timeout, receiver)
        .await
        .map_err(|_| "原生帧快照未在期限内完成")?
        .map_err(|_| "原生帧快照回调已取消")?
}

pub async fn opacity(app: &tauri::AppHandle, label: &str, alpha: f64) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = (app, label, alpha);
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let window = app.get_window(label).ok_or("浮层窗口已关闭")?;
        let app = app.clone();
        let (sender, receiver) = oneshot::channel();
        app.run_on_main_thread(move || {
            #[cfg(target_os = "linux")]
            let result = {
                use gtk::prelude::*;
                window
                    .gtk_window()
                    .map(|window| window.set_opacity(alpha))
                    .map_err(|e| e.to_string())
            };
            #[cfg(target_os = "macos")]
            let result = window
                .ns_window()
                .map(|window| unsafe {
                    (&*window.cast::<objc2_app_kit::NSWindow>()).setAlphaValue(alpha);
                })
                .map_err(|e| e.to_string());
            let _ = sender.send(result);
        })
        .map_err(|e| e.to_string())?;
        receiver.await.map_err(|_| "窗口透明度操作已取消")?
    }
}
