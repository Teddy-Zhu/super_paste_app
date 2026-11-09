use crate::{
    credentials::CredentialStore,
    mcp::{McpService, McpSettings, McpState},
    storage::Storage,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    enabled: bool,
    port: u16,
    auth_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key_ref: Option<String>,
}

pub struct McpConfiguration {
    storage: Arc<Storage>,
    credentials: Arc<CredentialStore>,
    service: Arc<McpService>,
    gate: Mutex<()>,
    closed: AtomicBool,
}

pub fn generate_key() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl McpConfiguration {
    pub fn new(
        storage: Arc<Storage>,
        credentials: Arc<CredentialStore>,
        service: Arc<McpService>,
    ) -> Self {
        Self {
            storage,
            credentials,
            service,
            gate: Mutex::new(()),
            closed: AtomicBool::new(false),
        }
    }

    fn ensure_open(&self) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            Err("MCP 服务正在关闭".into())
        } else {
            Ok(())
        }
    }

    async fn stored_or_default(&self) -> Result<StoredSettings, String> {
        if let Some(settings) = self.stored().await? {
            return Ok(settings);
        }
        let settings = StoredSettings {
            enabled: false,
            port: data_format::DEFAULT_MCP_PORT,
            auth_enabled: true,
            api_key_ref: None,
        };
        self.storage
            .write_meta(
                "mcp.settings",
                &serde_json::to_value(&settings).map_err(|e| e.to_string())?,
            )
            .await?;
        Ok(settings)
    }

    async fn stored(&self) -> Result<Option<StoredSettings>, String> {
        self.storage
            .read_meta("mcp.settings")
            .await?
            .map(|value| serde_json::from_value(value).map_err(|e| format!("MCP 配置无效: {e}")))
            .transpose()
    }

    pub async fn initialize(&self) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        self.ensure_open()?;
        let stored = self.stored_or_default().await?;
        // A disabled server needs no vault access. Its fresh key is provisioned
        // when the user opens MCP settings, or by the one-time import tool.
        let key = if stored.enabled {
            match self.read_key(stored.api_key_ref.clone()).await {
                Ok(key) => key,
                Err(error) => {
                    self.service.report_error(error.clone());
                    return Err(error);
                }
            }
        } else {
            String::new()
        };
        self.ensure_open()?;
        self.service
            .apply_settings(McpSettings {
                enabled: stored.enabled,
                port: stored.port,
                auth_enabled: stored.auth_enabled,
                api_key: key,
            })
            .await?;
        Ok(())
    }

    pub async fn diagnostic_state(&self) -> Result<serde_json::Value, String> {
        let stored = self.stored().await?.unwrap_or(StoredSettings {
            enabled: false,
            port: data_format::DEFAULT_MCP_PORT,
            auth_enabled: true,
            api_key_ref: None,
        });
        let status = self.service.state().status;
        Ok(
            serde_json::json!({"enabled":stored.enabled,"port":stored.port,"authEnabled":stored.auth_enabled,"running":status.running,"endpoint":status.endpoint,"error":status.error}),
        )
    }

    pub async fn state(&self) -> Result<McpState, String> {
        let _gate = self.gate.lock().await;
        self.ensure_open()?;
        let mut stored = self.stored_or_default().await?;
        if stored.api_key_ref.is_none() {
            stored.api_key_ref = Some(self.stage_key(generate_key()).await?);
            self.ensure_open()?;
            self.storage
                .write_meta(
                    "mcp.settings",
                    &serde_json::to_value(&stored).map_err(|e| e.to_string())?,
                )
                .await?;
        }
        let key = self.read_key(stored.api_key_ref).await?;
        self.ensure_open()?;
        let mut state = self.service.state();
        state.settings = McpSettings {
            enabled: stored.enabled,
            port: stored.port,
            auth_enabled: stored.auth_enabled,
            api_key: key,
        };
        Ok(state)
    }

    pub async fn save(&self, settings: McpSettings) -> Result<McpState, String> {
        let _gate = self.gate.lock().await;
        self.ensure_open()?;
        if settings.port == 0 {
            return Err("MCP 端口无效".into());
        }
        if settings.enabled && settings.auth_enabled && settings.api_key.trim().is_empty() {
            return Err("启用 MCP 鉴权需要密钥".into());
        }
        let previous = self.service.state().settings;
        let old = self.stored().await?;
        let old_reference = old.and_then(|v| v.api_key_ref);
        let reference = if settings.api_key == previous.api_key && old_reference.is_some() {
            old_reference.clone()
        } else if settings.api_key.is_empty() {
            None
        } else {
            Some(self.stage_key(settings.api_key.clone()).await?)
        };
        let created = reference != old_reference;
        self.ensure_open()?;
        if created && let Some(reference) = old_reference.as_ref() {
            self.remember_cleanup(reference).await?;
        }
        if let Err(error) = self.service.apply_settings(settings.clone()).await {
            if created && let Some(reference) = reference {
                self.remove_reference(reference).await;
            }
            let rollback = self.service.apply_settings(previous).await.err();
            return Err(match rollback {
                Some(rollback) => format!("{error}; MCP 回滚失败: {rollback}"),
                None => error,
            });
        }
        let stored = StoredSettings {
            enabled: settings.enabled,
            port: settings.port,
            auth_enabled: settings.auth_enabled,
            api_key_ref: reference.clone(),
        };
        let saved = self
            .storage
            .write_meta(
                "mcp.settings",
                &serde_json::to_value(stored).map_err(|e| e.to_string())?,
            )
            .await;
        if let Err(error) = saved {
            let rollback = self.service.apply_settings(previous).await.err();
            if created && let Some(reference) = reference {
                self.remove_reference(reference).await;
            }
            return Err(match rollback {
                Some(rollback) => format!("{error}; MCP 回滚失败: {rollback}"),
                None => error,
            });
        }
        if created && let Err(error) = self.cleanup_pending().await {
            tracing::error!(%error,"MCP configuration committed; credential cleanup deferred");
        }
        Ok(self.service.state())
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        self.closed.store(true, Ordering::Release);
        let _gate = self.gate.lock().await;
        self.service.stop().await
    }

    pub async fn retry_cleanup(&self) -> Result<(), String> {
        let _gate = self.gate.lock().await;
        self.ensure_open()?;
        self.cleanup_pending().await
    }

    async fn remember_cleanup(&self, reference: &str) -> Result<(), String> {
        let mut queue = self
            .storage
            .read_meta("mcp.credentials.cleanup")
            .await?
            .map(serde_json::from_value::<Vec<String>>)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        if !queue.iter().any(|value| value == reference) {
            queue.push(reference.to_owned());
            self.storage
                .write_meta("mcp.credentials.cleanup", &serde_json::json!(queue))
                .await?;
        }
        Ok(())
    }

    async fn stage_key(&self, key: String) -> Result<String, String> {
        let reference = CredentialStore::new_reference();
        self.remember_cleanup(&reference).await?;
        let credentials = self.credentials.clone();
        let target = reference.clone();
        tokio::task::spawn_blocking(move || {
            credentials
                .store_secret(&target, &key)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(reference)
    }

    async fn cleanup_pending(&self) -> Result<(), String> {
        let active = self
            .stored()
            .await?
            .and_then(|settings| settings.api_key_ref);
        let queue = self
            .storage
            .read_meta("mcp.credentials.cleanup")
            .await?
            .map(serde_json::from_value::<Vec<String>>)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let mut retained = Vec::new();
        for reference in queue {
            if active.as_ref() != Some(&reference)
                && !self.remove_reference(reference.clone()).await
            {
                retained.push(reference);
            }
        }
        self.storage
            .write_meta("mcp.credentials.cleanup", &serde_json::json!(retained))
            .await
    }

    async fn read_key(&self, reference: Option<String>) -> Result<String, String> {
        match reference {
            Some(reference) => {
                let credentials = self.credentials.clone();
                tokio::task::spawn_blocking(move || {
                    credentials
                        .get_secret(&reference)
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())?
            }
            None => Ok(String::new()),
        }
    }

    async fn remove_reference(&self, reference: String) -> bool {
        let credentials = self.credentials.clone();
        match tokio::task::spawn_blocking(move || credentials.delete_secret(&reference)).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::warn!(%error,"MCP credential retained for later cleanup");
                false
            }
            Err(error) => {
                tracing::error!(%error,"MCP credential cleanup task failed");
                false
            }
        }
    }
}
