use std::{collections::HashSet, sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use super_paste_core::{Error, Result, clipboard::ClipboardSnapshot, images::{ImageLimits, ImagePriority, PixelBuffer, PixelLayout, PixelSpec, ResizeFilter}, preview::{PreviewBody, PreviewModel, PreviewScope}};
use super_paste_format::{Digest, HistoryEntry, RecordId, TextContent};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};
use super::{OverlayView, Phase};

pub(super) struct CardPreview {
    revision: u64, ticket: u64, spec: PixelSpec, cancel: CancellationToken,
    pub model: Option<Arc<PreviewModel>>, pub image: Option<Arc<RenderImage>>, pixel: Option<Digest>,
    pub rich: Option<super::rich::RichCard>,
    pub text: Option<super::preview_content::PreparedText>, pub caption: Option<SharedString>, pub href: Option<SharedString>,
    pub file_labels: Option<Arc<[super::preview_content::FileLabels]>>,
    pub error: Option<SharedString>, pub image_error: Option<SharedString>, pub loading: bool,
    task: Option<Task<()>>, rich_task: Option<Task<()>>, pub copying: bool, pub copied: Option<&'static str>,
}
impl Drop for CardPreview { fn drop(&mut self) { self.cancel.cancel(); } }
impl CardPreview {
    fn fail_loading(&mut self, message: SharedString) {
        self.cancel.cancel(); self.task = None; self.rich_task = None;
        self.copying = false; self.copied = None;
        self.model = None; self.image = None; self.pixel = None; self.rich = None;
        self.text = None; self.caption = None; self.href = None; self.file_labels = None; self.image_error = None;
        self.loading = false; self.error = Some(message);
    }
}
struct LoadedPreview { model: Arc<PreviewModel>, pixels: Result<Option<Arc<PixelBuffer>>> }

