use std::path::PathBuf;

use crate::error::AppError;
use crate::result::Result;
use crate::types::sync::ConflictStrategy;

/// WebDAV 同步服务（占位实现，阶段 3 完整实现需要 reqwest_dav）。
pub struct WebDavSync {
    _server_url: String,
    _username: String,
    _password: String,
    _remote_path: String,
    _local_db_path: PathBuf,
}

impl WebDavSync {
    pub fn new(
        server_url: String,
        username: String,
        password: String,
        remote_path: String,
        local_db_path: PathBuf,
    ) -> Self {
        Self {
            _server_url: server_url,
            _username: username,
            _password: password,
            _remote_path: remote_path,
            _local_db_path: local_db_path,
        }
    }

    /// 双向同步（占位实现）。
    pub async fn sync_bidirectional(&self, _strategy: ConflictStrategy) -> Result<usize> {
        // TODO: 实现 WebDAV 上传/下载与冲突解决
        tracing::warn!("WebDAV 同步功能尚未实现");
        Ok(0)
    }

    /// 仅上传本地数据库到远程（占位）。
    pub async fn upload(&self) -> Result<()> {
        Err(AppError::sync("上传功能尚未实现"))
    }

    /// 仅下载远程数据库到本地（占位）。
    pub async fn download(&self) -> Result<()> {
        Err(AppError::sync("下载功能尚未实现"))
    }
}

/// 冲突解决器（占位实现）。
pub struct ConflictResolver;

impl ConflictResolver {
    /// 合并本地与远程数据库（占位实现）。
    pub fn merge_databases(
        _local: &PathBuf,
        _remote: &PathBuf,
        _strategy: ConflictStrategy,
    ) -> Result<MergeResult> {
        tracing::warn!("数据库合并功能尚未实现");
        Ok(MergeResult {
            merged_count: 0,
            conflict_count: 0,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MergeResult {
    pub merged_count: usize,
    pub conflict_count: usize,
}
