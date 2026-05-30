use std::path::PathBuf;

use crate::error::AppError;
use crate::result::Result;

/// OCR 引擎占位实现（阶段 3 完整实现需要 oar-ocr crate，当前仅提供接口）。
pub struct OcrEngine {
    _model_dir: PathBuf,
}

impl OcrEngine {
    pub fn new(model_dir: PathBuf) -> Self {
        Self {
            _model_dir: model_dir,
        }
    }

    /// 对图片执行 OCR 识别（占位实现，返回空字符串）。
    pub fn recognize(&self, _image_data: &[u8]) -> Result<String> {
        // TODO: 集成 oar-ocr 实现真实 OCR
        tracing::warn!("OCR 引擎尚未实现，返回空结果");
        Ok(String::new())
    }

    /// 批量识别（占位）。
    pub fn recognize_batch(&self, images: Vec<Vec<u8>>) -> Result<Vec<String>> {
        images.iter().map(|img| self.recognize(img)).collect()
    }
}

/// OCR 模型下载器（占位实现）。
pub struct ModelDownloader {
    _cache_dir: PathBuf,
    _cdn_url: String,
}

impl ModelDownloader {
    pub fn new(cache_dir: PathBuf, cdn_url: String) -> Self {
        Self {
            _cache_dir: cache_dir,
            _cdn_url: cdn_url,
        }
    }

    /// 确保模型文件已下载并校验（占位实现）。
    pub async fn ensure_models(&self) -> Result<()> {
        // TODO: 实现模型下载与校验逻辑
        tracing::info!("OCR 模型下载器占位实现，跳过下载");
        Ok(())
    }

    async fn _verify_file(&self, _path: &PathBuf, _expected_hash: &str) -> Result<bool> {
        Ok(true)
    }

    async fn _download_model(&self, _name: &str, _dest: &PathBuf) -> Result<()> {
        Err(AppError::ocr("模型下载功能尚未实现"))
    }
}
