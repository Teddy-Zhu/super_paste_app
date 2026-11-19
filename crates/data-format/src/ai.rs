use serde::{Deserialize, Serialize};
use crate::ValidationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider { Openai, Claude }
impl AiProvider {
    pub fn as_str(self) -> &'static str { match self { Self::Openai => "openai", Self::Claude => "claude" } }
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        match value { "openai" => Ok(Self::Openai), "claude" => Ok(Self::Claude), _ => Err(ValidationError("不支持的 AI OCR 服务".into())) }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiPurpose { Ocr, ConnectionTest }
impl AiPurpose {
    pub fn as_str(self) -> &'static str { match self { Self::Ocr => "ocr", Self::ConnectionTest => "connection-test" } }
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        match value { "ocr" => Ok(Self::Ocr), "connection-test" => Ok(Self::ConnectionTest), _ => Err(ValidationError("AI OCR 请求用途无效".into())) }
    }
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTokens {
    pub input_tokens: Option<u64>, pub output_tokens: Option<u64>,
    pub cached_input_tokens: u64, pub cache_write_tokens: u64, pub total_tokens: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageEntry {
    pub id: uuid::Uuid, pub provider: AiProvider, pub base_url: String, pub model: String, pub purpose: AiPurpose,
    pub started_at: i64, pub finished_at: i64, pub duration_ms: u64, pub success: bool,
    #[serde(flatten)] pub tokens: AiTokens,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageTotals {
    pub requests: u64, pub failures: u64, pub unknown_usage: u64, pub input_tokens: u64, pub output_tokens: u64,
    pub cached_input_tokens: u64, pub cache_write_tokens: u64, pub total_tokens: u64, pub duration_ms: u64,
    pub first_used_at: Option<i64>, pub last_used_at: Option<i64>,
}
impl AiUsageTotals {
    pub fn merge(&mut self, value: &Self) -> Result<(), ValidationError> {
        macro_rules! add { ($($field:ident),*) => { $(self.$field = self.$field.checked_add(value.$field).ok_or_else(|| ValidationError("AI OCR 统计数值溢出".into()))?;)* }; }
        add!(requests, failures, unknown_usage, input_tokens, output_tokens, cached_input_tokens, cache_write_tokens, total_tokens, duration_ms);
        self.first_used_at = match (self.first_used_at, value.first_used_at) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
        self.last_used_at = match (self.last_used_at, value.last_used_at) { (Some(a), Some(b)) => Some(a.max(b)), (a, b) => a.or(b) };
        Ok(())
    }
}
#[derive(Clone, Debug, Default)]
pub struct AiUsageQuery { pub from: Option<i64>, pub to: Option<i64> }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelUsage { pub provider: AiProvider, pub base_url: String, pub model: String, #[serde(flatten)] pub totals: AiUsageTotals }
#[derive(Clone, Debug, Serialize)]
pub struct AiDailyUsage { pub day: String, #[serde(flatten)] pub totals: AiUsageTotals }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageStats {
    pub cleared_at: Option<i64>, pub totals: AiUsageTotals,
    pub models: Vec<AiModelUsage>, pub days: Vec<AiDailyUsage>, pub recent: Vec<AiUsageEntry>,
}
#[derive(Clone, Debug, Serialize)]
pub struct AiModelOption { pub id: String, pub name: String }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConnectionTest { pub latency_ms: u64, pub model: String }
