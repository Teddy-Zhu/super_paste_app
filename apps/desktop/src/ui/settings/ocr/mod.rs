mod ai;
mod local;
mod model_picker;
mod extra;
mod render;

use std::{collections::{BTreeMap, BTreeSet, VecDeque}, sync::Arc};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::{input::{InputEvent, InputState, TextareaState}, select::SelectEvent};
use super_paste_core::{Error, Result, events::AppEvent, ocr::catalog::{self, OcrModelStatus}};
use super_paste_format::{AiOcrConfig, AiProvider, ConfigChange, ConfigField, CredentialId, OcrMode, OpenaiApi, RemoteRequestFormat};
use uuid::Uuid;
use tokio_util::sync::{CancellationToken, DropGuard};
use super::PaneContext;
use crate::ui::{configuration::Feedback, draft::Draft, forms::{self, Choice, ChoiceState}, palette::Palette};
use model_picker::{ModelEdited, ModelPicker};

const FIELDS: [ConfigField; 12] = [ConfigField::EnableLocalOcr, ConfigField::OcrMode, ConfigField::OcrLocalModel, ConfigField::OcrRemoteUrl,
    ConfigField::OcrRemoteExtraParams, ConfigField::OcrRemoteImageFieldName, ConfigField::OcrRemoteRequestFormat, ConfigField::OcrRemoteResultPath,
    ConfigField::OpenaiConfig, ConfigField::ClaudeConfig, ConfigField::OpenaiApiKey, ConfigField::ClaudeApiKey];
struct ProfileInputs { url: Entity<InputState>, key: Entity<InputState>, model: Entity<ModelPicker> }
#[derive(Clone, Copy)]
enum TextField { RemoteUrl, RemoteImageField, RemoteResultPath, AiUrl(AiProvider), AiKey(AiProvider) }

