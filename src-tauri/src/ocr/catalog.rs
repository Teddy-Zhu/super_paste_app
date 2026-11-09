use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

const MANIFEST: &str = include_str!("../../../resources/ocr-models.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub group: String,
    pub description: String,
    pub archive: String,
    pub size: u64,
    pub sha256: Option<String>,
    pub optimize_space: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    release_url: String,
    default_model_id: String,
    models: Vec<ModelInfo>,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(MANIFEST).expect("built-in OCR model catalog must be valid JSON")
});

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

pub fn all_models() -> &'static [ModelInfo] {
    &CATALOG.models
}

pub fn default_model_id() -> &'static str {
    &CATALOG.default_model_id
}

pub fn release_url(model: &ModelInfo) -> String {
    format!("{}{}", CATALOG.release_url, model.archive)
}

pub fn get_model(id: &str) -> Result<&'static ModelInfo, String> {
    CATALOG
        .models
        .iter()
        .find(|model| model.id == id)
        .ok_or_else(|| "不支持的 OCR 模型".to_owned())
}

pub fn model_revision(model: &ModelInfo) -> String {
    let expected = model
        .sha256
        .clone()
        .unwrap_or_else(|| model.size.to_string());
    format!("{}:{expected}", model.archive)
}

pub fn model_status(id: &str, state: ModelState, stage: Option<ModelStage>) -> OcrModelStatus {
    let total = get_model(id).map(|model| model.size).unwrap_or_default();
    OcrModelStatus {
        id: id.to_owned(),
        state,
        stage,
        received_bytes: 0,
        total_bytes: total,
        error: None,
    }
}
