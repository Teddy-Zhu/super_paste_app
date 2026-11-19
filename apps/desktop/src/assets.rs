use std::borrow::Cow;
use gpui_kit::{AssetSource, SharedString};

#[derive(Clone, Copy)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "superpaste/feedback/loading-arc.svg" => Some(include_bytes!("../../../resources/feedback/loading-arc.svg")),
            "superpaste/feedback/loading-arc-small.svg" => Some(include_bytes!("../../../resources/feedback/loading-arc-small.svg")),
            "superpaste/feedback/ellipsis.svg" => Some(include_bytes!("../../../resources/feedback/ellipsis.svg")),
            "superpaste/window-controls/minimize.svg" => Some(include_bytes!("../../../resources/window-controls/minimize.svg")),
            "superpaste/window-controls/maximize.svg" => Some(include_bytes!("../../../resources/window-controls/maximize.svg")),
            "superpaste/window-controls/restore.svg" => Some(include_bytes!("../../../resources/window-controls/restore.svg")),
            "superpaste/window-controls/close.svg" => Some(include_bytes!("../../../resources/window-controls/close.svg")),
            "superpaste/icons/mdi/arrow-expand.svg" => Some(include_bytes!("../../../resources/icons/mdi/arrow-expand.svg")),
            "superpaste/icons/mdi/at.svg" => Some(include_bytes!("../../../resources/icons/mdi/at.svg")),
            "superpaste/icons/mdi/bug.svg" => Some(include_bytes!("../../../resources/icons/mdi/bug.svg")),
            "superpaste/icons/mdi/clipboard-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/clipboard-outline.svg")),
            "superpaste/icons/mdi/cog-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/cog-outline.svg")),
            "superpaste/icons/mdi/magnify.svg" => Some(include_bytes!("../../../resources/icons/mdi/magnify.svg")),
            "superpaste/icons/mdi/calendar-month-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/calendar-month-outline.svg")),
            "superpaste/icons/mdi/chart-line.svg" => Some(include_bytes!("../../../resources/icons/mdi/chart-line.svg")),
            "superpaste/icons/mdi/check.svg" => Some(include_bytes!("../../../resources/icons/mdi/check.svg")),
            "superpaste/icons/mdi/chevron-down.svg" => Some(include_bytes!("../../../resources/icons/mdi/chevron-down.svg")),
            "superpaste/icons/mdi/chevron-left.svg" => Some(include_bytes!("../../../resources/icons/mdi/chevron-left.svg")),
            "superpaste/icons/mdi/chevron-right.svg" => Some(include_bytes!("../../../resources/icons/mdi/chevron-right.svg")),
            "superpaste/icons/mdi/chevron-up.svg" => Some(include_bytes!("../../../resources/icons/mdi/chevron-up.svg")),
            "superpaste/icons/mdi/circle.svg" => Some(include_bytes!("../../../resources/icons/mdi/circle.svg")),
            "superpaste/icons/mdi/clipboard-text-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/clipboard-text-outline.svg")),
            "superpaste/icons/mdi/close.svg" => Some(include_bytes!("../../../resources/icons/mdi/close.svg")),
            "superpaste/icons/mdi/cog.svg" => Some(include_bytes!("../../../resources/icons/mdi/cog.svg")),
            "superpaste/icons/mdi/connection.svg" => Some(include_bytes!("../../../resources/icons/mdi/connection.svg")),
            "superpaste/icons/mdi/content-copy.svg" => Some(include_bytes!("../../../resources/icons/mdi/content-copy.svg")),
            "superpaste/icons/mdi/content-paste.svg" => Some(include_bytes!("../../../resources/icons/mdi/content-paste.svg")),
            "superpaste/icons/mdi/delete.svg" => Some(include_bytes!("../../../resources/icons/mdi/delete.svg")),
            "superpaste/icons/mdi/file-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/file-outline.svg")),
            "superpaste/icons/mdi/information.svg" => Some(include_bytes!("../../../resources/icons/mdi/information.svg")),
            "superpaste/icons/mdi/keyboard.svg" => Some(include_bytes!("../../../resources/icons/mdi/keyboard.svg")),
            "superpaste/icons/mdi/link-variant.svg" => Some(include_bytes!("../../../resources/icons/mdi/link-variant.svg")),
            "superpaste/icons/mdi/open-in-new.svg" => Some(include_bytes!("../../../resources/icons/mdi/open-in-new.svg")),
            "superpaste/icons/mdi/pencil-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/pencil-outline.svg")),
            "superpaste/icons/mdi/phone-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/phone-outline.svg")),
            "superpaste/icons/mdi/plus.svg" => Some(include_bytes!("../../../resources/icons/mdi/plus.svg")),
            "superpaste/icons/mdi/refresh.svg" => Some(include_bytes!("../../../resources/icons/mdi/refresh.svg")),
            "superpaste/icons/mdi/settings.svg" => Some(include_bytes!("../../../resources/icons/mdi/settings.svg")),
            "superpaste/icons/mdi/sync.svg" => Some(include_bytes!("../../../resources/icons/mdi/sync.svg")),
            "superpaste/icons/mdi/tag.svg" => Some(include_bytes!("../../../resources/icons/mdi/tag.svg")),
            "superpaste/icons/mdi/tag-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/tag-outline.svg")),
            "superpaste/icons/mdi/text-box.svg" => Some(include_bytes!("../../../resources/icons/mdi/text-box.svg")),
            "superpaste/icons/mdi/text-recognition.svg" => Some(include_bytes!("../../../resources/icons/mdi/text-recognition.svg")),
            "superpaste/icons/mdi/trash-can-outline.svg" => Some(include_bytes!("../../../resources/icons/mdi/trash-can-outline.svg")),
            "superpaste/app-icon.png" => Some(include_bytes!("../../../resources/app-icon.png")),
            _ => None,
        };
        match bytes {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut result = gpui_kit::assets::Assets.list(path)?;
        result.extend(PATHS.iter().copied().filter(|candidate| candidate.starts_with(path)).map(Into::into));
        Ok(result)
    }
}

