use serde::{Deserialize, Serialize};

pub const DEFAULT_PROVIDER_ID: &str = "default";
pub const DEFAULT_PROMPT_ID: &str = "builtin-general";
pub const DEFAULT_GLOSSARY_ID: &str = "global";
pub const DEFAULT_HISTORY_RETENTION: i64 = 50;
pub const DEFAULT_CACHE_MAX_BYTES: i64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub history_retention: i64,
    pub cache_enabled: bool,
    pub cache_max_bytes: i64,
    pub cache_usage_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    pub prompt_id: String,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: String,
    pub name: String,
    pub content: String,
    pub version: i64,
    pub is_builtin: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryTerm {
    pub id: String,
    pub source: String,
    pub target: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub created_at: String,
    pub source_text: String,
    pub translated_text: String,
    pub source_language: String,
    pub target_language: String,
    pub provider_name: String,
    pub model_id: String,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub usage_bytes: i64,
    pub entry_count: i64,
    pub max_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub settings: AppSettings,
    pub provider: ProviderConfig,
    pub models: Vec<ModelInfo>,
    pub prompts: Vec<Prompt>,
    pub glossary_terms: Vec<GlossaryTerm>,
    pub history: Vec<HistoryEntry>,
    pub cache_stats: CacheStats,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    pub request_id: String,
    pub source_text: String,
    pub source_language: String,
    pub target_language: String,
    pub model_id: String,
    pub prompt_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationStarted {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationDelta {
    pub request_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationCompleted {
    pub request_id: String,
    pub content: String,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationCancelled {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationFailed {
    pub request_id: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ProviderRecord {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    pub prompt_id: String,
}

#[derive(Debug, Clone)]
pub struct CachedTranslation {
    pub translated_text: String,
}
