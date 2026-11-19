use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

const MANIFEST: &str = include_str!("../../../../resources/ocr-models.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub group: String,
    pub description: String,
    pub archive: String,
    pub size: u64,
    pub sha256: String,
    pub optimize_space: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    release_url: String,
    default_model_id: String,
    models: Vec<ModelInfo>,
}

static CATALOG: LazyLock<Result<Catalog, String>> = LazyLock::new(|| {
    let mut catalog: Catalog = serde_json::from_str(MANIFEST).map_err(|error| format!("内置 OCR 模型目录格式无效：{error}"))?;
    let url = reqwest::Url::parse(&catalog.release_url).map_err(|_| "内置 OCR 模型下载地址无效".to_owned())?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || !catalog.release_url.ends_with('/') {
        return Err("内置 OCR 模型下载地址无效".to_owned());
    }
    let mut ids = std::collections::HashSet::new();
    for model in &mut catalog.models {
        if model.id.is_empty() || !model.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) || !ids.insert(model.id.clone())
            || model.size == 0 || !model.archive.ends_with(".zip") || model.archive.contains(['/', '\\']) || model.archive.contains("..") {
            return Err("内置 OCR 模型条目无效".to_owned());
        }
        if model.sha256.len() != 64 || !model.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("内置 OCR 模型缺少有效 SHA-256".to_owned());
        }
        model.sha256.make_ascii_lowercase();
    }
    if !ids.contains(&catalog.default_model_id) { return Err("内置 OCR 默认模型不存在".to_owned()); }
    Ok(catalog)
});
fn catalog() -> Result<&'static Catalog, String> { CATALOG.as_ref().map_err(Clone::clone) }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ModelState {
    NotDownloaded,
    Downloading,
    Ready,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ModelStage {
    Downloading,
    Extracting,
    PreparingDetector,
    Verifying,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelStatus {
    pub id: String,
    pub state: ModelState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<ModelStage>,
    pub received_bytes: u64,
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn all_models() -> Result<&'static [ModelInfo], String> {
    Ok(&catalog()?.models)
}

pub fn default_model_id() -> Result<&'static str, String> {
    Ok(&catalog()?.default_model_id)
}

pub fn release_url(model: &ModelInfo) -> Result<String, String> {
    Ok(format!("{}{}", catalog()?.release_url, model.archive))
}

pub fn get_model(id: &str) -> Result<&'static ModelInfo, String> {
    catalog()?
        .models
        .iter()
        .find(|model| model.id == id)
        .ok_or_else(|| "不支持的 OCR 模型".to_owned())
}

pub fn model_revision(model: &ModelInfo) -> String {
    format!("{}:{}", model.archive, model.sha256)
}

pub fn model_status(model: &ModelInfo, state: ModelState, stage: Option<ModelStage>) -> OcrModelStatus {
    OcrModelStatus {
        id: model.id.clone(),
        state,
        stage,
        received_bytes: 0,
        total_bytes: model.size,
        error: None,
    }
}