impl OverlayView {
    pub(super) fn reconcile_previews(&mut self, entries: &[(usize, Arc<HistoryEntry>)], window: &mut Window, cx: &mut Context<Self>) {
        let visible: HashSet<_> = entries.iter().map(|(_, entry)| entry.id).collect();
        if self.hovered_card.is_some_and(|id| !visible.contains(&id)) { self.hovered_card = None; }
        if self.pressed_card.is_some_and(|id| !visible.contains(&id)) { self.pressed_card = None; }
        self.previews.retain(|id, _| visible.contains(id));
        self.reconcile_source_icons(entries, window, cx);
        self.prune_rasters(window);
        let extent = self.card_size(); let scale = window.scale_factor();
        // Bands prevent one derived image for each pixel of a window resize.
        let band = |value: Pixels| ((f32::from(value).max(1.0) * scale).ceil() as u32).div_ceil(64).clamp(1, 128) * 64;
        let spec = PixelSpec { max_width: band(extent.width - px(24.0)), max_height: band(extent.height - px(134.0)), layout: PixelLayout::Bgra8, filter: ResizeFilter::Triangle };
        for (_, entry) in entries {
            if let Some(preview) = self.previews.get_mut(&entry.id).filter(|preview| preview.revision == entry.content_revision
                && !preview.loading && !preview.cancel.is_cancelled() && preview.rich_task.is_some()) {
                if let Some(rich) = preview.rich.as_ref().filter(|rich| rich.has_images()) {
                    if preview.spec == spec { continue; }
                    // The document worker owns generation checks and keeps
                    // the old raster until its replacement is accepted.
                    // A size change must not rebuild the native document.
                    if rich.image_sources().resize_if_running(spec) {
                        preview.spec = spec;
                        continue;
                    }
                }
            }
            let needs_load = self.previews.get(&entry.id).is_none_or(|preview| preview.revision != entry.content_revision || (entry.image.is_some() || preview.rich.as_ref().is_some_and(|rich| rich.has_images())) && preview.spec != spec);
            if !needs_load { continue; }
            self.start_preview(entry.clone(), spec, window, cx);
        }
        self.preview_layout = true;
        self.preview_prepared(window, cx);
    }
    fn start_preview(&mut self, entry: Arc<HistoryEntry>, spec: PixelSpec, window: &mut Window, cx: &mut Context<Self>) {
        let id = entry.id; let revision = entry.content_revision;
        self.preview_ticket = self.preview_ticket.wrapping_add(1); let ticket = self.preview_ticket;
        let previous = self.previews.remove(&id).filter(|preview| preview.revision == revision);
        let model = previous.as_ref().and_then(|preview| preview.model.clone());
        let cancel = CancellationToken::new(); let request_cancel = cancel.clone(); let services = self.services.clone();
        let task = services.runtime.spawn({ let services = services.clone(); async move {
            let model = match model {
                Some(model) => model,
                None => services.previews.get(entry, PreviewScope::Card, &request_cancel).await?,
            };
            let pixels = async {
                if let Some(image) = model.image.clone().filter(|_| matches!(&model.body, PreviewBody::Image)) {
                    let pixels = services.images.acquire_card_pixels(image, ImageLimits::default(), spec, &request_cancel).await?;
                    Ok(Some(pixels))
                } else { Ok(None) }
            }.await;
            Ok::<_, Error>(LoadedPreview { model, pixels })
        }});
        let foreground = cx.spawn_in(window, async move |weak, cx| {
            let task = AbortOnDropHandle::new(task);
            let result = task.await.map_err(Error::from).and_then(|value| value);
            let _ = weak.update_in(cx, |this, window, cx| this.accept_preview(id, revision, ticket, result, window, cx));
        });
        let mut preview = CardPreview { revision, ticket, spec, cancel, model: None, image: None, rich: None, text: None, caption: None, href: None, file_labels: None, pixel: None, error: None, image_error: None, loading: true,
            task: Some(foreground), rich_task: None, copying: false, copied: None };
        if let Some(previous) = &previous {
            preview.model = previous.model.clone(); preview.image = previous.image.clone(); preview.pixel = previous.pixel; preview.rich = previous.rich.clone();
            preview.text = previous.text.clone(); preview.caption = previous.caption.clone(); preview.href = previous.href.clone();
            preview.file_labels = previous.file_labels.clone();
        }
        self.previews.insert(id, preview);
    }
    fn accept_preview(&mut self, id: RecordId, revision: u64, ticket: u64, result: Result<LoadedPreview>, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Hidden || !self.previews.get(&id).is_some_and(|preview| preview.revision == revision && preview.ticket == ticket && preview.loading && !preview.cancel.is_cancelled()) { return; }
        if self.phase == Phase::Preparing { self.preparation_dirty = true; }
        let incoming = result.as_ref().ok().and_then(|loaded| loaded.pixels.as_ref().ok()).and_then(Option::as_ref).map(|pixels| pixels.id());
        if let Some(preview) = self.previews.get_mut(&id) { preview.loading = false; preview.task = None; preview.image = None; preview.pixel = None; }
        self.prune_rasters_with(incoming, window);
        let result = result.map(|loaded| {
            let image = loaded.pixels.and_then(|pixels| pixels.map(|pixels| { let id = pixels.id(); self.rasters.acquire(pixels).map(|image| (id, image)) }).transpose());
            let rich = if let PreviewBody::Rich(document) = &loaded.model.body { super::rich::RichCard::new(document, cx) } else { None };
            let previous = self.previews.get(&id).filter(|preview| preview.model.as_ref()
                .is_some_and(|model| Arc::ptr_eq(model, &loaded.model)));
            let literal = matches!(&loaded.model.body, PreviewBody::Text | PreviewBody::Code | PreviewBody::Link(_) | PreviewBody::Contact)
                || (matches!(&loaded.model.body, PreviewBody::Rich(_)) && rich.is_none());
            let text = literal.then(|| previous.and_then(|preview| preview.text.clone())
                .unwrap_or_else(|| super::preview_content::PreparedText::new(&loaded.model, cx)));
            let caption = previous.and_then(|preview| preview.caption.clone())
                .or_else(|| super::preview_content::image_caption(&loaded.model));
            let href = if let PreviewBody::Link(link) = &loaded.model.body {
                Some(previous.and_then(|preview| preview.href.clone()).unwrap_or_else(|| super::preview_content::shared_excerpt(&link.href, 12_000)))
            } else { None };
            let file_labels = previous.and_then(|preview| preview.file_labels.clone())
                .or_else(|| super::preview_content::file_labels(&loaded.model));
            (loaded.model, image, rich, text, caption, href, file_labels)
        });
        let failed = result.is_err();
        if let Some(preview) = self.previews.get_mut(&id) {
            match result {
                Ok((model, image, rich, text, caption, href, file_labels)) => {
                    preview.model = Some(model); preview.rich = rich; preview.text = text; preview.caption = caption; preview.href = href;
                    preview.file_labels = file_labels;
                    preview.error = None; preview.image_error = None;
                    match image {
                        Ok(Some((id, image))) => { preview.pixel = Some(id); preview.image = Some(image); },
                        Ok(None) => {},
                        Err(error) => { tracing::warn!(%error, "加载卡片图片失败"); preview.image_error = Some(error.to_string().into()); },
                    }
                },
                Err(error) => {
                    tracing::warn!(%error, "加载可见卡片预览失败"); preview.fail_loading(error.to_string().into());
                },
            }
        }
        if failed { self.prune_rasters(window); }
        self.complete_menu_preview(id, window, cx);
        self.start_rich_images(id, revision, ticket, window, cx);
        self.preview_prepared(window, cx); cx.notify();
    }
    fn start_rich_images(&mut self, id: RecordId, revision: u64, ticket: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self.previews.get_mut(&id) else { return; };
        let Some(rich) = preview.rich.as_ref().filter(|rich| rich.has_images()) else { return; };
        let (mut responses, worker) = super::rich::load_images(&self.services.runtime, self.services.images.clone(), rich.image_sources(), cx.entity().downgrade(), preview.spec, preview.cancel.clone(), ImagePriority::Visible);
        preview.rich_task = Some(cx.spawn_in(window, async move |weak, cx| {
            let worker = AbortOnDropHandle::new(worker);
            while let Some(response) = responses.recv().await {
                let accepted = weak.update_in(cx, |this, window, cx| this.accept_rich_image(id, revision, ticket, response, window, cx));
                if !matches!(accepted, Ok(true)) { return; }
            }
            if let Err(error) = worker.await { if !error.is_cancelled() {
                tracing::error!(%error, "富文本图片任务异常结束");
                let _ = weak.update_in(cx, |this, window, cx| {
                    if let Some(rich) = this.previews.get(&id).filter(|preview| preview.revision == revision && preview.ticket == ticket).and_then(|preview| preview.rich.as_ref()) { rich.image_loader_failed(); this.prune_rasters(window); if this.phase == Phase::Preparing { this.preparation_dirty = true; } this.preview_prepared(window, cx); cx.notify(); }
                });
            } }
        }));
    }
    fn accept_rich_image(&mut self, id: RecordId, revision: u64, ticket: u64, response: super::rich::ImageResponse, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.phase == Phase::Hidden || !self.previews.get(&id).is_some_and(|preview| preview.revision == revision && preview.ticket == ticket && preview.rich.is_some()) { return false; }
        if !self.previews.get(&id).and_then(|preview| preview.rich.as_ref()).is_some_and(|rich| rich.wants_image(&response.key, response.generation)) { self.preview_prepared(window, cx); cx.notify(); return true; }
        if self.phase == Phase::Preparing { self.preparation_dirty = true; }
        let incoming = response.pixels.as_ref().ok().map(|display| display.pixels.id());
        self.prune_rasters_with(incoming, window);
        let frame = response.pixels.and_then(|display| {
            let id = display.pixels.id();
            self.rasters.acquire(display.pixels).map(|image| super::rich::RichImageFrame { id, image, pin: None, natural_dimensions: display.natural_dimensions,
                animation: display.animation, next_frame: display.next_frame, checkpoint: display.checkpoint })
        });
        if let Err(error) = &frame { tracing::warn!(%error, "富文本内嵌图片无法预览"); }
        if let Some(rich) = self.previews.get(&id).and_then(|preview| preview.rich.as_ref()) { rich.accept_image(response.key, response.generation, frame, cx); }
        self.prune_rasters(window);
        self.preview_prepared(window, cx); cx.notify(); true
    }
    pub(super) fn prune_rasters(&mut self, window: &mut Window) {
        self.prune_rasters_with(None, window);
    }
    pub(super) fn prune_rasters_with(&mut self, incoming: Option<Digest>, window: &mut Window) {
        let mut visible = self.previews.values().filter_map(|preview| preview.pixel)
            .chain(self.source_icons.values().filter_map(|icon| icon.pixel)).chain(incoming).collect();
        for rich in self.previews.values().filter_map(|preview| preview.rich.as_ref()) { rich.retain_pixels(&mut visible); }
        if let Err(error) = self.rasters.retain(&visible, window) { tracing::error!(%error, "回收覆盖层绘制图片失败"); self.error = Some(error.to_string().into()); }
    }
    pub(super) fn clear_previews(&mut self, window: &mut Window) {
        if self.phase == Phase::Preparing { self.preparation_dirty = true; }
        self.previews.clear(); self.source_icons.clear(); self.preview_layout = false; self.preview_deadline = None;
        if let Err(error) = self.rasters.clear(window) { tracing::error!(%error, "释放覆盖层绘制资源失败"); self.error = Some(error.to_string().into()); }
    }
    pub(super) fn begin_preview_preparation(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        if self.phase != Phase::Preparing || self.frame.is_some() { return Ok(()); }
        if self.entries.is_empty() { return self.prepared(window, cx); }
        self.preview_layout = false;
        if self.preview_deadline.is_none() {
            window.set_pointer_passthrough(true).map_err(super::platform_error)?;
            window.set_visible(true, cx).map_err(super::platform_error)?;
            let generation = self.generation; let timer = cx.background_executor().timer(Duration::from_secs(3));
            self.preview_deadline = Some(cx.spawn_in(window, async move |weak, cx| {
                timer.await;
                let _ = weak.update_in(cx, |this, window, cx| {
                    if this.generation != generation || this.phase != Phase::Preparing { return; }
                    let mut changed = this.timeout_source_icons();
                    for preview in this.previews.values_mut() {
                        if preview.loading {
                            changed = true;
                            preview.fail_loading("预览加载超时".into());
                        }
                        if let Some(rich) = preview.rich.as_ref().filter(|rich| rich.images_pending()) {
                            changed = true;
                            preview.cancel.cancel(); preview.rich_task = None; rich.image_loader_timed_out();
                        }
                    }
                    if changed { this.prune_rasters(window); }
                    this.preparation_dirty |= changed;
                    this.preview_layout = true; this.preview_prepared(window, cx); cx.notify();
                });
            }));
        }
        Ok(())
    }
    pub(super) fn previews_ready(&self) -> bool {
        self.preview_layout && self.source_icons_ready() && self.previews.values().all(|preview| !preview.loading && !preview.rich.as_ref().is_some_and(|rich| rich.images_pending()))
    }
    pub(super) fn preview_prepared(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Preparing && self.previews_ready() {
            if let Err(error) = self.prepared(window, cx) { self.fail(error, window, cx); }
        }
    }
    pub(super) fn retry_preview(&mut self, id: RecordId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id).cloned() else { return; };
        let Some(spec) = self.previews.get(&id).map(|preview| preview.spec) else { return; };
        self.start_preview(entry, spec, window, cx); cx.notify();
    }
    pub(super) fn copy_color(&mut self, id: RecordId, rgb: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.services.is_shutting_down() { return; }
        let Some(preview) = self.previews.get_mut(&id).filter(|preview| !preview.copying) else { return; };
        let Some(model) = &preview.model else { return; };
        let super_paste_core::preview::PreviewBody::Color(color) = &model.body else { return; };
        let text = if rgb { color.rgb.clone() } else { color.hex.clone() }; let revision = preview.revision; let ticket = preview.ticket;
        preview.copying = true; preview.copied = None;
        let clipboard = self.services.clipboard.clone();
        let desktop = cx.global::<crate::host::DesktopGlobal>().0.downgrade();
        // The service owns the write; its detached result observer must also
        // survive virtual-list eviction without retaining preview resources.
        let task = self.services.runtime.spawn(async move { clipboard.write(ClipboardSnapshot { content: TextContent { text: Some(text), ..Default::default() }, image: None }, &CancellationToken::new()).await });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|value| value);
            let _ = weak.update_in(cx, |this, window, cx| {
                let visible = this.can_present_feedback(window) && !this.services.is_shutting_down();
                let Some(preview) = this.previews.get_mut(&id).filter(|preview| preview.revision == revision && preview.ticket == ticket) else { return; };
                preview.copying = false;
                if result.is_ok() && visible { preview.copied = Some(if rgb { "RGB" } else { "HEX" }); }
                cx.notify();
            });
            if let Err(error) = result {
                tracing::warn!(%error, "复制颜色失败");
                super::present_operation_failure(&weak, &desktop, format!("复制颜色失败：{error}").into(), cx);
            }
        }).detach();
        cx.notify();
    }
}
