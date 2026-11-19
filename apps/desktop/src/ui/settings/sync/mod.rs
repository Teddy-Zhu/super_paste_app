mod render;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::{input::{InputState, InputEvent}, select::SelectEvent};
use super_paste_core::{Error, Result, configuration::ConfigurationState, events::AppEvent};
use super_paste_format::{ConfigChange, ConfigField, CredentialId, SyncDirection, SyncOutcome, SyncState, SyncStatus, WebDavConfig};
use tokio_util::sync::{CancellationToken, DropGuard};
use uuid::Uuid;
use super::PaneContext;
use crate::ui::{configuration::Feedback, dialogs, draft::Draft, forms::{self, Choice, ChoiceState, IntegerInput, IntegerSpec, IntegerChanged}, palette::Palette};

const FIELDS: [ConfigField; 6] = [ConfigField::SyncEnabled, ConfigField::SyncProvider, ConfigField::SyncInterval, ConfigField::SyncDirection, ConfigField::WebDav, ConfigField::WebDavPassword];
const INTERVALS: [(&str, u64); 9] = [("5分钟", 300_000), ("10分钟", 600_000), ("15分钟", 900_000), ("30分钟", 1_800_000), ("1小时", 3_600_000), ("2小时", 7_200_000), ("6小时", 21_600_000), ("12小时", 43_200_000), ("24小时", 86_400_000)];
const DIRECTIONS: [(SyncDirection, &str, &str); 3] = [(SyncDirection::Bidirectional, "双向同步", "本地和远程文件双向同步"), (SyncDirection::Upload, "仅上传", "仅将本地文件上传到远程"), (SyncDirection::Download, "仅下载", "仅从远程下载文件到本地")];
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action { Save, Test, Manual }
#[derive(Clone)]
enum NetworkReply { ConnectionTested, Synchronized(Arc<SyncOutcome>) }
struct Active { id: Uuid, kind: Action, generation: u64, cancel: Option<DropGuard> }
#[derive(Clone, Copy)]
enum TextField { Url, Username, Password, Path }
pub struct SyncPane {
    context: PaneContext, draft: Draft, provider: Entity<ChoiceState<String>>, interval: Entity<ChoiceState<u64>>,
    url: Entity<InputState>, username: Entity<InputState>, password: Entity<InputState>, path: Entity<InputState>, timeout: Entity<IntegerInput>,
    radio_focus: [FocusHandle; 3], radio_hover: [bool; 3], generation: u64, pending: Option<Uuid>, action: Option<Active>,
    save_message: Option<(bool, SharedString)>, test_result: Option<(bool, SharedString)>, status: Arc<SyncStatus>, phase: SyncState, phase_message: Option<SharedString>,
    save_timer: Option<Task<()>>, phase_timer: Option<Task<()>>, _events: Task<()>, subscriptions: Vec<Subscription>,
}
impl SyncPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = Draft::new(context.configuration.clone(), FIELDS, cx); let sync = &draft.value.sync;
        let p = Palette::for_window(context.configuration.read(cx).projected.settings.color_mode, window);
        let provider = forms::choices(vec![Choice::new("请选择同步提供者", String::new()), Choice::new("WebDAV", "webdav".to_owned())], &sync.provider, window, cx);
        let interval = forms::choices(INTERVALS.into_iter().map(|(label, value)| Choice::new(label, value)).collect(), &sync.interval, window, cx);
        let url = input(&sync.webdav.server_url, "https://example.com/webdav", window, cx);
        let username = input(&sync.webdav.username, "输入用户名", window, cx);
        let password = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("输入密码"));
        let path = input(&sync.webdav.remote_path, "/super-paste", window, cx);
        let timeout = cx.new(|cx| IntegerInput::new("超时时间", sync.webdav.timeout.min(i64::MAX as u64) as i64, IntegerSpec { min: 5000, max: 120000, step: 1000, empty: Some(30000), clamp: true }, p, window, cx));
        let status = context.services.sync.snapshot(); let phase = if status.state == SyncState::Syncing { SyncState::Syncing } else { SyncState::Idle };
        let mut events = context.services.events.subscribe(); let services = context.services.clone();
        let event_task = cx.spawn_in(window, async move |weak, cx| {
            loop {
                match events.recv().await {
                    Ok(AppEvent::SyncStatusChanged(status)) => { if weak.update_in(cx, |this, window, cx| this.status_changed(status, window, cx)).is_err() { break; } },
                    Ok(AppEvent::SyncFinished { manual, result }) => { if weak.update_in(cx, |this, window, cx| this.finished_phase(manual, &result, window, cx)).is_err() { break; } },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let status = services.sync.snapshot();
                        if weak.update_in(cx, |this, window, cx| { this.status_changed(status.clone(), window, cx); if status.state != SyncState::Syncing {
                            this.phase = status.state; this.phase_message = status.error.clone().map(Into::into); this.expire_phase(window, cx);
                        } }).is_err() { break; }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {},
                }
            }
        });
        let mut pane = Self { context, draft, provider, interval, url, username, password, path, timeout, radio_focus: std::array::from_fn(|_| cx.focus_handle()), radio_hover: [false; 3], generation: 0,
            pending: None, action: None, save_message: None, test_result: None, status, phase, phase_message: None, save_timer: None, phase_timer: None, _events: event_task, subscriptions: Vec::new() };
        pane.subscribe(window, cx); pane
    }
    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions.push(cx.observe_in(&self.context.configuration, window, |this, _, window, cx| this.synchronize(window, cx)));
        self.subscriptions.push(cx.subscribe_in(&self.context.configuration, window, |this, _, feedback: &Feedback, window, cx| this.acknowledge(feedback, window, cx)));
        self.subscriptions.push(cx.observe_window_appearance(window, |_, _, cx| cx.notify()));
        self.subscriptions.push(cx.subscribe_in(&self.provider, window, |this, _, event: &SelectEvent<Vec<Choice<String>>>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event { this.change(ConfigChange::SyncProvider(value.clone()), window, cx); }
        }));
        self.subscriptions.push(cx.subscribe_in(&self.interval, window, |this, _, event: &SelectEvent<Vec<Choice<u64>>>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event { this.change(ConfigChange::SyncInterval(*value), window, cx); }
        }));
        self.subscriptions.push(cx.subscribe_in(&self.timeout, window, |this, _, value: &IntegerChanged, window, cx| this.edit_webdav(|config| config.timeout = value.0 as u64, window, cx)));
        for (input, field) in [(self.url.clone(), TextField::Url), (self.username.clone(), TextField::Username), (self.password.clone(), TextField::Password), (self.path.clone(), TextField::Path)] {
            self.subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value();
                    match field {
                        TextField::Password => match this.draft.set_secret_input(CredentialId::WebDavPassword, value.as_ref()) {
                            Ok(true) => this.edited(window, cx), Ok(false) => {}, Err(error) => this.save_feedback(false, error.to_string(), window, cx),
                        },
                        _ => this.edit_webdav(|config| match field { TextField::Url => config.server_url = value.to_string(), TextField::Username => config.username = value.to_string(), TextField::Path => config.remote_path = value.to_string(), TextField::Password => {} }, window, cx),
                    }
                } else { cx.notify(); }
            }));
        }
    }
    fn change(&mut self, change: ConfigChange, window: &mut Window, cx: &mut Context<Self>) {
        match self.draft.set(change) { Ok(true) => self.edited(window, cx), Ok(false) => {}, Err(error) => self.save_feedback(false, error.to_string(), window, cx) }
    }
    fn edit_webdav(&mut self, update: impl FnOnce(&mut WebDavConfig), window: &mut Window, cx: &mut Context<Self>) {
        let mut config = self.draft.value.sync.webdav.clone(); update(&mut config); self.change(ConfigChange::WebDav(config), window, cx);
    }
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1); self.test_result = None;
        if let Some(action) = &mut self.action { action.cancel = None; }
    }
    fn edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate(); self.save_message = None; self.save_timer = None; self.synchronize(window, cx); cx.notify();
    }
    fn synchronize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.context.configuration.read(cx).state.clone(); let changed = self.draft.synchronize(&state);
        if !changed.is_empty() {
            // A download can itself change the shared schedule. Only replacing
            // this machine's endpoint/credential makes its own result stale.
            if changed.contains(&ConfigField::WebDav) || changed.contains(&ConfigField::WebDavPassword) { self.invalidate(); }
            else if changed.contains(&ConfigField::SyncEnabled) || changed.contains(&ConfigField::SyncProvider) { self.test_result = None; }
            self.controls(&changed, window, cx);
        } cx.notify();
    }
    fn controls(&mut self, changed: &BTreeSet<ConfigField>, window: &mut Window, cx: &mut Context<Self>) {
        let sync = &self.draft.value.sync;
        if changed.contains(&ConfigField::SyncProvider) { self.provider.update(cx, |state, cx| state.set_selected_value(&sync.provider, window, cx)); }
        if changed.contains(&ConfigField::SyncInterval) { self.interval.update(cx, |state, cx| state.set_selected_value(&sync.interval, window, cx)); }
        if changed.contains(&ConfigField::WebDav) {
            for (input, text) in [(&self.url, &sync.webdav.server_url), (&self.username, &sync.webdav.username), (&self.path, &sync.webdav.remote_path)] { set_text(input, text, window, cx); }
        }
    }
    fn valid_webdav(&self) -> bool {
        let config = &self.draft.value.sync;
        config.provider == "webdav" && !config.webdav.server_url.trim().is_empty() && !config.webdav.username.trim().is_empty() && config.validate().is_ok()
            && (self.draft.has_saved_secret(CredentialId::WebDavPassword) || self.draft.secret_is_set(CredentialId::WebDavPassword))
    }
    fn start(&mut self, kind: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.action.is_some() { self.preflight_error(kind, Error::ResourceLimit("同步操作正在进行".into()), window, cx); return; }
        if (kind != Action::Save || self.draft.value.sync.enabled) && !self.valid_webdav() {
            self.preflight_error(kind, Error::Invalid("配置不完整，请填写完整的服务器信息并确保 URL 格式正确".into()), window, cx); return;
        }
        self.test_result = None;
        self.action = Some(Active { id: Uuid::new_v4(), kind, generation: self.generation, cancel: None });
        match self.draft.submit(FIELDS, [], cx) {
            Ok(id) => self.pending = Some(id),
            Err(error) => { self.action = None; self.preflight_error(kind, error, window, cx); }
        }
        cx.notify();
    }
    fn acknowledge(&mut self, feedback: &Feedback, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ack) = self.draft.acknowledge(feedback) else { return; };
        if ack.cleared_credentials.contains(&CredentialId::WebDavPassword) { set_text(&self.password, "", window, cx); }
        self.synchronize(window, cx);
        if self.pending != Some(ack.mutation_id) { return; } self.pending = None;
        let Some(action) = &self.action else { return; }; let kind = action.kind;
        let stale = action.generation != self.generation;
        match ack.result {
            Err(error) => { self.action = None; self.preflight_error(kind, error, window, cx); },
            Ok(_) if stale => { self.action = None; cx.notify(); },
            Ok(state) if kind == Action::Save && !state.snapshot.value.sync.enabled => { self.action = None; self.save_feedback(true, "配置保存成功", window, cx); },
            Ok(state) => self.network(state, window, cx),
        }
    }
    fn network(&mut self, expected: ConfigurationState, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.action.as_mut() else { return; };
        let token = CancellationToken::new(); action.cancel = Some(token.clone().drop_guard());
        let id = action.id; let generation = action.generation; let kind = action.kind;
        if kind == Action::Save { self.save_feedback(true, "正在测试连接...", window, cx); }
        let service = self.context.services.sync.clone(); let runtime = self.context.services.runtime.clone();
        cx.spawn_in(window, async move |weak, cx| {
            let result = runtime.spawn(async move {
                if kind == Action::Manual { service.synchronize(expected, token).await.map(NetworkReply::Synchronized) }
                else { service.test_connection(expected, token).await.map(|_| NetworkReply::ConnectionTested) }
            }).await.map_err(Error::from).and_then(|result| result);
            if weak.update_in(cx, |this, window, cx| {
                if this.action.as_ref().is_none_or(|action| action.id != id) { return; }
                this.action = None;
                if this.generation == generation && !this.context.services.is_shutting_down() { this.report(kind, result.clone(), window, cx); } cx.notify();
            }).is_err() && kind == Action::Manual && !matches!(&result, Err(Error::Cancelled)) {
                let message = match result { Ok(NetworkReply::Synchronized(outcome)) => format!("手动同步{}: {}", if outcome.conflict.is_some() { "失败" } else { "完成" }, outcome.message), Err(error) => format!("手动同步失败: {error}"), Ok(NetworkReply::ConnectionTested) => "手动同步未完成：操作结果类型错误".into() };
                let _ = cx.update(|window, cx| dialogs::message(message, window, cx));
            }
        }).detach();
    }
    fn report(&mut self, kind: Action, result: Result<NetworkReply>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(NetworkReply::Synchronized(outcome)) => dialogs::message(format!("手动同步{}: {}", if outcome.conflict.is_some() { "失败" } else { "完成" }, outcome.message), window, cx),
            Ok(NetworkReply::ConnectionTested) => {
                self.test_result = Some((true, "连接测试成功！服务器配置正确".into()));
                if kind == Action::Save { self.save_feedback(true, "配置保存成功", window, cx); }
            },
            Err(error) if kind == Action::Manual => dialogs::message(if matches!(error, Error::Cancelled) { "手动同步已取消，请重新同步".into() } else { format!("手动同步失败: {error}") }, window, cx),
            Err(error) => {
                let cancelled = matches!(error, Error::Cancelled);
                self.test_result = Some((false, if cancelled { "连接测试已取消".into() } else { format!("连接测试失败: {error}").into() }));
                if kind == Action::Save { self.save_feedback(cancelled, if cancelled { "配置已保存，连接测试已取消" } else { "配置已保存，但连接测试失败" }, window, cx); }
            },
        }
        cx.notify();
    }
    fn preflight_error(&mut self, kind: Action, error: Error, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            Action::Save => self.save_feedback(false, format!("保存失败: {error}"), window, cx),
            Action::Test => { self.test_result = Some((false, format!("连接测试失败: {error}").into())); cx.notify(); },
            Action::Manual => dialogs::message(format!("手动同步失败: {error}"), window, cx),
        }
    }
    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.context.configuration.read(cx).state.clone();
        match self.draft.reset(&state) {
            Ok(()) => { self.invalidate(); self.controls(&FIELDS.into_iter().collect(), window, cx); set_text(&self.password, "", window, cx); self.save_feedback(true, "配置已重置", window, cx); },
            Err(error) => self.save_feedback(false, error.to_string(), window, cx),
        }
    }
    fn clear_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.draft.clear_secret(CredentialId::WebDavPassword) {
            Ok(changed) => { set_text(&self.password, "", window, cx); if changed { self.edited(window, cx); } },
            Err(error) => self.save_feedback(false, error.to_string(), window, cx),
        }
    }
    fn save_feedback(&mut self, success: bool, message: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        self.save_message = Some((success, message.into()));
        let timer = cx.background_executor().timer(Duration::from_secs(3));
        self.save_timer = Some(cx.spawn_in(window, async move |weak, cx| { timer.await; let _ = weak.update_in(cx, |this, _, cx| { this.save_message = None; this.save_timer = None; cx.notify(); }); }));
        cx.notify();
    }
    fn status_changed(&mut self, status: Arc<SyncStatus>, _: &mut Window, cx: &mut Context<Self>) {
        if status.state == SyncState::Syncing { self.phase = SyncState::Syncing; self.phase_timer = None; self.phase_message = Some(if self.action.as_ref().is_some_and(|action| action.kind == Action::Manual) { "正在执行手动同步..." } else { "正在执行定时同步..." }.into()); }
        else if status.state == SyncState::Idle && self.phase == SyncState::Syncing { self.phase = SyncState::Idle; self.phase_message = None; self.phase_timer = None; }
        self.status = status; cx.notify();
    }
    fn finished_phase(&mut self, _: bool, result: &Result<Arc<SyncOutcome>>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(outcome) => { self.phase = if outcome.conflict.is_some() { SyncState::Error } else { SyncState::Success }; self.phase_message = Some(outcome.message.clone().into()); },
            Err(error) => { self.phase = SyncState::Error; self.phase_message = Some(error.to_string().into()); },
        }
        self.expire_phase(window, cx); cx.notify();
    }
    fn expire_phase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(Duration::from_secs(if self.phase == SyncState::Error { 5 } else { 3 }));
        self.phase_timer = Some(cx.spawn_in(window, async move |weak, cx| { timer.await; let _ = weak.update_in(cx, |this, _, cx| { this.phase = SyncState::Idle; this.phase_message = None; this.phase_timer = None; cx.notify(); }); }));
    }
}
fn input(value: &str, placeholder: &'static str, window: &mut Window, cx: &mut App) -> Entity<InputState> { cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()).placeholder(placeholder)) }
fn set_text(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) { if input.read(cx).value().as_ref() != value { input.update(cx, |state, cx| state.set_value(value.to_owned(), window, cx)); } }
