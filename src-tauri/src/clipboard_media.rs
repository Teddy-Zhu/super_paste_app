use crate::runtime::RuntimeHolder;
use tauri::{Manager, UriSchemeContext, UriSchemeResponder, Wry, http};

// Keep image bytes out of JSON IPC. Only application webviews may resolve stored media.
pub fn handle(
    context: UriSchemeContext<'_, Wry>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let response = |status, body| {
        http::Response::builder()
            .status(status)
            .header(http::header::CACHE_CONTROL, "no-store")
            .body(body)
            .expect("valid clipboard media response")
    };
    if !matches!(context.webview_label(), "main" | "overlay") {
        responder.respond(response(403, Vec::new()));
        return;
    }
    let parts: Vec<_> = request
        .uri()
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect();
    if parts.len() != 3
        || !matches!(parts[0], "preview" | "original")
        || parts[1..].iter().any(|part| {
            part.is_empty()
                || part.len() > 128
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        responder.respond(response(400, Vec::new()));
        return;
    }
    let runtime = match context.app_handle().state::<RuntimeHolder>().get() {
        Ok(runtime) => runtime,
        Err(_) => {
            responder.respond(response(503, Vec::new()));
            return;
        }
    };
    let (kind, id, digest) = (
        parts[0].to_owned(),
        parts[1].to_owned(),
        parts[2].to_owned(),
    );
    tauri::async_runtime::spawn(async move {
        match runtime
            .storage
            .get_clipboard_media(&kind, &id, &digest)
            .await
        {
            Ok(Some(bytes)) => {
                let mime = image::guess_format(&bytes)
                    .map(|format| format.to_mime_type())
                    .unwrap_or("application/octet-stream");
                responder.respond(
                    http::Response::builder()
                        .header(http::header::CONTENT_TYPE, mime)
                        .header(http::header::CACHE_CONTROL, "no-store")
                        .body(bytes)
                        .expect("valid clipboard image response"),
                );
            }
            Ok(None) => responder.respond(response(404, Vec::new())),
            Err(error) => {
                tracing::warn!(%error, "读取剪贴板图片失败");
                responder.respond(response(500, Vec::new()));
            }
        }
    });
}
