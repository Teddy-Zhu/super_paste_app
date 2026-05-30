use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictStrategy {
    LocalWins,
    RemoteWins,
    MergeByTime,
}

impl ConflictStrategy {
    pub fn from_str(s: &str) -> Self {
        match s {
            "local_wins" => Self::LocalWins,
            "remote_wins" => Self::RemoteWins,
            _ => Self::MergeByTime,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled: bool,
    pub server_url: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
    pub strategy: ConflictStrategy,
}
