use std::{collections::BTreeMap, future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAccess {
    Main,
    Overlay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialAvailability {
    Available,
    Configured,
    Unset,
    Unavailable,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreSnapshot {
    pub value: Value,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigReadResult {
    pub snapshot: StoreSnapshot,
    pub field_versions: BTreeMap<String, i64>,
    pub credential_fields: BTreeMap<String, CredentialAvailability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigBootstrap {
    pub settings: ConfigReadResult,
    pub sync: ConfigReadResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreMutationMetadata {
    pub client_id: String,
    pub mutation_id: String,
    #[serde(default)]
    pub base_versions: BTreeMap<String, i64>,
    #[serde(default)]
    pub credential_changes: BTreeMap<String, CredentialChange>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CredentialChange {
    Set,
    Unset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreWriteResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<StoreSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_versions: Option<BTreeMap<String, i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_fields: Option<BTreeMap<String, CredentialAvailability>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub settings: Value,
    pub sync: Value,
    pub versions: BTreeMap<String, i64>,
}

#[derive(Debug, Clone)]
pub struct ConfigChange {
    pub store_id: String,
    pub key: String,
}

pub type ConfigEffectFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait PreparedConfigEffect: Send {
    fn apply(&mut self) -> ConfigEffectFuture<'_>;
    fn rollback(&mut self) -> ConfigEffectFuture<'_>;
}

pub trait ConfigEffectHooks: Send + Sync {
    fn prepare<'a>(
        &'a self,
        before: &'a ConfigSnapshot,
        after: &'a ConfigSnapshot,
        changes: &'a [ConfigChange],
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn PreparedConfigEffect>, String>> + Send + 'a>>;
}

impl StoreWriteResult {
    pub(crate) fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            snapshot: None,
            field_versions: None,
            credential_fields: None,
            error: Some(error.into()),
        }
    }
}