pub struct OcrPane {
    context: PaneContext, draft: Draft, mode: Entity<ChoiceState<OcrMode>>, local_model: Entity<ChoiceState<String>>,
    api: Entity<ChoiceState<OpenaiApi>>, remote_format: Entity<ChoiceState<RemoteRequestFormat>>,
    remote_url: Entity<InputState>, image_field: Entity<InputState>, result_path: Entity<InputState>, extra: Entity<TextareaState>, profiles: [ProfileInputs; 2],
    extra_height: Pixels, extra_drag: Option<(Pixels, Pixels)>,
    saving: Option<Uuid>, error: Option<SharedString>, message: Option<SharedString>,
    statuses: BTreeMap<String, Arc<OcrModelStatus>>, status_updates: BTreeSet<String>,
    status_epoch: u64, status_again: bool, status_visible: bool,
    status_cancel: Option<DropGuard>, status_task: Option<Task<()>>,
    local_action: Option<Uuid>, cancelling: bool, confirm_delete: bool, local_error: Option<SharedString>, status_error: Option<SharedString>, local_message: Option<SharedString>,
    ai_jobs: [ai::Jobs; 2], ai_queue: VecDeque<ai::Intent>, ai_commits: BTreeMap<Uuid, ai::Intent>,
    _events: Task<()>, subscriptions: Vec<Subscription>,
}
impl OcrPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = Draft::new(context.configuration.clone(), FIELDS, cx);
        let settings = &draft.value.settings;
        let p = Palette::for_window(context.configuration.read(cx).projected.settings.color_mode, window);
        let mode = forms::choices(vec![Choice::new("本地模型", OcrMode::Local), Choice::new("远程接口", OcrMode::Remote),
            Choice::new("OpenAI 兼容接口", OcrMode::Openai), Choice::new("Claude 兼容接口", OcrMode::Claude)], &settings.ocr_mode, window, cx);
        let (model_items, initial_error) = match catalog::all_models() {
            Ok(models) => (models.iter().map(|model| Choice::new(format!("{} · {:.1} MB", model.name, model.size as f64 / 1048576.0), model.id.clone())).collect(), None),
            Err(error) => (Vec::new(), Some(SharedString::from(error))),
        };
        let local_model = forms::choices(model_items, &settings.ocr_local_model, window, cx);
        let api = forms::choices(vec![Choice::new("Chat Completions", OpenaiApi::Chat), Choice::new("Responses", OpenaiApi::Responses)], &settings.ocr_ai_configs.openai.api, window, cx);
        let remote_format = forms::choices(vec![Choice::new("JSON", RemoteRequestFormat::Json), Choice::new("表单 (multipart/form-data)", RemoteRequestFormat::Form)], &settings.ocr_remote_request_format, window, cx);
        let remote_url = input(&settings.ocr_remote_url, "https://example.com/api/ocr", window, cx);
        let image_field = input(&settings.ocr_remote_image_field_name, "image", window, cx);
        let result_path = input(&settings.ocr_remote_result_path, "data.result", window, cx);
        let extra = cx.new(|cx| TextareaState::new(window, cx).rows(5).default_value(settings.ocr_remote_extra_params.clone()).placeholder("{\"lang\":\"zh\",\"scene\":\"general\"}"));
        let profiles = [&settings.ocr_ai_configs.openai, &settings.ocr_ai_configs.claude].map(|profile| ProfileInputs {
            url: input(&profile.base_url, "", window, cx), key: cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("输入 API Key")),
            model: cx.new(|cx| ModelPicker::new(profile.model.clone(), p, window, cx)),
        });
        let mut events = context.services.events.subscribe();
        let event_task = cx.spawn_in(window, async move |weak, cx| {
            loop {
                match events.recv().await {
                    Ok(AppEvent::OcrModelChanged(status)) => { if weak.update_in(cx, |this, _, cx| {
                        if this.status_visible && this.status_task.is_some() && !this.status_updates.contains(&status.id) {
                            this.status_updates.insert(status.id.clone());
                        }
                        if let Some(current) = this.statuses.get_mut(&status.id) { *current = status; }
                        else { this.statuses.insert(status.id.clone(), status); }
                        cx.notify();
                    }).is_err() { break; } }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => { if weak.update_in(cx, |this, window, cx| this.refresh_models(window, cx)).is_err() { break; } }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {},
                }
            }
        });
        let mut pane = Self {
            context, draft, mode, local_model, api, remote_format, remote_url, image_field, result_path, extra, profiles,
            extra_height: px(110.0), extra_drag: None,
            saving: None, error: None, message: None, statuses: BTreeMap::new(), status_updates: BTreeSet::new(), status_epoch: 0,
            status_again: false, status_visible: window.is_visible(), status_cancel: None, status_task: None,
            local_action: None, cancelling: false, confirm_delete: false, local_error: initial_error, status_error: None, local_message: None,
            ai_jobs: [ai::Jobs::default(), ai::Jobs::default()], ai_queue: VecDeque::new(), ai_commits: BTreeMap::new(), _events: event_task, subscriptions: Vec::new(),
        };
        pane.subscribe(window, cx); pane.refresh_models(window, cx); pane
    }
    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions.push(cx.observe_in(&self.context.configuration, window, |this, _, window, cx| this.synchronize(window, cx)));
        self.subscriptions.push(cx.subscribe_in(&self.context.configuration, window, |this, _, feedback: &Feedback, window, cx| this.acknowledge(feedback, window, cx)));
        self.subscriptions.push(cx.observe_window_appearance(window, |_, _, cx| cx.notify()));
        self.subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() && this.extra_drag.take().is_some() { cx.notify(); }
        }));
        if let Some(owner) = self.context.owner.upgrade() {
            self.subscriptions.push(cx.observe_in(&owner, window, |this, _, window, cx| {
                let visible = window.is_visible() && this.context.is_active(cx.entity_id(), cx);
                this.set_status_visible(visible, window, cx);
            }));
        }
        self.subscriptions.push(cx.observe_window_visibility(window, |this, _, window, cx| {
            let visible = window.is_visible() && this.context.is_active(cx.entity_id(), cx);
            this.set_status_visible(visible, window, cx);
            if !window.is_visible() && this.extra_drag.take().is_some() { cx.notify(); }
        }));
        self.subscriptions.push(cx.subscribe_in(&self.mode, window, |this, _, event: &SelectEvent<Vec<Choice<OcrMode>>>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event { this.change(ConfigChange::OcrMode(*value), window, cx); }
        }));
        self.subscriptions.push(cx.subscribe_in(&self.local_model, window, |this, _, event: &SelectEvent<Vec<Choice<String>>>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event { this.change(ConfigChange::OcrLocalModel(value.clone()), window, cx); }
        }));
        self.subscriptions.push(cx.subscribe_in(&self.api, window, |this, _, event: &SelectEvent<Vec<Choice<OpenaiApi>>>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event { this.edit_profile(AiProvider::Openai, |profile| profile.api = *value, window, cx); }
        }));
        self.subscriptions.push(cx.subscribe_in(&self.remote_format, window, |this, _, event: &SelectEvent<Vec<Choice<RemoteRequestFormat>>>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event { this.change(ConfigChange::OcrRemoteRequestFormat(*value), window, cx); }
        }));
        let mut inputs = vec![(self.remote_url.clone(), TextField::RemoteUrl), (self.image_field.clone(), TextField::RemoteImageField), (self.result_path.clone(), TextField::RemoteResultPath)];
        for provider in [AiProvider::Openai, AiProvider::Claude] {
            let fields = &self.profiles[index(provider)];
            inputs.push((fields.url.clone(), TextField::AiUrl(provider))); inputs.push((fields.key.clone(), TextField::AiKey(provider)));
            self.subscriptions.push(cx.subscribe_in(&fields.model, window, move |this, _, event: &ModelEdited, window, cx| {
                this.edit_profile(provider, |profile| profile.model = event.0.trim().to_owned(), window, cx);
            }));
        }
        for (input, field) in inputs {
            self.subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) { this.text_changed(field, input.read(cx).value().as_ref(), window, cx); }
                else { cx.notify(); }
            }));
        }
        self.subscriptions.push(cx.subscribe_in(&self.extra, window, |this, extra, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) { this.change(ConfigChange::OcrRemoteExtraParams(extra.read(cx).value().to_string()), window, cx); }
        }));
    }
    fn text_changed(&mut self, field: TextField, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        match field {
            TextField::RemoteUrl => self.change(ConfigChange::OcrRemoteUrl(value.trim().to_owned()), window, cx),
            TextField::RemoteImageField => self.change(ConfigChange::OcrRemoteImageFieldName(value.trim().to_owned()), window, cx),
            TextField::RemoteResultPath => self.change(ConfigChange::OcrRemoteResultPath(value.trim().to_owned()), window, cx),
            TextField::AiUrl(provider) => self.edit_profile(provider, |profile| profile.base_url = value.trim().to_owned(), window, cx),
            TextField::AiKey(provider) => match self.draft.set_secret_input(credential(provider), value.trim()) {
                Ok(true) => { self.invalidate_profile(provider, true, cx); self.edited(window, cx); }
                Ok(false) => {}, Err(error) => { self.error = Some(error.to_string().into()); cx.notify(); }
            },
        }
    }
    fn change(&mut self, change: ConfigChange, window: &mut Window, cx: &mut Context<Self>) {
        let field = change.field();
        match self.draft.set(change) {
            Ok(true) => {
                if field == ConfigField::OcrMode { self.extra_drag = None; for provider in [AiProvider::Openai, AiProvider::Claude] { self.invalidate_profile(provider, true, cx); } }
                if matches!(field, ConfigField::OcrLocalModel | ConfigField::OcrMode) { self.confirm_delete = false; self.local_message = None; }
                self.edited(window, cx);
            }
            Ok(false) => {}, Err(error) => { self.error = Some(error.to_string().into()); cx.notify(); }
        }
    }
    fn edit_profile(&mut self, provider: AiProvider, update: impl FnOnce(&mut AiOcrConfig), window: &mut Window, cx: &mut Context<Self>) {
        let old = self.profile(provider).clone(); let mut value = old.clone(); update(&mut value);
        if value == old { return; }
        let list_changed = old.base_url != value.base_url;
        let change = match provider { AiProvider::Openai => ConfigChange::OpenaiConfig(value), AiProvider::Claude => ConfigChange::ClaudeConfig(value) };
        match self.draft.set(change) {
            Ok(true) => { self.invalidate_profile(provider, list_changed, cx); self.edited(window, cx); }
            Ok(false) => {}, Err(error) => { self.error = Some(error.to_string().into()); cx.notify(); }
        }
    }
    fn edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message = None; self.error = None; self.synchronize(window, cx); cx.notify();
    }
    fn profile(&self, provider: AiProvider) -> &AiOcrConfig {
        match provider { AiProvider::Openai => &self.draft.value.settings.ocr_ai_configs.openai, AiProvider::Claude => &self.draft.value.settings.ocr_ai_configs.claude }
    }
    fn selected_provider(&self) -> Option<AiProvider> { match self.draft.value.settings.ocr_mode { OcrMode::Openai => Some(AiProvider::Openai), OcrMode::Claude => Some(AiProvider::Claude), _ => None } }
    fn synchronize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.context.configuration.read(cx).state.clone();
        let current = &self.draft.value.settings.ocr_ai_configs;
        let incoming = &state.snapshot.value.settings.ocr_ai_configs;
        let endpoints_changed = [current.openai.base_url != incoming.openai.base_url, current.claude.base_url != incoming.claude.base_url];
        let changed = self.draft.synchronize(&state);
        let mode_changed = changed.contains(&ConfigField::OcrMode);
        for provider in [AiProvider::Openai, AiProvider::Claude] {
            let key_changed = changed.contains(&credential(provider).field());
            if mode_changed || changed.contains(&profile_field(provider)) || key_changed {
                // Model IDs and the recognition API do not change the models endpoint.
                self.invalidate_profile(provider, mode_changed || key_changed || endpoints_changed[index(provider)], cx);
            }
        }
        self.sync_controls(&changed, window, cx); cx.notify();
    }
    fn sync_controls(&mut self, changed: &BTreeSet<ConfigField>, window: &mut Window, cx: &mut Context<Self>) {
        let value = &self.draft.value.settings;
        if changed.contains(&ConfigField::OcrMode) { self.extra_drag = None; self.mode.update(cx, |state, cx| state.set_selected_value(&value.ocr_mode, window, cx)); }
        if changed.contains(&ConfigField::OcrLocalModel) { self.local_model.update(cx, |state, cx| state.set_selected_value(&value.ocr_local_model, window, cx)); }
        if changed.contains(&ConfigField::OcrRemoteRequestFormat) { self.remote_format.update(cx, |state, cx| state.set_selected_value(&value.ocr_remote_request_format, window, cx)); }
        for (field, input, text) in [(ConfigField::OcrRemoteUrl, &self.remote_url, &value.ocr_remote_url),
            (ConfigField::OcrRemoteImageFieldName, &self.image_field, &value.ocr_remote_image_field_name), (ConfigField::OcrRemoteResultPath, &self.result_path, &value.ocr_remote_result_path)] {
            if changed.contains(&field) { set_text(input, text, window, cx); }
        }
        if changed.contains(&ConfigField::OcrRemoteExtraParams) && self.extra.read(cx).value().as_ref() != value.ocr_remote_extra_params {
            self.extra.update(cx, |state, cx| state.set_value(value.ocr_remote_extra_params.clone(), window, cx));
        }
        for provider in [AiProvider::Openai, AiProvider::Claude] {
            if changed.contains(&profile_field(provider)) {
                let profile = self.profile(provider); let fields = &self.profiles[index(provider)];
                set_text(&fields.url, &profile.base_url, window, cx);
                fields.model.update(cx, |state, cx| state.set_value(&profile.model, window, cx));
                if provider == AiProvider::Openai { self.api.update(cx, |state, cx| state.set_selected_value(&profile.api, window, cx)); }
            }
        }
    }
    fn acknowledge(&mut self, feedback: &Feedback, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ack) = self.draft.acknowledge(feedback) else { return; };
        for id in ack.cleared_credentials { for provider in [AiProvider::Openai, AiProvider::Claude] { if credential(provider) == id { set_text(&self.profiles[index(provider)].key, "", window, cx); } } }
        let save = self.saving == Some(ack.mutation_id);
        if save {
            self.saving = None;
            match &ack.result { Ok(_) => self.message = Some("OCR 配置已保存".into()), Err(error) => self.error = Some(error.to_string().into()) }
        }
        self.synchronize(window, cx);
        if let Some(intent) = self.ai_commits.remove(&ack.mutation_id) { self.profile_committed(intent, ack.result, window, cx); }
        self.pump_ai(window, cx); cx.notify();
    }
    fn validate_extra(&self) -> Result<()> {
        let value = self.draft.value.settings.ocr_remote_extra_params.trim();
        if value.is_empty() { return Ok(()); }
        match serde_json::from_str::<serde_json::Value>(value) {
            Ok(serde_json::Value::Object(_)) => Ok(()), Ok(_) => Err(Error::Invalid("额外参数必须是 JSON 对象".into())),
            Err(_) => Err(Error::Invalid("额外参数 JSON 格式不正确".into())),
        }
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving.is_some() || self.local_busy() || !self.draft.has_unsaved() { return; }
        let result = (|| {
            self.validate_extra()?;
            let value = &self.draft.value.settings;
            let normalizations = [ConfigChange::OcrRemoteExtraParams(if value.ocr_remote_extra_params.trim().is_empty() { "{}".to_owned() } else { value.ocr_remote_extra_params.trim().to_owned() }),
                ConfigChange::OcrRemoteImageFieldName(if value.ocr_remote_image_field_name.is_empty() { "image".to_owned() } else { value.ocr_remote_image_field_name.clone() }),
                ConfigChange::OcrRemoteResultPath(if value.ocr_remote_result_path.is_empty() { "data.result".to_owned() } else { value.ocr_remote_result_path.clone() })];
            let mut changed = BTreeSet::new();
            for change in normalizations { let field = change.field(); if self.draft.set(change)? { changed.insert(field); } }
            self.sync_controls(&changed, window, cx);
            if self.draft.value.settings.enable_local_ocr {
                match self.draft.value.settings.ocr_mode {
                    OcrMode::Remote if self.draft.value.settings.ocr_remote_url.is_empty() => return Err(Error::Invalid("请输入远程 OCR 调用地址".into())),
                    OcrMode::Openai => self.validate_profile(AiProvider::Openai, true)?,
                    OcrMode::Claude => self.validate_profile(AiProvider::Claude, true)?, _ => {},
                }
            }
            self.draft.submit(FIELDS, [], cx)
        })();
        self.message = None;
        match result { Ok(id) => { self.extra_drag = None; self.saving = Some(id); self.error = None; }, Err(error) => self.error = Some(error.to_string().into()) }
        cx.notify();
    }
    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving.is_some() || self.local_busy() || self.draft.pending() { return; }
        let state = self.context.configuration.read(cx).state.clone();
        match self.draft.reset(&state) {
            Ok(()) => {
                for provider in [AiProvider::Openai, AiProvider::Claude] { self.invalidate_profile(provider, true, cx); set_text(&self.profiles[index(provider)].key, "", window, cx); }
                self.sync_controls(&FIELDS.into_iter().collect(), window, cx); self.message = None; self.error = None; self.confirm_delete = false;
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
        cx.notify();
    }
}
fn index(provider: AiProvider) -> usize { match provider { AiProvider::Openai => 0, AiProvider::Claude => 1 } }
fn credential(provider: AiProvider) -> CredentialId { match provider { AiProvider::Openai => CredentialId::OpenaiApiKey, AiProvider::Claude => CredentialId::ClaudeApiKey } }
fn profile_field(provider: AiProvider) -> ConfigField { match provider { AiProvider::Openai => ConfigField::OpenaiConfig, AiProvider::Claude => ConfigField::ClaudeConfig } }
fn input(value: &str, placeholder: &'static str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()).placeholder(placeholder))
}
fn set_text(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value { input.update(cx, |state, cx| state.set_value(value.to_owned(), window, cx)); }
}
