mod layout;

use std::{collections::HashSet, sync::Arc};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, TextSelection, TextView, TextViewState, TextViewStyle};
use gpui_kit::component::WindowExt;
use super_paste_core::{Error, Result, clipboard::ClipboardSnapshot, images::{DisplayFrame, ImageLimits, ImagePriority, ImageSource, PixelSpec}, preview::{PreviewBody, PreviewKind, PreviewModel, PreviewNotice, PreviewScope, text_whitespace}};
use super_paste_format::{Digest, RecordId, SourceApp, SourceId, TextContent};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};
use crate::{services::Services, ui::{configuration::ConfigurationGlobal, dialogs, palette::Palette, rasters::WindowRasters, text::normal_flow_text}};
use super::{OverlayView, rich::{ImageSources, RichCard, StandaloneImage}};

pub(super) struct Details {
    pub id: RecordId, revision: u64, request: u64, cancel: CancellationToken,
    source: Option<Arc<SourceApp>>, pub(super) presenting: bool, pub(super) returning: bool,
    _task: Option<Task<()>>, validation: Option<Task<()>>, window: Option<AnyWindowHandle>,
    view: Option<WeakEntity<DetailsView>>,
}
impl Drop for Details { fn drop(&mut self) { self.cancel.cancel(); } }
struct Loaded { model: Arc<PreviewModel>, frame: Result<Option<DisplayFrame>>, dimensions: Option<(u32, u32)>, spec: PixelSpec }
fn image_spec(window: &Window) -> PixelSpec {
    super::rich::display_spec((window.viewport_size().width - px(48.)).max(px(1.)), window.scale_factor())
}
pub(super) struct DetailsView {
    content: Option<Content>, services: Arc<Services>, plain: bool,
    owner: WeakEntity<OverlayView>, owner_window: AnyWindowHandle, request: u64,
    copying: bool, copied: Option<&'static str>, disposed: bool, focus: FocusHandle,
    rasters: WindowRasters, cancel: CancellationToken, image_task: Option<Task<()>>,
    scroll: ScrollHandle, _release: Subscription, _subscriptions: Vec<Subscription>,
}
struct Content {
    model: Arc<PreviewModel>, source: Option<Arc<SourceApp>>, raw: Option<Entity<TextViewState>>,
    formatted: Option<Entity<TextViewState>>, rich: Option<RichCard>, rich_unavailable: bool,
    link: Option<Entity<TextViewState>>,
    image: Option<StandaloneImage>, image_error: Option<SharedString>,
    dimensions: Option<(u32, u32)>, image_name: Option<SharedString>,
}
impl Content {
    fn formatted(&self) -> &Entity<TextViewState> { self.formatted.as_ref().expect("literal content has a text selection state") }
    fn image_sources(&self) -> Option<ImageSources> {
        self.image.as_ref().map(StandaloneImage::sources)
            .or_else(|| self.rich.as_ref().filter(|rich| rich.has_images()).map(RichCard::image_sources))
    }
}
impl OverlayView {
    pub(super) fn details_loading(&self, id: RecordId) -> bool { self.details.as_ref().is_some_and(|details| details.id == id && details.window.is_none() && !details.returning) }
    pub(super) fn open_details(&mut self, id: RecordId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_open() || window.has_active_dialog(cx) { return; }
        if self.details.as_ref().is_some_and(|details| details.returning) { self.details = None; }
        if let Some(details) = self.details.as_ref().filter(|details| details.id == id) {
            if let Some(child) = details.window {
                if let Err(error) = child.update(cx, |_, child, _| child.activate_window_with_token(None))
                    .map_err(super::platform_error).and_then(|result| result.map_err(super::platform_error)) {
                    dialogs::message(format!("激活详情窗口失败：{error}"), window, cx);
                }
            }
            return;
        }
        self.scrolling.stop();
        self.close_details(window, cx);
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id).cloned() else { return; };
        self.preview_ticket = self.preview_ticket.wrapping_add(1); let request = self.preview_ticket;
        let revision = entry.content_revision; let source = entry.source.clone();
        let services = self.services.clone(); let cancel = CancellationToken::new(); let token = cancel.clone();
        let spec = super::rich::display_spec((self.work.bounds.size.width - px(112.)).min(px(1232.)).max(px(1.)), window.scale_factor());
        let task = AbortOnDropHandle::new(services.runtime.spawn({ let services = services.clone(); async move {
            services.images.with_priority(ImagePriority::Interactive, async {
                let model = services.previews.get(entry, PreviewScope::Details, &token).await?;
                let mut dimensions = None;
                let frame = async {
                    if let Some(image) = model.image.clone().filter(|_| matches!(&model.body, PreviewBody::Image)) {
                        let encoded = services.images.acquire_encoded(ImageSource::Stored(image), &token).await?;
                        let display = services.images.acquire_display_frame(encoded, None, ImageLimits::default(), spec, &token).await?;
                        dimensions = Some(display.natural_dimensions);
                        Ok(Some(display))
                    } else { Ok(None) }
                }.await;
                Ok::<_, Error>(Loaded { model, frame, dimensions, spec })
            }).await
        }}));
        let foreground = cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| this.accept_details(request, result, window, cx));
        });
        self.details = Some(Details { id, revision, request, cancel, source, presenting: false, returning: false, _task: Some(foreground), validation: None, window: None, view: None }); cx.notify();
    }
    fn accept_details(&mut self, request: u64, result: Result<Loaded>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.details.as_ref().filter(|details| details.request == request) else { return; };
        if !self.is_open() { self.close_details(window, cx); return; }
        let source = self.entries.iter().find(|entry| entry.id == session.id && entry.content_revision == session.revision)
            .map(|entry| entry.source.clone()).unwrap_or_else(|| session.source.clone());
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => { self.details = None; dialogs::message(format!("读取剪贴板详情失败：{error}"), window, cx); cx.notify(); return; },
        };
        let dimensions = loaded.dimensions.or_else(|| loaded.model.image.as_ref().and_then(|image| image.width.zip(image.height)));
        let options = layout::options(self.work, &loaded.model, dimensions);
        let owner = cx.entity().downgrade(); let owner_window = window.window_handle(); let services = self.services.clone(); let cancel = session.cancel.clone();
        if let Some(details) = &mut self.details { details.presenting = true; }
        #[cfg(target_os = "linux")]
        if self.layer_shell {
            if let Err(error) = window.set_layer_shell_keyboard_interactivity(gpui_kit::layer_shell::KeyboardInteractivity::OnDemand) {
                self.details = None; dialogs::message(format!("打开详情失败：{error}"), window, cx); return;
            }
        }
        // GPUI draws the new window synchronously. Its first frame may read
        // the parent (for example, the source icon), so no parent entity or
        // window update may enclose native window creation.
        cx.defer(move |cx| {
            if cancel.is_cancelled() || !owner.upgrade().is_some_and(|owner| {
                let owner = owner.read(cx);
                owner.is_open() && owner.details.as_ref().is_some_and(|details| details.request == request && details.presenting)
            }) { return; }
            let builder_owner = owner.clone();
            let opened = gpui_kit::open_window(options, cx, move |child, cx| {
                child.set_window_title(&format!("{} · 详情", loaded.model.kind.label()));
                let view = cx.new(|cx| DetailsView::new(loaded, dimensions, source, services, builder_owner, owner_window, request, cancel, child, cx));
                let weak = view.downgrade();
                child.on_window_should_close(cx, move |child, cx| { let _ = weak.update(cx, |view, cx| view.close(child, cx)); false });
                view.update(cx, |view, cx| view.start_images(child, cx));
                view
            });
            let orphan = opened.as_ref().ok().map(|(window, _)| *window);
            let attached = owner_window.update(cx, |_, window, cx| {
                owner.update(cx, |owner, cx| owner.attach_details(request, opened, window, cx))
            }).ok().and_then(std::result::Result::ok).unwrap_or(false);
            if !attached {
                if let Some(orphan) = orphan { let _ = orphan.update(cx, |_, window, _| window.remove_window()); }
            }
        });
        cx.notify();
    }
    fn attach_details(&mut self, request: u64, opened: anyhow::Result<(AnyWindowHandle, Entity<DetailsView>)>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.is_open() || !self.details.as_ref().is_some_and(|details| details.request == request && details.presenting) { return false; }
        match opened {
            Ok((child, view)) => {
                if let Some(details) = &mut self.details { details.window = Some(child); details.view = Some(view.downgrade()); details._task = None; }
                // Keep the panel mapped. Focus may move between these two
                // windows without retiring the current history session.
            },
            Err(error) => {
                self.details = None;
                if let Err(error) = self.activate(window, cx) { self.fail(error, window, cx); }
                dialogs::message(format!("打开详情窗口失败：{error}"), window, cx);
            },
        }
        cx.notify(); true
    }
    fn finish_details(&mut self, request: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(details) = self.details.as_mut().filter(|details| details.request == request && !details.returning) else { return; };
        details.returning = true; details.presenting = true; details.cancel.cancel();
        details.window = None; details.view = None; details._task = None; details.validation = None;
        self.prune_rasters(window);
        if self.is_open() {
            // Let the compositor retire the child before asking it to focus
            // the panel. Retain the focus guard until activation settles.
            let owner = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| { let _ = owner.update(cx, |this, cx| {
                if !this.details.as_ref().is_some_and(|details| details.request == request && details.returning) || !this.is_open() { return; }
                if let Err(error) = this.activate(window, cx) { this.fail(error, window, cx); return; }
                this.complete_details_return(window, cx);
            }); });
        }
        cx.notify();
    }
    pub(super) fn complete_details_return(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.details.as_ref().filter(|details| details.returning && window.is_window_active()).map(|details| details.request) else { return; };
        let owner = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| { let _ = owner.update(cx, |this, cx| {
            if window.is_window_active() && this.details.as_ref().is_some_and(|details| details.request == request && details.returning) {
                this.details = None; cx.notify();
            }
        }); });
    }
    pub(super) fn reconcile_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(details) = &self.details else { return; };
        let Some(entry) = self.entries.iter().find(|entry| entry.id == details.id) else { return; };
        if entry.content_revision != details.revision { self.dismiss_details(window, cx); return; }
        if let Some(view) = details.view.as_ref().and_then(|view| view.upgrade()) {
            let source = entry.source.clone();
            view.update(cx, |view, cx| { if let Some(content) = &mut view.content {
                let changed = match (&content.source, &source) {
                    (None, None) => false,
                    (Some(previous), Some(next)) => !Arc::ptr_eq(previous, next) && (previous.id != next.id || previous.application != next.application || previous.icon != next.icon),
                    _ => true,
                };
                content.source = source; if changed { cx.notify(); }
            } });
        }
    }
    pub(super) fn validate_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(details) = self.details.as_mut().filter(|details| !details.returning) else { return; };
        let (id, revision, request) = (details.id, details.revision, details.request);
        let storage = self.services.storage.clone(); let cancel = details.cancel.clone();
        let task = AbortOnDropHandle::new(self.services.runtime.spawn(async move { storage.record_revision(id, &cancel).await }));
        details.validation = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.details.as_ref().is_some_and(|details| details.request == request) { return; }
                if !matches!(result, Ok(Some(value)) if value == revision) {
                    if let Err(error) = &result { tracing::warn!(%error, "验证详情记录失败"); }
                    this.dismiss_details(window, cx);
                }
            });
        }));
    }
    pub(super) fn detail_sources<'a>(&'a self, cx: &'a App) -> impl Iterator<Item = Arc<SourceApp>> + 'a {
        self.details.iter().filter_map(move |details| details.view.as_ref()?.upgrade()?.read(cx).content.as_ref()?.source.clone())
    }
    pub(super) fn notify_details_source(&self, id: SourceId, cx: &mut App) {
        if let Some(view) = self.details.as_ref().and_then(|details| details.view.as_ref()).and_then(|view| view.upgrade()) {
            if view.read(cx).content.as_ref().and_then(|content| content.source.as_ref()).is_some_and(|source| source.id == id) { view.update(cx, |_, cx| cx.notify()); }
        }
    }
    pub(super) fn dismiss_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(details) = &self.details else { return; };
        let (request, child, view) = (details.request, details.window, details.view.clone());
        if let (Some(child), Some(view)) = (child, view.and_then(|view| view.upgrade())) {
            if child.update(cx, |_, child, cx| view.update(cx, |view, cx| view.close(child, cx))).is_ok() { return; }
        }
        self.finish_details(request, window, cx);
    }
    pub(super) fn close_details(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(details) = self.details.take() {
            if let Some(child) = details.window {
                if let Err(error) = child.update(cx, |_, child, cx| {
                    if let Some(view) = details.view.as_ref().and_then(|view| view.upgrade()) { view.update(cx, |view, cx| view.close(child, cx)); }
                    else { child.remove_window(); }
                }) { tracing::warn!(%error, "关闭详情窗口失败"); }
            }
            cx.notify();
        }
    }
}
impl DetailsView {
    fn new(loaded: Loaded, dimensions: Option<(u32, u32)>, source: Option<Arc<SourceApp>>, services: Arc<Services>, owner: WeakEntity<OverlayView>, owner_window: AnyWindowHandle, request: u64, cancel: CancellationToken, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut rasters = WindowRasters::default(); let image_source = loaded.model.image.clone();
        let raster = loaded.frame.and_then(|frame| frame.map(|frame| {
            let source = image_source.ok_or_else(|| Error::Invalid("详情图片来源缺失".into()))?;
            let id = frame.pixels.id(); let image = rasters.acquire(frame.pixels)?; let pin = rasters.pin(id)?;
            Ok(StandaloneImage::new(source, super::rich::RichImageFrame { id, image, pin: Some(pin), natural_dimensions: frame.natural_dimensions,
                animation: frame.animation, next_frame: frame.next_frame, checkpoint: frame.checkpoint }, loaded.spec))
        }).transpose());
        let (image, image_error) = match raster { Ok(image) => (image, None), Err(error) => { tracing::warn!(%error, "加载详情图片失败"); (None, Some("图片无法预览".into())) } };
        let model = loaded.model;
        let rich = match &model.body { PreviewBody::Rich(document) => RichCard::details(document, cx), PreviewBody::Table(_) => RichCard::data_table(&model, cx), PreviewBody::Files => Some(RichCard::files(&model, cx)), _ => None };
        let rich_unavailable = matches!(&model.body, PreviewBody::Rich(_) | PreviewBody::Table(_)) && rich.is_none();
        let formatted = if rich.is_some() || matches!(&model.body, PreviewBody::Empty) || (matches!(&model.body, PreviewBody::Image) && model.text.trim_matches(text_whitespace).is_empty()) { None }
            else { Some(if let PreviewBody::Link(link) = &model.body { literal(&link.detail, &model, cx) }
            else if let PreviewBody::Color(color) = &model.body { literal(&color.original, &model, cx) }
            else if matches!(&model.body, PreviewBody::Contact) { super::preview_content::contact_state(&model, cx) }
            else { text_literal(&model.display_text, &model, cx) }) };
        let link = if let PreviewBody::Link(link) = &model.body { Some(literal(&link.href, &model, cx)) } else { None };
        let image_name = model.files.first().filter(|_| matches!(&model.body, PreviewBody::Image)).map(|file| normal_flow_text(SharedString::from(file.name.clone())));
        let release = cx.on_release_in(window, |this: &mut Self, window, cx| { this.dispose(window, cx); this.return_to_panel(cx); });
        let configuration = cx.global::<ConfigurationGlobal>().0.clone();
        let subscriptions = vec![
            cx.observe_in(&configuration, window, |_, configuration, window, cx| {
                crate::ui::theme::apply(configuration.read(cx).projected.settings.color_mode, window.appearance(), cx); cx.notify();
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                let mode = cx.global::<ConfigurationGlobal>().0.read(cx).projected.settings.color_mode;
                crate::ui::theme::apply(mode, window.appearance(), cx); cx.notify();
            }),
        ];
        let focus = cx.focus_handle(); window.focus(&focus, cx);
        Self { content: Some(Content { model, source, raw: None, formatted, rich, rich_unavailable, link, image, image_error, dimensions, image_name }),
            services, owner, owner_window, request, plain: false, copying: false, copied: None, disposed: false, focus,
            rasters, cancel, image_task: None, scroll: ScrollHandle::new(), _release: release, _subscriptions: subscriptions }
    }
    fn return_to_panel(&self, cx: &mut App) {
        let (owner, parent, request) = (self.owner.clone(), self.owner_window, self.request);
        cx.defer(move |cx| { let _ = parent.update(cx, |_, window, cx| { let _ = owner.update(cx, |owner, cx| owner.finish_details(request, window, cx)); }); });
    }
    fn dispose(&mut self, window: &mut Window, cx: &mut App) {
        if self.disposed { return; }
        self.disposed = true; self.cancel.cancel(); self.image_task = None; TextSelection::clear(window, cx); self.content = None;
        if let Err(error) = self.rasters.clear(window) { tracing::error!(%error, "释放详情绘制资源失败"); }
    }
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dispose(window, cx); window.remove_window(); self.return_to_panel(cx);
    }
    pub(super) fn prune_rasters(&mut self, window: &mut Window) {
        self.prune_rasters_with(None, window);
    }
    fn prune_rasters_with(&mut self, incoming: Option<Digest>, window: &mut Window) {
        let mut visible: HashSet<_> = incoming.into_iter().collect();
        if let Some(sources) = self.content.as_ref().and_then(Content::image_sources) { sources.retain_pixels(&mut visible); }
        if let Err(error) = self.rasters.retain(&visible, window) { tracing::error!(%error, "回收详情绘制资源失败"); }
    }
    fn start_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sources) = self.content.as_ref().and_then(Content::image_sources) else { return; };
        let (mut responses, worker) = super::rich::load_images(&self.services.runtime, self.services.images.clone(), sources, cx.entity().downgrade(), image_spec(window), self.cancel.clone(), ImagePriority::Interactive);
        self.image_task = Some(cx.spawn_in(window, async move |weak, cx| {
            let worker = AbortOnDropHandle::new(worker);
            while let Some(response) = responses.recv().await {
                if !matches!(weak.update_in(cx, |this, window, cx| this.accept_image(response, window, cx)), Ok(true)) { return; }
            }
            if let Err(error) = worker.await { if !error.is_cancelled() {
                tracing::error!(%error, "详情图片任务异常结束");
                let _ = weak.update_in(cx, |this, window, cx| { if let Some(sources) = this.content.as_ref().and_then(Content::image_sources) { sources.fail_pending("图片加载任务异常结束".into()); } this.prune_rasters(window); cx.notify(); });
            } }
        }));
    }
    fn accept_image(&mut self, response: super::rich::ImageResponse, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.disposed || self.cancel.is_cancelled() { return false; }
        let Some(sources) = self.content.as_ref().and_then(Content::image_sources) else { return false; };
        if !sources.wants(&response.key, response.generation) { cx.notify(); return true; }
        self.prune_rasters_with(response.pixels.as_ref().ok().map(|frame| frame.pixels.id()), window);
        let frame = response.pixels.and_then(|display| {
            let id = display.pixels.id(); let image = self.rasters.acquire(display.pixels)?; let pin = self.rasters.pin(id)?;
            Ok(super::rich::RichImageFrame { id, image, pin: Some(pin), natural_dimensions: display.natural_dimensions, animation: display.animation, next_frame: display.next_frame, checkpoint: display.checkpoint })
        });
        if let Err(error) = &frame { tracing::warn!(%error, "详情图片无法预览"); }
        if let Some(content) = &mut self.content {
            if let Some(image) = &content.image { image.sources().accept(response.key, response.generation, frame); }
            else if let Some(rich) = &content.rich { rich.accept_image(response.key, response.generation, frame, cx); }
        }
        self.prune_rasters(window); cx.notify(); true
    }
}
fn literal(text: &Arc<str>, owner: &Arc<PreviewModel>, cx: &mut App) -> Entity<TextViewState> {
    shared_literal(SharedString::from(text.clone()), owner, cx)
}
fn text_literal(text: &Arc<str>, owner: &Arc<PreviewModel>, cx: &mut App) -> Entity<TextViewState> {
    shared_literal(super::preview_content::display_text(text), owner, cx)
}
fn shared_literal(source: SharedString, owner: &Arc<PreviewModel>, cx: &mut App) -> Entity<TextViewState> {
    super::preview_content::literal_state(source, owner, cx)
}
impl DetailsView {
    fn toggle(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Retained text states outlive the visible representation. Retire its
        // selection even when keyboard activation has no pointer-down event.
        TextSelection::clear(window, cx);
        self.plain = !self.plain;
        if let Some(content) = &mut self.content {
            if self.plain && content.raw.is_none() { content.raw = Some(text_literal(&content.model.text, &content.model, cx)); }
            if self.plain { if let Some(rich) = content.rich.as_ref() { rich.release_images(); } }
            if self.plain { if let Some(image) = &content.image { image.sources().release_visible(); } }
        }
        if self.plain { self.prune_rasters(window); }
        self.scroll.set_offset(Point::default()); cx.notify();
    }
    fn text_view(&self, state: &Entity<TextViewState>, p: Palette, monospace: bool) -> Div {
        div().min_w_0().text_size(px(14.0)).line_height(relative(1.65)).text_color(p.text_primary).tab_size(4)
            .when(monospace, |text| text.font_family("monospace").line_height(relative(1.7)).whitespace_nowrap().p(px(10.0)).rounded(px(8.0)).bg(p.bg_secondary))
            .child(TextView::new(state).selectable(true).scrollable(false)
                .require_native_layout("文本显示失败")
                .style(TextViewStyle::default().with_foreground(p.text_primary)))
    }
    fn text_preview(&self, state: &Entity<TextViewState>, text: &str, code: bool, p: Palette) -> Div {
        let compact = super::preview_content::compact_text(text);
        div().w_full().min_w_0().flex().flex_col().gap(px(16.0))
            .when(text.trim_matches(text_whitespace).is_empty(), |body| body.child(div().text_size(px(11.0)).text_color(p.text_secondary).child("空白文本")))
            .child(self.text_view(state, p, code)
                .when(compact, |body| body.font_weight(FontWeight::MEDIUM).line_height(relative(if code { 1.7 } else { 1.5 })))
                .map(|element| super::rich::automatic_text(element, text)))
    }
    fn copy_color(&mut self, rgb: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.copying || self.services.is_shutting_down() { return; }
        let Some(content) = &self.content else { return; };
        let PreviewBody::Color(color) = &content.model.body else { return; };
        let value = if rgb { color.rgb.clone() } else { color.hex.clone() };
        let services = self.services.clone(); self.copying = true;
        self.copied = None;
        let owner = self.owner.clone(); let desktop = cx.global::<crate::host::DesktopGlobal>().0.downgrade();
        let task = services.runtime.spawn({ let service = services.clipboard.clone(); async move {
            service.write(ClipboardSnapshot { content: TextContent { text: Some(value), ..TextContent::default() }, image: None }, &CancellationToken::new()).await
        }});
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                this.copying = false;
                let active = this.content.is_some() && !this.services.is_shutting_down() && this.owner.upgrade().is_some_and(|owner| {
                    let owner = owner.read(cx);
                    owner.can_present_feedback(window) && owner.details.as_ref().is_some_and(|details| details.request == this.request)
                });
                if result.is_ok() && active {
                    this.copied = Some(if rgb { "RGB" } else { "HEX" });
                }
                cx.notify();
            });
            if let Err(error) = result {
                tracing::warn!(%error, "复制详情颜色失败");
                super::present_operation_failure(&owner, &desktop, format!("复制颜色失败：{error}").into(), cx);
            }
        }).detach(); cx.notify();
    }
}
impl Render for DetailsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(cx.global::<ConfigurationGlobal>().0.read(cx).projected.settings.color_mode, window);
        let Some(content) = &self.content else { return div().into_any_element(); };
        if !self.plain && content.image.is_none() {
            if let Some(sources) = content.image_sources() { sources.set_spec(image_spec(window)); }
        }
        let model = &content.model;
        let source = content.source.as_ref().and_then(|source| self.owner.upgrade().map(|owner| owner.read(cx).source_label(source, p)));
        let can_toggle = !model.text.is_empty() && (matches!(&model.body, PreviewBody::Rich(_) | PreviewBody::Table(_)) || model.text != model.display_text);
        let summary = if let Some(image) = model.image.as_ref().filter(|_| matches!(&model.body, PreviewBody::Image)) {
            if let Some((width, height)) = content.dimensions { format!("图片 · {width} × {height} · {}", byte_size(image.encoded_bytes)) }
            else { format!("图片 · {}", byte_size(image.encoded_bytes)) }
        } else if matches!(&model.body, PreviewBody::Files) { format!("{} 个文件", model.files_total) } else { model.summary.to_string() };
        let body = if self.plain {
            content.raw.as_ref().map(|raw| self.text_preview(raw, &model.text, matches!(model.kind, PreviewKind::Code | PreviewKind::Json), p)).unwrap_or_else(div)
        } else { match &model.body {
            PreviewBody::Rich(_) | PreviewBody::Table(_) | PreviewBody::Files if content.rich.is_some() => content.rich.as_ref().unwrap().render_details(window, cx),
            PreviewBody::Image => {
                let (width, height) = content.dimensions.unwrap_or((400, 100));
                let fitted = layout::fit_image(width, height, size((window.viewport_size().width - px(48.)).max(px(1.)), (window.viewport_size().height - px(120.)).max(px(1.))));
                div().flex().flex_col().gap(px(16.0))
                    .child(div().relative().w_full().h(fitted.height.max(px(80.))).flex_shrink_0().flex().items_center().justify_center().rounded(px(8.0)).overflow_hidden()
                        .child(super::preview_content::checkerboard(p, 8.0))
                        .when_some(content.image.as_ref(), |frame, image| frame.child(div().relative().w(fitted.width).h(fitted.height)
                            .child(image.render(p).absolute().inset_0())))
                        .when_some(content.image_error.clone(), |frame, error| frame.child(super::preview_content::image_failure(error, p).absolute().inset_0())))
                    .when_some(content.formatted.as_ref(), |body, caption| body.child(div().border_t_1().border_color(p.border_light).pt(px(6.0)).child(div().text_size(px(10.0)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_secondary).child("附带文本"))
                        .child(self.text_view(caption, p, false).mt(px(2.0)).text_size(px(12.0)).line_height(relative(1.5))
                            .map(|text| super::rich::automatic_text(text, &model.text)))))
                    .when_some(model.files.first(), |body, file| {
                        let path = SharedString::from(file.path.clone());
                        body.child(div().id("details-image-file").min_w_0().flex().items_center().gap(px(4.0)).text_size(px(11.0)).text_color(p.text_secondary)
                            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(window, cx))
                            .child(crate::ui::icon("file-outline", 11.0).flex_shrink_0()).child(div().flex_1().min_w_0().text_ellipsis()
                                .child(content.image_name.as_ref().expect("loaded image files have a prepared name").clone())))
                    })
            },
            PreviewBody::Contact => div().flex_1().flex().flex_col().items_center().gap(px(16.0)).child(div().mt_auto().text_color(p.type_red).child(crate::ui::icon(if model.kind == PreviewKind::Email { "at" } else { "phone-outline" }, 40.0)))
                .child(self.text_view(content.formatted(), p, false).w_full().text_size(px(21.0)).font_weight(FontWeight::MEDIUM)
                    .map(|element| super::rich::automatic_text(element, &model.text).text_center())),
            PreviewBody::Link(link) => div().flex_1().flex().flex_col().gap(px(16.0))
                .child(div().flex().items_start().gap(px(8.0)).text_size(px(18.0)).text_color(p.type_orange)
                    .child(crate::ui::icon("link-variant", 18.0).flex_shrink_0().mt(px(5.0)))
                    .child(div().flex_1().min_w_0().font_weight(FontWeight::BOLD).overflow_wrap(OverflowWrap::Anywhere).child(SharedString::from(link.host.clone()))))
                .child(self.text_view(content.formatted(), p, false).map(|element| super::rich::automatic_text(element, &link.detail)))
                .when_some(content.link.as_ref(), |body, state| body.child(self.text_view(state, Palette { text_primary: p.text_secondary, ..p }, false).mt_auto().text_size(px(11.0))
                    .map(|element| super::rich::automatic_text(element, &link.href)))),
            PreviewBody::Color(color) => div().flex_1().flex().flex_col().gap(px(16.0))
                .child(super::preview_content::color_swatch(color, p).flex_1())
                .child(self.text_view(content.formatted(), p, false).flex_shrink_0().font_family("monospace").text_size(px(18.0)).text_center())
                .child(div().flex_shrink_0().font_family("monospace").text_size(px(12.0)).text_center().text_color(p.text_secondary).child(SharedString::from(color.rgb.clone())))
                .child(div().flex_shrink_0().flex().justify_center().gap(px(8.0)).children([false, true].into_iter().map(|rgb| {
                    super::preview_content::color_copy_button(if rgb { "details-copy-rgb" } else { "details-copy-hex" }, rgb, self.copying, p)
                        .on_click(cx.listener(move |this, _, window, cx| this.copy_color(rgb, window, cx)))
                })))
                .child(div().id("details-copy-status").role(Role::Status).flex_shrink_0().h(px(16.0)).line_height(px(16.0)).text_center().text_size(px(11.0)).text_color(p.text_secondary).child(self.copied.map(|format| format!("已复制 {format}")).unwrap_or_default())),
            PreviewBody::Empty => div().flex().items_center().justify_center().text_color(p.text_secondary).child("暂无可预览内容"),
            _ => self.text_preview(content.formatted(), &model.display_text, matches!(model.kind, PreviewKind::Code | PreviewKind::Json), p),
        } };
        let preview = div().w_full().min_w_0().min_h_full().flex().flex_col().gap(px(16.0))
            .when_some(model.notice, |body, notice| body.child(div().flex_shrink_0().text_size(px(11.0)).text_color(p.text_secondary).child(notice.text())))
            .when(!self.plain && model.notice != Some(PreviewNotice::Excerpt) && content.rich.as_ref().is_some_and(|rich| rich.truncated), |body| body.child(div().id("details-rich-excerpt-status").role(Role::Status).flex_shrink_0().text_size(px(11.0)).text_color(p.text_secondary).child(PreviewNotice::Excerpt.text())))
            .child(body)
            .when(content.rich_unavailable && !self.plain, |body| body.child(div().id("details-rich-status").role(Role::Status).flex_shrink_0().text_size(px(11.0)).text_color(p.text_secondary).child("富文本预览不可用")))
            .when(!self.plain && content.rich.as_ref().is_some_and(|rich| rich.image_failed()), |body| body.child(div().id("details-rich-image-status").role(Role::Status).flex_shrink_0().text_size(px(11.0)).text_color(p.text_secondary).child("部分图片无法预览")));
        div().id("clipboard-details").size_full().min_w_0().min_h_0().p(px(24.)).flex().flex_col().gap(px(12.0)).bg(p.window).text_color(p.text_primary)
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if window.has_active_dialog(cx) { return; }
                let key = &event.keystroke;
                if key.key == "escape" || (key.key == "w" && (key.modifiers.control || key.modifiers.platform)) {
                    window.prevent_default(); cx.stop_propagation(); this.close(window, cx);
                }
            }))
            .child(div().flex_shrink_0().flex().items_center().justify_between().gap(px(8.0)).child(div().flex_1().min_w_0().text_ellipsis().text_size(px(12.0)).text_color(p.text_secondary).child(summary))
                .when(can_toggle, |bar| bar.child(Button::new("details-toggle").accessibility_label(if self.plain { "排版" } else { "原文" }).selected(self.plain)
                    .flex_shrink_0().text_size(px(12.0)).px(px(8.0)).py(px(4.0)).rounded(px(4.0)).border_1().border_color(p.control_border).child(if self.plain { "排版" } else { "原文" }).on_click(cx.listener(Self::toggle))))
                .when_some(source, |bar, source| bar.child(source)))
            .child(div().id("details-scroll").w_full().flex_1().min_h_0().overflow_scroll().track_scroll(&self.scroll)
                .on_scroll(|_, _, cx| cx.stop_propagation()).child(preview)).into_any_element()
    }
}
fn byte_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{bytes} B") } else if bytes < 1024 * 1024 { format!("{:.1} KB", bytes as f64 / 1024.0) } else { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) }
}
