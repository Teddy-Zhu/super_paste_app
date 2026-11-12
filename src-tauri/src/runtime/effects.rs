use super::Runtime;
use crate::config::{
    ConfigChange, ConfigEffectFuture, ConfigEffectHooks, ConfigSnapshot, PreparedConfigEffect,
};
use std::sync::{Arc, OnceLock, Weak};

pub(super) struct EffectHooks {
    pub(super) runtime: Arc<OnceLock<Weak<Runtime>>>,
}
struct PreparedEffects {
    runtime: Arc<Runtime>,
    before: ConfigSnapshot,
    after: ConfigSnapshot,
    changes: Vec<ConfigChange>,
    applied: bool,
}

impl ConfigEffectHooks for EffectHooks {
    fn prepare<'a>(
        &'a self,
        before: &'a ConfigSnapshot,
        after: &'a ConfigSnapshot,
        changes: &'a [ConfigChange],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Box<dyn PreparedConfigEffect>, String>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let runtime = self
                .runtime
                .get()
                .and_then(Weak::upgrade)
                .ok_or("配置副作用服务尚未就绪")?;
            if runtime.is_quitting() {
                return Err("应用正在退出".into());
            }
            Ok(Box::new(PreparedEffects {
                runtime,
                before: before.clone(),
                after: after.clone(),
                changes: changes.to_vec(),
                applied: false,
            }) as Box<dyn PreparedConfigEffect>)
        })
    }
}

impl PreparedEffects {
    async fn apply_snapshot(&self, snapshot: &ConfigSnapshot) -> Result<(), String> {
        let changed = |field: &str| {
            self.changes
                .iter()
                .any(|change| change.store_id == "settings" && change.key == field)
        };
        if changed("globalShortcut") {
            self.runtime
                .shortcuts
                .apply(
                    snapshot.settings["globalShortcut"]
                        .as_str()
                        .ok_or("快捷键无效")?,
                )
                .await?;
        }
        if changed("autoStart") {
            self.runtime
                .apply_autostart(
                    snapshot.settings["autoStart"]
                        .as_bool()
                        .ok_or("自启配置无效")?,
                )
                .await?;
        }
        if changed("clipboardListening") {
            self.runtime.reconcile_listening(false).await?;
        }
        if changed("colorMode") {
            self.runtime.apply_theme(&snapshot.settings["colorMode"])?;
        }
        if changed("overlayPosition") {
            self.runtime.windows.position_changed().await?;
        }
        if self.changes.iter().any(|change| {
            change.store_id == "settings"
                && (change.key.starts_with("ocr") || change.key == "enableLocalOCR")
        }) {
            self.runtime.ocr.configuration_changed().await?;
        }
        if self.changes.iter().any(|change| change.store_id == "sync") {
            self.runtime.sync.configure().await?;
        }
        Ok(())
    }
}

impl PreparedConfigEffect for PreparedEffects {
    fn apply(&mut self) -> ConfigEffectFuture<'_> {
        Box::pin(async move {
            self.applied = true;
            self.apply_snapshot(&self.after).await
        })
    }
    fn rollback(&mut self) -> ConfigEffectFuture<'_> {
        Box::pin(async move {
            if self.applied {
                self.apply_snapshot(&self.before).await?;
            }
            Ok(())
        })
    }
}
