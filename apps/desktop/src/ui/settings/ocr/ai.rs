use super::*;
use super_paste_core::configuration::ConfigurationState;
use super_paste_format::{AiConnectionTest, AiModelOption};
use tokio_util::sync::{CancellationToken, DropGuard};

#[derive(Default)]
pub(super) struct Jobs {
    profile_generation: u64, test_generation: u64, fetching: bool, testing: bool,
    model_cancel: Option<DropGuard>, test_cancel: Option<DropGuard>,
    pub model_message: Option<SharedString>, pub model_error: Option<SharedString>, pub test_message: Option<SharedString>, pub test_error: Option<SharedString>,
}
impl Jobs {
    pub fn fetching(&self) -> bool { self.fetching }
    pub fn testing(&self) -> bool { self.testing }
}
#[derive(Clone, Copy)]
pub(super) struct Intent { provider: AiProvider, test: bool, generation: u64 }
enum Reply { Models(Arc<Vec<AiModelOption>>), Test(Arc<AiConnectionTest>) }
impl OcrPane {
    pub(super) fn invalidate_profile(&mut self, provider: AiProvider, list: bool, cx: &mut Context<Self>) {
        let jobs = &mut self.ai_jobs[index(provider)];
        jobs.test_generation = jobs.test_generation.wrapping_add(1);
        jobs.testing = false; jobs.test_cancel = None; jobs.test_message = None; jobs.test_error = None;
        if list {
            jobs.profile_generation = jobs.profile_generation.wrapping_add(1);
            jobs.fetching = false; jobs.model_cancel = None; jobs.model_message = None; jobs.model_error = None;
            self.profiles[index(provider)].model.update(cx, |picker, cx| picker.set_options(Arc::new(Vec::new()), false, cx));
        }
    }
    pub(super) fn validate_profile(&self, provider: AiProvider, model: bool) -> Result<()> {
        let profile = self.profile(provider); let id = credential(provider);
        if profile.base_url.trim().is_empty() { return Err(Error::Invalid("请输入 Base URL".into())); }
        if !self.draft.secret_is_set(id) && !self.draft.has_saved_secret(id) { return Err(Error::Invalid("请输入 API Key".into())); }
        if model && profile.model.trim().is_empty() { return Err(Error::Invalid("请选择或输入模型名称、模型 ID".into())); }
        Ok(())
    }
    fn intent_current(&self, intent: Intent) -> bool {
        let jobs = &self.ai_jobs[index(intent.provider)];
        self.selected_provider() == Some(intent.provider) && intent.generation == if intent.test { jobs.test_generation } else { jobs.profile_generation }
    }
    pub(super) fn start_ai(&mut self, provider: AiProvider, test: bool, window: &mut Window, cx: &mut Context<Self>) {
        let jobs = &self.ai_jobs[index(provider)];
        if self.saving.is_some() || (if test { jobs.testing } else { jobs.fetching }) { return; }
        if let Err(error) = self.validate_profile(provider, test) { self.ai_failure(provider, test, error); cx.notify(); return; }
        let jobs = &mut self.ai_jobs[index(provider)];
        let generation = if test { jobs.testing = true; jobs.test_message = None; jobs.test_error = None; jobs.test_generation }
            else { jobs.fetching = true; jobs.model_message = None; jobs.model_error = None; jobs.profile_generation };
        self.ai_queue.push_back(Intent { provider, test, generation });
        self.pump_ai(window, cx); cx.notify();
    }
    pub(super) fn pump_ai(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let mut index_in_queue = 0;
        while index_in_queue < self.ai_queue.len() {
            let intent = self.ai_queue[index_in_queue];
            if !self.intent_current(intent) {
                self.ai_queue.remove(index_in_queue); self.finish_ai(intent); continue;
            }
            if self.draft.pending_field(profile_field(intent.provider)) || self.draft.pending_field(credential(intent.provider).field()) { index_in_queue += 1; continue; }
            self.ai_queue.remove(index_in_queue);
            match self.draft.submit([profile_field(intent.provider), credential(intent.provider).field()], [], cx) {
                Ok(id) => { self.ai_commits.insert(id, intent); }
                Err(error) => { self.finish_ai(intent); self.ai_failure(intent.provider, intent.test, error); }
            }
        }
    }
    pub(super) fn profile_committed(&mut self, intent: Intent, result: Result<ConfigurationState>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.intent_current(intent) { self.finish_ai(intent); return; }
        let state = match result {
            Ok(state) => state,
            Err(error) => { self.finish_ai(intent); self.ai_failure(intent.provider, intent.test, error); return; }
        };
        let token = CancellationToken::new();
        let jobs = &mut self.ai_jobs[index(intent.provider)];
        if intent.test { jobs.test_cancel = Some(token.clone().drop_guard()); } else { jobs.model_cancel = Some(token.clone().drop_guard()); }
        let service = self.context.services.ocr.clone();
        let request = self.context.services.runtime.spawn(async move {
            if intent.test { service.test_ai_connection(intent.provider, state, token).await.map(Reply::Test) }
            else { service.list_ai_models(intent.provider, state, token).await.map(Reply::Models) }
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = request.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                let current = this.intent_current(intent);
                this.finish_ai(intent);
                if current {
                    match result {
                        Ok(Reply::Models(models)) => {
                            if models.is_empty() { this.ai_jobs[index(intent.provider)].model_message = Some("接口未返回可选模型，可手动填写模型 ID".into()); }
                            let reveal = this.saving.is_none();
                            this.profiles[index(intent.provider)].model.update(cx, |picker, cx| picker.set_options(models, reveal, cx));
                        }
                        Ok(Reply::Test(result)) => this.ai_jobs[index(intent.provider)].test_message = Some(format!("连接成功 · {} · {} ms", result.model, result.latency_ms).into()),
                        Err(error) => this.ai_failure(intent.provider, intent.test, error),
                    }
                }
                this.pump_ai(window, cx); cx.notify();
            });
        }).detach();
    }
    fn finish_ai(&mut self, intent: Intent) {
        let jobs = &mut self.ai_jobs[index(intent.provider)];
        // Invalidating a profile allows its replacement to start immediately.
        // Only the owning generation may release that replacement's busy state or guard.
        if intent.generation != if intent.test { jobs.test_generation } else { jobs.profile_generation } { return; }
        if intent.test { jobs.testing = false; jobs.test_cancel = None; } else { jobs.fetching = false; jobs.model_cancel = None; }
    }
    fn ai_failure(&mut self, provider: AiProvider, test: bool, error: Error) {
        let text = format!("{}：{error}", if test { "连接测试失败" } else { "拉取模型失败" }).into();
        if test { self.ai_jobs[index(provider)].test_error = Some(text); } else { self.ai_jobs[index(provider)].model_error = Some(text); }
    }
    pub(super) fn clear_ai_key(&mut self, provider: AiProvider, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving.is_some() { return; }
        match self.draft.clear_secret(credential(provider)) {
            Ok(changed) => {
                set_text(&self.profiles[index(provider)].key, "", window, cx);
                if changed { self.invalidate_profile(provider, true, cx); self.edited(window, cx); }
            }
            Err(error) => { self.error = Some(error.to_string().into()); cx.notify(); }
        }
    }
}
