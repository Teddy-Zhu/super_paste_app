mod copy;
mod guide;
mod render;
mod reference;

use std::{sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use super_paste_core::{Error, clipboard::ClipboardSnapshot, credentials::Secret};
use super_paste_format::{ConfigChange, ConfigField, CredentialId, McpStatus, TextContent};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use super::PaneContext;
use crate::ui::{configuration::Feedback, draft::Draft, forms::{IntegerChanged, IntegerInput, IntegerSpec}, palette::Palette};
use guide::Client;

const FIELDS: [ConfigField; 4] = [ConfigField::McpEnabled, ConfigField::McpPort, ConfigField::McpAuthEnabled, ConfigField::McpApiKey];
#[derive(Clone, Copy)]
enum SecretAction { ShowKey, RevealGuide { client: Client, epoch: u64 }, CopyKey, CopyConfig(Client) }

pub struct McpPane {
    context: PaneContext, draft: Draft, port: Entity<IntegerInput>, key: Entity<InputState>, key_masked: bool,
    saved_versions: [u64; 4], endpoint: Entity<InputState>,
    saved_key: Option<Entity<InputState>>, show_key: bool, status: Arc<McpStatus>,
    pending: Option<Uuid>, operation: Option<Uuid>, message: Option<(bool, SharedString)>, message_timer: Option<Task<()>>,
    client: Client, client_focus: [FocusHandle; 4], guide_open: bool, guide_revealed: bool, guide_epoch: u64, guide_code: Option<Entity<TextareaState>>,
    reference_open: bool, tools_open: u8, push_open: bool, prompt_open: bool, push_code: Option<Entity<TextareaState>>,
    subscriptions: Vec<Subscription>, _status_task: Task<()>,
}
impl McpPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = Draft::new(context.configuration.clone(), FIELDS, cx);
        let p = Palette::for_window(draft.value.settings.color_mode, window);
        let port = cx.new(|cx| IntegerInput::new(copy::PORT, draft.value.mcp.port as i64, IntegerSpec { min: 1024, max: 65535, step: 1, empty: None, clamp: false }, p, window, cx));
        let key = cx.new(|cx| InputState::new(window, cx).max_utf16_length(256).masked(true).placeholder("输入 API Key"));
        let saved_versions = FIELDS.map(|field| context.configuration.read(cx).state.fields.get(&field).copied().unwrap_or(0));
        let endpoint = cx.new(|cx| InputState::new(window, cx).default_value(super_paste_format::mcp_endpoint(draft.value.mcp.port)));
        let mut updates = context.services.mcp.subscribe();
        let status = updates.borrow_and_update().clone();
        let task = cx.spawn(async move |weak, cx| {
            while updates.changed().await.is_ok() {
                let status = updates.borrow_and_update().clone();
                if weak.update(cx, |this, cx| { this.status = status; cx.notify(); }).is_err() { break; }
            }
        });
        let configuration = context.configuration.clone();
        let subscriptions = vec![
            cx.observe_in(&configuration, window, |this, _, window, cx| this.synchronize(window, cx)),
            cx.subscribe_in(&configuration, window, |this, _, feedback: &Feedback, window, cx| this.acknowledge(feedback, window, cx)),
            cx.observe(&port, |_, _, cx| cx.notify()),
            cx.subscribe_in(&port, window, |this, _, value: &IntegerChanged, window, cx| this.change(ConfigChange::McpPort(value.0 as u16), window, cx)),
            cx.subscribe_in(&key, window, |this, key, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = key.read(cx).value();
                    match this.draft.set_secret_input(CredentialId::McpApiKey, value.as_ref()) { Ok(true) => this.edited(cx), Ok(false) => {}, Err(error) => this.feedback(false, error.to_string(), window, cx) }
                } else { cx.notify(); }
            }),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            cx.observe_window_visibility(window, |_, _, _, cx| cx.notify()),
        ];
        Self { context, draft, port, key, key_masked: true, saved_versions, endpoint, saved_key: None, show_key: false, status, pending: None, operation: None, message: None, message_timer: None,
            client: Client::Cursor, client_focus: std::array::from_fn(|_| cx.focus_handle()), guide_open: false, guide_revealed: false, guide_epoch: 0, guide_code: None,
            reference_open: false, tools_open: 0, push_open: false, prompt_open: false, push_code: None, subscriptions, _status_task: task }
    }
    fn busy(&self) -> bool { self.pending.is_some() || self.operation.is_some() }
    fn synchronize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.context.configuration.read(cx).state.clone();
        self.draft.synchronize(&state);
        let versions = FIELDS.map(|field| state.fields.get(&field).copied().unwrap_or(0));
        if self.saved_versions != versions { self.saved_versions = versions; self.conceal(window, cx); }
        let endpoint = super_paste_format::mcp_endpoint(state.snapshot.value.mcp.port);
        if self.endpoint.read(cx).value().as_ref() != endpoint { self.endpoint.update(cx, |state, cx| state.set_value(endpoint, window, cx)); }
        cx.notify();
    }
    fn change(&mut self, change: ConfigChange, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        match self.draft.set(change) { Ok(true) => self.edited(cx), Ok(false) => {}, Err(error) => self.feedback(false, error.to_string(), window, cx) }
    }
    fn edited(&mut self, cx: &mut Context<Self>) { self.message = None; self.message_timer = None; cx.notify(); }
    fn mask(&mut self, masked: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.key_masked != masked { self.key.update(cx, |state, cx| state.toggle_masked(window, cx)); self.key_masked = masked; }
    }
    fn conceal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.saved_key = None; self.show_key = false; self.guide_revealed = false; self.guide_epoch = self.guide_epoch.wrapping_add(1); self.mask(true, window, cx);
        if self.guide_open { self.set_guide(None, window, cx); } else { self.guide_code = None; }
    }
    fn valid(&self, cx: &App) -> bool {
        self.port.read(cx).valid_value(cx).is_some() && (!self.draft.secret_is_set(CredentialId::McpApiKey) || valid_key(self.key.read(cx).value().as_ref())) && (!self.draft.value.mcp.enabled || !self.draft.value.mcp.auth_enabled ||
            (self.draft.secret_is_keep(CredentialId::McpApiKey) && self.draft.has_saved_secret(CredentialId::McpApiKey)) || valid_key(self.key.read(cx).value().as_ref()))
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        if !self.valid(cx) { self.feedback(false, "请填写有效端口和 API Key", window, cx); return; }
        if let Some(port) = self.port.read(cx).valid_value(cx) { if let Err(error) = self.draft.set(ConfigChange::McpPort(port as u16)) { self.feedback(false, error.to_string(), window, cx); return; } }
        self.message = None;
        match self.draft.submit(FIELDS, [], cx) { Ok(id) => self.pending = Some(id), Err(error) => self.feedback(false, error.to_string(), window, cx) }
        cx.notify();
    }
    fn acknowledge(&mut self, feedback: &Feedback, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ack) = self.draft.acknowledge(feedback) else { return; };
        if ack.cleared_credentials.contains(&CredentialId::McpApiKey) { self.key.update(cx, |state, cx| state.set_value("", window, cx)); }
        if self.pending != Some(ack.mutation_id) { self.synchronize(window, cx); return; }
        self.pending = None; self.conceal(window, cx);
        let expected = match ack.result { Ok(state) => state, Err(error) => { self.feedback(false, error.to_string(), window, cx); return; } };
        // A configuration no-op deliberately skips effects. Saving unchanged
        // settings must still be able to retry a failed listener.
        let id = Uuid::new_v4(); self.operation = Some(id);
        let service = self.context.services.mcp.clone(); let runtime = self.context.services.runtime.clone();
        cx.spawn_in(window, async move |weak, cx| {
            let result = runtime.spawn(async move { service.retry(expected).await }).await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.operation != Some(id) { return; } this.operation = None;
                match result { Ok(status) => { this.status = status; this.feedback(true, copy::SAVED, window, cx); }, Err(error) => this.feedback(false, format!("配置已保存，应用服务配置失败：{error}"), window, cx) }
            });
        }).detach(); cx.notify();
    }
    fn clear_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        match self.draft.clear_secret(CredentialId::McpApiKey) {
            Ok(_) => { self.key.update(cx, |state, cx| state.set_value("", window, cx)); self.conceal(window, cx); self.edited(cx); },
            Err(error) => self.feedback(false, error.to_string(), window, cx),
        }
    }
    fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        let id = Uuid::new_v4(); self.operation = Some(id);
        let service = self.context.services.mcp.clone(); let runtime = self.context.services.runtime.clone();
        cx.spawn_in(window, async move |weak, cx| {
            let result = runtime.spawn(async move { service.generate_key().await }).await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.operation != Some(id) { return; } this.operation = None;
                match result {
                    Ok(key) => match this.draft.set_secret_input(CredentialId::McpApiKey, key.expose()) {
                        Ok(_) => { this.saved_key = None; this.key.update(cx, |state, cx| state.set_value(key.expose().to_owned(), window, cx)); this.mask(!this.show_key, window, cx); this.edited(cx); },
                        Err(error) => this.feedback(false, error.to_string(), window, cx),
                    },
                    Err(error) => this.feedback(false, error.to_string(), window, cx),
                }
            });
        }).detach(); cx.notify();
    }
    fn toggle_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        if self.show_key { self.show_key = false; self.saved_key = None; self.mask(true, window, cx); cx.notify(); }
        else if self.draft.secret_is_keep(CredentialId::McpApiKey) && self.draft.has_saved_secret(CredentialId::McpApiKey) { self.secret(SecretAction::ShowKey, window, cx); }
        else { self.show_key = true; self.mask(false, window, cx); cx.notify(); }
    }
    fn secret(&mut self, action: SecretAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        if matches!(action, SecretAction::CopyKey) && self.draft.secret_is_set(CredentialId::McpApiKey) {
            self.copy_text(self.key.read(cx).value().to_string(), window, cx); return;
        }
        if matches!(action, SecretAction::CopyKey) && !self.draft.has_saved_secret(CredentialId::McpApiKey) { self.feedback(false, "没有可复制的密钥", window, cx); return; }
        let expected = self.context.configuration.read(cx).state.clone();
        if let SecretAction::CopyConfig(client) = action { if !client.includes_key(&expected.snapshot.value.mcp) {
            match guide::configuration(client, &expected.snapshot.value.mcp, None) { Ok(text) => self.copy_text(text, window, cx), Err(error) => self.feedback(false, error.to_string(), window, cx) } return;
        } }
        let id = Uuid::new_v4(); self.operation = Some(id);
        let service = self.context.services.mcp.clone(); let runtime = self.context.services.runtime.clone(); let guarded = expected.clone();
        cx.spawn_in(window, async move |weak, cx| {
            let result = runtime.spawn(async move { service.read_key(guarded).await }).await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.operation != Some(id) { return; } this.operation = None;
                let state = &this.context.configuration.read(cx).state;
                if FIELDS.iter().any(|field| state.fields.get(field) != expected.fields.get(field)) { this.feedback(false, "MCP 配置已变更，请重试", window, cx); return; }
                match result { Ok(key) => this.secret_ready(action, key, window, cx), Err(error) => this.feedback(false, error.to_string(), window, cx) }
            });
        }).detach(); cx.notify();
    }
    fn secret_ready(&mut self, action: SecretAction, key: Arc<Secret>, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        match action {
            SecretAction::ShowKey => { self.saved_key = Some(cx.new(|cx| InputState::new(window, cx).default_value(key.expose().to_owned()))); self.show_key = true; cx.notify(); },
            SecretAction::RevealGuide { client, epoch } => { if self.guide_open && self.client == client && self.guide_epoch == epoch { self.guide_revealed = true; self.set_guide(Some(key.expose()), window, cx); } },
            SecretAction::CopyKey => self.copy_text(key.expose().to_owned(), window, cx),
            SecretAction::CopyConfig(client) => {
                let settings = &self.context.configuration.read(cx).state.snapshot.value.mcp;
                match guide::configuration(client, settings, Some(key.expose())) { Ok(text) => self.copy_text(text, window, cx), Err(error) => self.feedback(false, error.to_string(), window, cx) }
            },
        }
    }
    fn copy_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() { return; }
        let id = Uuid::new_v4(); self.operation = Some(id);
        let clipboard = self.context.services.clipboard.clone(); let runtime = self.context.services.runtime.clone();
        cx.spawn_in(window, async move |weak, cx| {
            let result = runtime.spawn(async move { clipboard.write(ClipboardSnapshot { content: TextContent { text: Some(text.into()), ..Default::default() }, image: None }, &CancellationToken::new()).await }).await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.operation != Some(id) { return; } this.operation = None;
                match result { Ok(()) => this.feedback(true, copy::COPIED, window, cx), Err(error) => this.feedback(false, format!("{}：{error}", copy::COPY_FAILED), window, cx) }
            });
        }).detach(); cx.notify();
    }
    fn set_guide(&mut self, key: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let settings = &self.context.configuration.read(cx).state.snapshot.value.mcp;
        match guide::configuration(self.client, settings, key) { Ok(text) => self.guide_code = Some(code_state(text, window, cx)), Err(error) => self.feedback(false, error.to_string(), window, cx) }
        cx.notify();
    }
    fn select_client(&mut self, client: Client, window: &mut Window, cx: &mut Context<Self>) {
        self.client = client; self.guide_revealed = false; self.guide_epoch = self.guide_epoch.wrapping_add(1);
        if self.guide_open { self.set_guide(None, window, cx); }
        self.client_focus[client.index()].focus(window, cx); cx.notify();
    }
    fn feedback(&mut self, success: bool, text: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        self.message = Some((success, text.into()));
        let timer = cx.background_executor().timer(Duration::from_secs(3));
        self.message_timer = Some(cx.spawn_in(window, async move |weak, cx| { timer.await; let _ = weak.update_in(cx, |this, _, cx| { this.message = None; this.message_timer = None; cx.notify(); }); })); cx.notify();
    }
}
fn valid_key(key: &str) -> bool { (8..=256).contains(&key.len()) && key.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) }
fn code_state(text: String, window: &mut Window, cx: &mut App) -> Entity<TextareaState> {
    let rows = text.lines().count().clamp(2, 40);
    cx.new(|cx| TextareaState::new(window, cx).rows(rows).soft_wrap(false).default_value(text))
}
