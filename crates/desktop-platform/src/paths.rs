use std::path::PathBuf;
use super_paste_core::{Error, Result};
use super_paste_format::{APPLICATION_ID, DATABASE_FILE};

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data: PathBuf,
    pub database: PathBuf,
    pub models: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
}
impl AppPaths {
    pub fn discover() -> Result<Self> {
        let data = dirs::data_local_dir().ok_or_else(|| Error::Platform("无法确定应用数据目录".into()))?.join(APPLICATION_ID);
        let cache = dirs::cache_dir().ok_or_else(|| Error::Platform("无法确定应用缓存目录".into()))?.join(APPLICATION_ID).join("cache");
        Ok(Self { database: data.join(DATABASE_FILE), models: data.join("ocr-models"), logs: data.join("logs"), data, cache })
    }

    pub async fn initialize(&self) -> Result<()> {
        for directory in [&self.data, &self.models, &self.cache, &self.logs] {
            tokio::fs::create_dir_all(directory).await?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).await?;
            }
        }
        Ok(())
    }
}
