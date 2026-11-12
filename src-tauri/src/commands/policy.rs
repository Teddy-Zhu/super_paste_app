use super::{WindowAccess, main_only, overlay_only};

pub(super) fn authorize(channel: &str, access: WindowAccess) -> Result<(), String> {
    if matches!(
        channel,
        "statistics:get-copy" | "statistics:clear-copy" | "ocr:get-usage" | "ocr:clear-usage"
    ) || channel.starts_with("mcp:")
        || channel.starts_with("scheduled-sync:")
        || matches!(
            channel,
            "ocr:download-model"
                | "ocr:cancel-download"
                | "ocr:delete-model"
                | "ocr:redownload-model"
                | "ocr:ai-list-models"
                | "ocr:ai-test-connection"
                | "export-diagnostics"
                | "system-access-retry"
                | "app:restart"
        )
    {
        main_only(access)?;
    }
    if matches!(
        channel,
        "clipboard-copy-color"
            | "overlay-ready"
            | "overlay-open-prepared"
            | "overlay-close-started"
            | "open-overlay-finished"
            | "close-overlay-window"
            | "overlay-trace"
    ) {
        overlay_only(access)?;
    }
    Ok(())
}
