use std::sync::Arc;

pub type Result<T> = std::result::Result<T, Error>;

/// Cloneable errors let concurrent consumers observe the same failed shared task.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(Arc<str>),
    #[error("记录不存在或已被删除")]
    NotFound,
    #[error("操作已取消")]
    Cancelled,
    #[error("资源限制：{0}")]
    ResourceLimit(Arc<str>),
    #[error("数据库错误：{0}")]
    Database(Arc<str>),
    #[error("文件访问失败：{0}")]
    Io(Arc<str>),
    #[error("系统能力调用失败：{0}")]
    Platform(Arc<str>),
    #[error("图片处理失败：{0}")]
    Image(Arc<str>),
    #[error("后台任务失败：{0}")]
    Task(Arc<str>),
    #[error("配置已被其他操作修改：{0}")]
    Conflict(Arc<str>),
    #[error("系统凭据库不可用")]
    CredentialUnavailable,
    #[error("系统凭据操作失败")]
    CredentialOperation,
    #[error("系统凭据内容格式无效")]
    CredentialEncoding,
}
impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self { Self::Database(value.to_string().into()) }
}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        value.get_ref().and_then(|error| error.downcast_ref::<Error>()).cloned()
            .or_else(|| embedded_error(&value)).unwrap_or_else(|| Self::Io(value.to_string().into()))
    }
}
impl From<image::ImageError> for Error {
    fn from(value: image::ImageError) -> Self {
        match value {
            image::ImageError::Limits(error) => Self::ResourceLimit(error.to_string().into()),
            image::ImageError::IoError(error) => error.into(),
            other => Self::Image(other.to_string().into()),
        }
    }
}
impl From<png::DecodingError> for Error {
    fn from(value: png::DecodingError) -> Self {
        match value {
            png::DecodingError::LimitsExceeded => Self::ResourceLimit("PNG 解码工作区超出限制".into()),
            png::DecodingError::IoError(error) => error.into(),
            other => Self::Image(other.to_string().into()),
        }
    }
}
impl From<super_paste_format::ValidationError> for Error {
    fn from(value: super_paste_format::ValidationError) -> Self { Self::Invalid(value.0.into()) }
}
impl From<tokio::task::JoinError> for Error {
    fn from(value: tokio::task::JoinError) -> Self { Self::Task(value.to_string().into()) }
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self { embedded_error(&value).unwrap_or_else(|| Self::Invalid(value.to_string().into())) }
}
fn embedded_error(error: &(dyn std::error::Error + 'static)) -> Option<Error> {
    let mut source = error.source();
    while let Some(value) = source {
        if let Some(error) = value.downcast_ref::<Error>() { return Some(error.clone()); }
        source = value.source();
    }
    None
}