const PATHS: &[&str] = &[
    "superpaste/window-controls/minimize.svg",
    "superpaste/window-controls/maximize.svg",
    "superpaste/window-controls/restore.svg",
    "superpaste/window-controls/close.svg",
    "superpaste/icons/mdi/arrow-expand.svg",
    "superpaste/icons/mdi/at.svg",
    "superpaste/icons/mdi/bug.svg",
    "superpaste/icons/mdi/clipboard-outline.svg",
    "superpaste/icons/mdi/cog-outline.svg",
    "superpaste/icons/mdi/magnify.svg",
    "superpaste/icons/mdi/calendar-month-outline.svg",
    "superpaste/icons/mdi/chart-line.svg",
    "superpaste/icons/mdi/check.svg",
    "superpaste/icons/mdi/chevron-down.svg",
    "superpaste/icons/mdi/chevron-left.svg",
    "superpaste/icons/mdi/chevron-right.svg",
    "superpaste/icons/mdi/chevron-up.svg",
    "superpaste/icons/mdi/circle.svg",
    "superpaste/icons/mdi/clipboard-text-outline.svg",
    "superpaste/icons/mdi/close.svg",
    "superpaste/icons/mdi/cog.svg",
    "superpaste/icons/mdi/connection.svg",
    "superpaste/icons/mdi/content-copy.svg",
    "superpaste/icons/mdi/content-paste.svg",
    "superpaste/icons/mdi/delete.svg",
    "superpaste/icons/mdi/file-outline.svg",
    "superpaste/icons/mdi/information.svg",
    "superpaste/icons/mdi/keyboard.svg",
    "superpaste/icons/mdi/link-variant.svg",
    "superpaste/icons/mdi/open-in-new.svg",
    "superpaste/icons/mdi/pencil-outline.svg",
    "superpaste/icons/mdi/phone-outline.svg",
    "superpaste/icons/mdi/plus.svg",
    "superpaste/icons/mdi/refresh.svg",
    "superpaste/icons/mdi/settings.svg",
    "superpaste/icons/mdi/sync.svg",
    "superpaste/icons/mdi/tag.svg",
    "superpaste/icons/mdi/tag-outline.svg",
    "superpaste/icons/mdi/text-box.svg",
    "superpaste/icons/mdi/text-recognition.svg",
    "superpaste/icons/mdi/trash-can-outline.svg",
    "superpaste/feedback/loading-arc.svg",
    "superpaste/feedback/loading-arc-small.svg",
    "superpaste/feedback/ellipsis.svg",
    "superpaste/app-icon.png",
];
