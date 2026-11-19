use std::{collections::{HashMap, HashSet}, sync::Arc};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::tooltip::Tooltip;
use super_paste_core::{Error, Result, images::{ImageLimits, ImageSource, PixelBuffer, PixelLayout, PixelSpec, ResizeFilter}};
use super_paste_format::{Digest, HistoryEntry, SourceApp, SourceId};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};
use crate::ui::palette::Palette;
use super::{OverlayView, Phase};

pub(super) struct SourceIcon {
    source: Arc<SourceApp>, spec: PixelSpec, ticket: u64, cancel: CancellationToken,
    _task: Option<Task<()>>, image: Option<Arc<RenderImage>>, pub pixel: Option<Digest>, failed: bool,
}
impl Drop for SourceIcon { fn drop(&mut self) { self.cancel.cancel(); } }

impl OverlayView {
    pub(super) fn source_icons_ready(&self) -> bool { self.source_icons.values().all(|icon| icon._task.is_none()) }
    pub(super) fn timeout_source_icons(&mut self) -> bool {
        let mut changed = false;
        for icon in self.source_icons.values_mut().filter(|icon| icon._task.is_some()) {
            icon.cancel.cancel(); icon._task = None; icon.failed = true; changed = true;
        }
        if changed { tracing::warn!("覆盖层来源图标加载超时"); }
        changed
    }
    pub(super) fn reconcile_source_icons(&mut self, entries: &[(usize, Arc<HistoryEntry>)], window: &mut Window, cx: &mut Context<Self>) {
        // Closing Kit dialogs still display their source until their view is
        // released. Reuse these same icon owners alongside visible cards.
        let sources: HashMap<_, _> = self.detail_sources(cx)
            .chain(entries.iter().filter_map(|(_, entry)| entry.source.clone()))
            .filter(|source| source.icon.is_some()).map(|source| (source.id, source)).collect();
        self.source_icons.retain(|id, _| sources.contains_key(id));
        let extent = (32.0 * window.scale_factor()).ceil().clamp(1.0, 96.0) as u32;
        let spec = PixelSpec { max_width: extent, max_height: extent, layout: PixelLayout::Bgra8, filter: ResizeFilter::Triangle };
        for (id, source) in sources {
            if let Some(icon) = self.source_icons.get_mut(&id) {
                if icon.spec == spec && (Arc::ptr_eq(&icon.source, &source) || icon.source.icon == source.icon) {
                    icon.source = source; continue;
                }
            }
            self.start_source_icon(source, spec, window, cx);
        }
    }
    fn start_source_icon(&mut self, source: Arc<SourceApp>, spec: PixelSpec, window: &mut Window, cx: &mut Context<Self>) {
        let Some(data) = source.icon.clone() else { return; };
        let id = source.id; let cancel = CancellationToken::new(); let token = cancel.clone(); let services = self.services.clone();
        self.preview_ticket = self.preview_ticket.wrapping_add(1); let ticket = self.preview_ticket;
        let task = AbortOnDropHandle::new(services.runtime.spawn({ let services = services.clone(); async move {
            let encoded = services.images.import_shared_encoded(data, &token).await?;
            // Preserve encoded orientation/color and native sample precision;
            // canonical data pixels are not an intermediate display format.
            services.images.acquire_display_pixels(ImageSource::Encoded(encoded),
                ImageLimits { max_encoded_bytes: 8 * 1024 * 1024, max_pixels: 96 * 96, max_dimension: 96 }, spec, &token)
                .await.map(|display| display.pixels)
        }}));
        let foreground = cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| this.accept_source_icon(id, ticket, result, window, cx));
        });
        self.source_icons.insert(id, SourceIcon { source, spec, ticket, cancel, _task: Some(foreground), image: None, pixel: None, failed: false });
    }
    fn accept_source_icon(&mut self, id: SourceId, ticket: u64, result: Result<Arc<PixelBuffer>>, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Hidden || !self.source_icons.get(&id).is_some_and(|icon| icon.ticket == ticket
            && icon._task.is_some() && !icon.cancel.is_cancelled()) { return; }
        if self.phase == Phase::Preparing { self.preparation_dirty = true; }
        let incoming = result.as_ref().ok().map(|pixels| pixels.id());
        self.prune_rasters_with(incoming, window);
        let result = result.and_then(|pixels| { let id = pixels.id(); self.rasters.acquire(pixels).map(|image| (id, image)) });
        if let Some(icon) = self.source_icons.get_mut(&id) {
            icon._task = None;
            match result {
                Ok((pixel, image)) => { icon.pixel = Some(pixel); icon.image = Some(image); },
                Err(error) => { icon.failed = true; tracing::debug!(%error, "加载来源应用图标失败"); },
            }
        }
        self.notify_details_source(id, cx);
        self.preview_prepared(window, cx); cx.notify();
    }
    pub(super) fn prune_removed_source_icons(&mut self, cx: &App) {
        let sources: HashSet<_> = self.entries.iter().filter_map(|entry| entry.source.as_ref().map(|source| source.id))
            .chain(self.detail_sources(cx).map(|source| source.id)).collect();
        self.source_icons.retain(|id, _| sources.contains(id));
    }
    pub(super) fn source_label(&self, source: &SourceApp, p: Palette) -> AnyElement {
        let icon = self.source_icons.get(&source.id);
        let has_icon = source.icon.is_some() && !icon.is_some_and(|icon| icon.failed);
        let tooltip: SharedString = format!("来源应用 · {}", source.application).into();
        div().id("source-app").flex().flex_col().items_end().justify_center().gap(px(3.0)).min_w_0().max_w(px(120.0)).flex_shrink_0().ml_auto().text_right()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .when(has_icon, |label| label.child(div().size(px(32.0)).flex_shrink_0()
                .when_some(icon.and_then(|icon| icon.image.clone()), |frame, image| frame.child(img(image).size_full().object_fit(ObjectFit::Contain)))))
            .child(div().max_w_full().text_ellipsis().text_color(p.text_secondary).text_size(px(if has_icon { 10.0 } else { 12.0 }))
                .line_height(px(if has_icon { 14.0 } else { 16.0 })).child(SharedString::from(source.application.clone()))).into_any_element()
    }
}
