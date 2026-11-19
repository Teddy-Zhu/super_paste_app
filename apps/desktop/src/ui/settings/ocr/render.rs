use super::*;
use gpui_kit::component::{input::Input, select::Select};
use catalog::{ModelStage, ModelState};
use crate::ui::widgets::{TextButton, Toggle};
use super::local::ModelAction;

impl OcrPane {
    fn local_controls(&self, p: Palette, cx: &mut Context<Self>) -> Div {
        let saving = self.saving.is_some(); let busy = self.local_busy(); let model_id = &self.draft.value.settings.ocr_local_model;
        let info = catalog::get_model(model_id); let status = self.statuses.get(model_id);
        let downloading = status.is_some_and(|status| status.state == ModelState::Downloading);
        let ready = status.is_some_and(|status| status.state == ModelState::Ready);
        let error = status.is_some_and(|status| status.state == ModelState::Error);
        let status_text = match status {
            Some(status) => match status.state {
                ModelState::Ready => "已下载，可离线使用".to_owned(),
                ModelState::Error => status.error.clone().unwrap_or_else(|| "下载失败".to_owned()),
                ModelState::Downloading => match status.stage.unwrap_or(ModelStage::Downloading) {
                    ModelStage::Downloading => "下载中", ModelStage::Extracting => "解压中", ModelStage::PreparingDetector => "准备文字检测模型", ModelStage::Verifying => "校验中",
                }.to_owned(),
                ModelState::NotDownloaded => "尚未下载".to_owned(),
            },
            None if self.status_task.is_some() => "正在读取模型状态…".to_owned(),
            None => "模型状态不可用".to_owned(),
        };
        let weak = cx.entity().downgrade(); let redownload = weak.clone(); let remove = weak.clone(); let download = weak.clone(); let cancel = weak.clone(); let confirm = weak.clone(); let dismiss = weak.clone();
        let description = match info { Ok(info) => format!("{} · 下载大小 {:.1} MB", info.description, info.size as f64 / 1048576.0), Err(error) => error };
        let mut body = forms::setting_info("本地 OCR 模型", "启用自动 OCR 或首次手动识别时下载所选模型，下载后可离线使用。", p)
            .child(div().mt(px(12.0)).w_full().child(Select::new(&self.local_model).intrinsic_size(true).w_full().h(px(32.0)).accessibility_label("本地 OCR 模型").disabled(saving || busy)))
            .child(note(description, p.text_secondary, 10.0))
            .child(note(status_text, if error { p.danger } else { p.text_secondary }, 10.0));
        if let Some(status) = status.filter(|_| downloading) {
            let total = if status.total_bytes > 0 { status.total_bytes } else { catalog::get_model(model_id).map(|info| info.size).unwrap_or(0) };
            let progress = if total > 0 { ((u128::from(status.received_bytes) * 100) / u128::from(total)).min(100) as u32 } else { 0 };
            body = body.child(div().mt(px(12.0)).flex().flex_col().gap(px(8.0))
                .child(div().id("model-download-progress").role(Role::ProgressIndicator).w_full().h(px(8.0)).rounded_full().bg(p.bg_tertiary).overflow_hidden()
                    .aria_label("模型下载进度").child(div().h_full().w(relative(progress as f32 / 100.0)).bg(p.primary)))
                .child(note(format!("{progress}% · {:.1} MB / {:.1} MB", status.received_bytes as f64 / 1048576.0, total as f64 / 1048576.0), p.text_secondary, 0.0)));
        }
        body = body.child(actions()
            .when(ready && !downloading, |row| row
                .child(TextButton::new("ocr-model-redownload", if self.local_action.is_some() { "正在处理…" } else { "强制重新下载" }, p).neutral().disabled(saving || busy)
                    .on_click(move |_, window, cx| { let _ = redownload.update(cx, |this, cx| this.model_action(ModelAction::Redownload, window, cx)); }))
                .child(TextButton::new("ocr-model-delete", "删除模型", p).neutral().disabled(saving || busy)
                    .on_click(move |_, _, cx| { let _ = remove.update(cx, |this, cx| { this.confirm_delete = true; cx.notify(); }); })))
            .when(!ready && !downloading, |row| row.child(TextButton::new("ocr-model-download", if error { "重试下载" } else { "下载模型" }, p).neutral().disabled(saving || busy)
                .on_click(move |_, window, cx| { let _ = download.update(cx, |this, cx| this.model_action(ModelAction::Download, window, cx)); })))
            .when(downloading, |row| row.child(TextButton::new("ocr-model-cancel", "取消下载", p).neutral().disabled(self.cancelling)
                .on_click(move |_, window, cx| { let _ = cancel.update(cx, |this, cx| this.model_action(ModelAction::Cancel, window, cx)); }))));
        if self.confirm_delete {
            body = body.child(note("确定删除所选本地模型？删除当前启用模型将关闭自动 OCR。", p.text_secondary, 10.0)
                .child(actions().child(TextButton::new("ocr-model-confirm-delete", "确认删除", p).neutral().disabled(busy)
                    .on_click(move |_, window, cx| { let _ = confirm.update(cx, |this, cx| this.model_action(ModelAction::Delete, window, cx)); }))
                    .child(TextButton::new("ocr-model-dismiss-delete", "取消", p).neutral().on_click(move |_, _, cx| { let _ = dismiss.update(cx, |this, cx| { this.confirm_delete = false; cx.notify(); }); }))));
        }
        body.when_some(self.local_error.clone(), |body, error| body.child(note(error, p.danger, 8.0)))
            .when_some(self.status_error.clone(), |body, error| body.child(note(error, p.danger, 8.0))
            .child(TextButton::new("ocr-model-refresh", "刷新模型状态", p).neutral().disabled(self.status_task.is_some()).on_click(move |_, window, cx| { let _ = weak.update(cx, |this, cx| this.refresh_models(window, cx)); })))
            .when_some(self.local_message.clone(), |body, message| body.child(note(message, p.success_text, 10.0)))
    }
    fn ai_controls(&mut self, provider: AiProvider, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        let saving = self.saving.is_some(); let index = index(provider); let fields = &self.profiles[index]; let id = credential(provider);
        let has_saved = self.draft.has_saved_secret(id); let has_key = has_saved || self.draft.secret_is_set(id); let profile = self.profile(provider);
        let url_empty = profile.base_url.is_empty(); let model_empty = profile.model.is_empty(); let jobs = &self.ai_jobs[index];
        let placeholder = if has_saved && self.draft.secret_is_keep(id) { "已保存的密钥留空即可继续使用" } else { "输入 API Key" };
        if fields.key.read(cx).placeholder_text() != placeholder { fields.key.update(cx, |state, cx| state.set_placeholder(placeholder, window, cx)); }
        fields.model.update(cx, |picker, _| picker.present(saving, p));
        let clear = cx.entity().downgrade(); let fetch = clear.clone(); let test = clear.clone();
        let mut rows = Vec::with_capacity(5);
        if provider == AiProvider::Openai {
            rows.push(row(p, true, window).child(forms::setting_info("OpenAI API", "选择服务提供的图片识别接口", p))
                .child(Select::new(&self.api).intrinsic_size(true).w(px(220.0)).h(px(32.0)).accessibility_label("OpenAI API").disabled(saving)));
        }
        rows.push(multiline("Base URL", "支持自定义服务地址和 /v1 前缀，不包含具体接口路径。", p)
            .child(div().mt(px(12.0)).child(text_field(&fields.url, "Base URL", saving, p, window, cx))));
        rows.push(multiline("API Key", "所选服务的 API 密钥", p)
            .child(div().mt(px(12.0)).child(text_field(&fields.key, "API Key", saving, p, window, cx)))
            .child(div().flex().items_start().child(TextButton::new("ocr-ai-clear-key", "清除密钥", p).neutral().disabled(saving)
                .on_click(move |_, window, cx| { let _ = clear.update(cx, |this, cx| this.clear_ai_key(provider, window, cx)); }))));
        rows.push(multiline("模型名称", "选择或输入支持图片识别的模型名称、模型 ID。", p)
            .child(actions().child(TextButton::new("ocr-ai-models", if jobs.fetching() { "正在拉取…" } else { "拉取模型" }, p).neutral()
                .disabled(saving || jobs.fetching() || !has_key || url_empty).on_click(move |_, window, cx| { let _ = fetch.update(cx, |this, cx| this.start_ai(provider, false, window, cx)); })))
            .child(div().mt(px(12.0)).w_full().child(fields.model.clone()))
            .when_some(jobs.model_error.clone(), |body, error| body.child(note(error, p.danger, 8.0)))
            .when_some(jobs.model_message.clone(), |body, message| body.child(note(message, p.text_secondary, 10.0))));
        rows.push(multiline("测试连通性", "使用当前配置发送一次图片请求，会产生少量 Token 用量。", p)
            .child(actions().child(TextButton::new("ocr-ai-test", if jobs.testing() { "正在检测…" } else { "测试连通性" }, p).neutral()
                .disabled(saving || jobs.testing() || !has_key || url_empty || model_empty).on_click(move |_, window, cx| { let _ = test.update(cx, |this, cx| this.start_ai(provider, true, window, cx)); })))
            .when_some(jobs.test_error.clone(), |body, error| body.child(note(error, p.danger, 8.0)))
            .when_some(jobs.test_message.clone(), |body, message| body.child(note(message, p.success_text, 10.0))));
        rows
    }
    fn remote_controls(&self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        let saving = self.saving.is_some();
        vec![
            multiline("调用地址", "远程 OCR API 地址，默认使用 POST 请求", p).child(div().mt(px(12.0)).child(text_field(&self.remote_url, "调用地址", saving, p, window, cx))),
            row(p, true, window).child(forms::setting_info("请求格式", "json: base64 字段；form: 文件字段上传", p))
                .child(Select::new(&self.remote_format).intrinsic_size(true).w(px(220.0)).h(px(32.0)).accessibility_label("请求格式").disabled(saving)),
            multiline("图片字段名称", "例如 `image`，JSON 模式下为 base64 字段，表单模式下为文件字段", p).child(div().mt(px(12.0)).child(text_field(&self.image_field, "图片字段名称", saving, p, window, cx))),
            multiline("额外调用参数 (JSON)", "仅支持 JSON 对象，所有字段会与图片字段一起提交", p)
                .child(div().mt(px(12.0)).w_full().child(self.extra_params(saving, p, window, cx)))
                .when_some(self.validate_extra().err(), |body, error| body.child(note(error.to_string(), p.danger, 8.0))),
            multiline("返回结果读取路径", "仅支持 JSON 响应。示例：`data.result` 对应 `res.data.result`", p).child(div().mt(px(12.0)).child(text_field(&self.result_path, "返回结果读取路径", saving, p, window, cx))),
        ]
    }
}
impl Render for OcrPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let saving = self.saving.is_some(); let busy = self.local_busy(); let unsaved = self.draft.has_unsaved(); let mode = self.draft.value.settings.ocr_mode;
        let toggle = cx.entity().downgrade(); let save = toggle.clone(); let reset = toggle.clone();
        let mut rows = vec![
            row(p, false, window).child(forms::setting_info("自动 OCR", "保存图片到历史记录时自动执行 OCR 识别", p))
                .child(Toggle::new("ocr-auto-enable", "自动 OCR", self.draft.value.settings.enable_local_ocr, p).disabled(saving)
                    .on_change(move |enabled, window, cx| { let _ = toggle.update(cx, |this, cx| this.change(ConfigChange::EnableLocalOcr(enabled), window, cx)); })),
            row(p, true, window).child(forms::setting_info("OCR 方式", "支持本地模型、通用远程接口及 AI 视觉识别", p))
                .child(Select::new(&self.mode).intrinsic_size(true).w(px(220.0)).h(px(32.0)).accessibility_label("OCR 方式").disabled(saving || busy)),
        ];
        match mode {
            OcrMode::Local => rows.push(row(p, true, window).items_start().child(self.local_controls(p, cx))),
            OcrMode::Remote => rows.extend(self.remote_controls(p, window, cx)),
            OcrMode::Openai => rows.extend(self.ai_controls(AiProvider::Openai, p, window, cx)),
            OcrMode::Claude => rows.extend(self.ai_controls(AiProvider::Claude, p, window, cx)),
        }
        rows.push(row(p, true, window).items_start().child(forms::setting_info("保存配置", "修改后点击保存，配置才会写入并生效", p)
            .child(actions().child(TextButton::new("ocr-save", if saving { "正在保存…" } else { "保存 OCR 配置" }, p).primary().disabled(!unsaved || saving || busy || self.draft.pending())
                .on_click(move |_, window, cx| { let _ = save.update(cx, |this, cx| this.save(window, cx)); }))
                .child(TextButton::new("ocr-discard", "放弃修改", p).neutral().disabled(!unsaved || saving || busy || self.draft.pending())
                    .on_click(move |_, window, cx| { let _ = reset.update(cx, |this, cx| this.discard(window, cx)); })))
            .when(unsaved, |body| body.child(note("有未保存修改", p.warning_text, 10.0)))
            .when_some(self.message.clone(), |body, message| body.child(note(message, p.success_text, 10.0)))
            .when_some(self.error.clone(), |body, error| body.child(note(error, p.danger, 8.0)))));
        forms::page("ocr-settings", window).child(forms::heading("OCR 设置", "配置本地、远程或 AI OCR 服务", p)).child(forms::settings_group(p).children(rows))
    }
}
fn actions() -> Div { div().mt(px(12.0)).flex().flex_wrap().gap(px(8.0)) }
fn note(text: impl Into<SharedString>, color: Hsla, top: f32) -> Div { div().mt(px(top)).text_size(px(12.0)).line_height(px(18.0)).text_color(color).child(text.into()) }
fn multiline(title: &'static str, description: &'static str, p: Palette) -> Div { forms::setting_row(p, true).items_stretch().flex_col().gap_0().child(forms::setting_info(title, description, p)) }
fn row(p: Palette, divided: bool, window: &Window) -> Div {
    forms::setting_row(p, divided).gap(px(if window.viewport_size().width < px(900.0) { 16.0 } else { 20.0 }))
}
fn text_field(input: &Entity<InputState>, label: &'static str, disabled: bool, p: Palette, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let focused = input.read(cx).focus_handle(cx).contains_focused(window, cx);
    field_frame(input.entity_id(), disabled, focused, p, window, cx)
        .child(Input::new(input).text_direction(gpui::TextDirection::Ltr).aria_label(label).disabled(disabled).appearance(false).focus_bordered(false)
            .w_full().h_auto().min_h(px(28.0)).px(px(9.0)).py(px(5.0))
            .text_size(px(13.0)).line_height(relative(1.4)).bg(transparent_black()).rounded(px(4.0)))
}

/// Use the same frame for text, textarea and editable model controls. Keyed
/// interaction state follows the rendered control; no pane-wide hover cache
/// or animation timer survives a control leaving the active page.
pub(super) fn field_frame(id: EntityId, disabled: bool, focused: bool, p: Palette, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let interaction = window.use_keyed_state(("ocr-field-hover", id), cx, |_, _| false);
    let hovered = *interaction.read(cx) && !disabled;
    let target = if !disabled && (focused || hovered) { p.primary } else { p.control_border };
    let border = gpui_kit::base::transition((ElementId::View(id), "ocr-field-border"), target,
        gpui_kit::base::Transition::new(std::time::Duration::from_millis(100)), window, cx);
    div().id(("ocr-field", id)).w_full().min_w_0().min_h(px(30.0)).rounded(px(4.0)).border_1()
        .border_color(border).bg(if disabled { p.bg_secondary } else if hovered && !focused { p.bg_tertiary } else { p.bg_primary })
        .when(disabled, |body| body.opacity(0.5))
        .on_hover(move |hovered, _, cx| interaction.update(cx, |value, cx| {
            if *value != *hovered { *value = *hovered; cx.notify(); }
        }))
}
