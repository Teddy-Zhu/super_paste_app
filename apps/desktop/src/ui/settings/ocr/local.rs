use super::*;
use catalog::ModelState;
use tokio_util::task::AbortOnDropHandle;

#[derive(Clone, Copy)]
pub(super) enum ModelAction { Download, Cancel, Redownload, Delete }
impl OcrPane {
    pub(super) fn local_busy(&self) -> bool {
        self.local_action.is_some() || self.statuses.values().any(|status| status.state == ModelState::Downloading)
    }
    pub(super) fn set_status_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.status_visible == visible { return; }
        self.status_visible = visible;
        if visible {
            if self.status_again { self.refresh_models(window, cx); }
        } else {
            self.status_again = true;
            self.status_epoch = self.status_epoch.wrapping_add(1);
            self.status_cancel = None;
            self.status_updates.clear();
            // Keep the read slot while the manager drains an accepted file I/O.
        }
    }
    pub(super) fn refresh_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status_again = true;
        if !self.status_visible || self.status_task.is_some() || self.context.services.is_shutting_down() { return; }
        self.status_again = false;
        self.status_updates.clear();
        let epoch = self.status_epoch; let service = self.context.services.ocr.clone();
        let cancel = CancellationToken::new();
        self.status_cancel = Some(cancel.clone().drop_guard());
        let request = AbortOnDropHandle::new(self.context.services.runtime.spawn(async move { service.list_models(&cancel).await }));
        self.status_task = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = request.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                this.status_cancel = None; this.status_task = None;
                if !this.status_visible || epoch != this.status_epoch {
                    this.status_updates.clear();
                    this.status_again = true;
                    if this.status_visible { this.refresh_models(window, cx); }
                    return;
                }
                match result {
                    Ok(statuses) => {
                        this.status_error = None;
                        // Preserve events received during this read; progress
                        // must not restart a filesystem scan of every model.
                        let updated = &this.status_updates;
                        this.statuses.retain(|id, _| updated.contains(id));
                        for status in statuses {
                            if !updated.contains(&status.id) { this.statuses.insert(status.id.clone(), Arc::new(status)); }
                        }
                    }
                    Err(error) => this.status_error = Some(format!("读取模型状态失败：{error}").into()),
                }
                this.status_updates.clear();
                if this.status_again { this.refresh_models(window, cx); }
                cx.notify();
            });
        }));
    }
    pub(super) fn model_action(&mut self, action: ModelAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.context.services.is_shutting_down() { return; }
        let cancel = matches!(action, ModelAction::Cancel);
        if if cancel { self.cancelling } else { self.saving.is_some() || self.local_busy() } { return; }
        if matches!(action, ModelAction::Delete) && !self.confirm_delete { return; }
        let id = self.draft.value.settings.ocr_local_model.clone(); let mutation = Uuid::new_v4();
        if let Err(error) = catalog::get_model(&id) { self.local_error = Some(error.into()); cx.notify(); return; }
        if cancel { self.cancelling = true; } else { self.local_action = Some(mutation); }
        self.local_error = None; self.local_message = None; cx.notify();
        let services = self.context.services.clone();
        let service = services.ocr.clone(); let effects = services.effects.clone(); let selected = id.clone(); let configuration = services.configuration.clone();
        let request = services.runtime.spawn(async move {
            match action {
                ModelAction::Download => service.download_model(&id).await,
                ModelAction::Cancel => service.cancel_download(&id).await,
                ModelAction::Redownload => service.redownload_model(id).await,
                ModelAction::Delete => service.remove_model(id, effects).await,
            }?;
            Ok(configuration.snapshot())
        });
        let owner = window.window_handle();
        cx.spawn_in(window, async move |weak, cx| {
            let result = request.await.map_err(Error::from).and_then(|result| result); let failure = result.as_ref().err().cloned();
            let handled = weak.update_in(cx, |this, window, cx| {
                let active = this.context.is_active(cx.entity_id(), cx);
                if cancel { this.cancelling = false; } else if this.local_action == Some(mutation) { this.local_action = None; }
                match result {
                    Ok(state) => {
                        match action {
                            ModelAction::Delete => {
                                this.confirm_delete = false; this.local_message = Some("模型已删除".into());
                                if this.draft.value.settings.ocr_mode == OcrMode::Local && this.draft.value.settings.ocr_local_model == selected {
                                    if !state.snapshot.value.settings.enable_local_ocr {
                                        if let Err(error) = this.draft.reset_fields([ConfigField::EnableLocalOcr], &state) { this.local_error = Some(error.to_string().into()); }
                                    }
                                }
                            }
                            ModelAction::Redownload => this.local_message = Some("模型已重新下载".into()), _ => {},
                        }
                        this.refresh_models(window, cx);
                    }
                    Err(error) => this.local_error = Some(format!("模型操作失败：{error}").into()),
                }
                cx.notify();
                active
            }).unwrap_or(false);
            if !handled && !services.is_shutting_down() {
                if let Some(error) = failure { if !matches!(error, Error::Cancelled) { let _ = owner.update(cx, |_, window, cx| crate::ui::dialogs::message(format!("模型操作失败：{error}"), window, cx)); } }
            }
        }).detach();
    }
}
