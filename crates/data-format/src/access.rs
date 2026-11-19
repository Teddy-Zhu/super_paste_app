use std::sync::Arc;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShortcutState { Disabled, Registered, PortalRequested, Failed }
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShortcutStatus { pub accelerator: Arc<str>, pub state: ShortcutState, pub error: Option<Arc<str>> }
