use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_PROVIDER_ID: &str = "default";
pub const DEFAULT_PROMPT_ID: &str = "builtin-general";
pub const DEFAULT_GLOSSARY_ID: &str = "global";
pub const DEFAULT_HISTORY_RETENTION: i64 = 50;
pub const DEFAULT_CACHE_MAX_BYTES: i64 = 256 * 1024 * 1024;
pub const DICTIONARY_HISTORY_LIMIT: i64 = 20;
pub const DICTIONARY_DISTRIBUTION_SCHEMA_VERSION: &str = "distribution_entry_v5";
pub const DICTIONARY_SQLITE_SCHEMA_VERSION: &str = "distribution_sqlite_v1";

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
    pub dictionary: DictionaryState,
    pub dictionary_history: Vec<DictionaryHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DictionaryStatus {
    NotInstalled,
    Ready,
    Updating,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryState {
    pub status: DictionaryStatus,
    pub installed_release: Option<String>,
    pub artifact_sha256: Option<String>,
    pub entry_count: Option<i64>,
    pub distribution_schema_version: Option<String>,
    pub sqlite_schema_version: Option<String>,
    pub installed_at: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub database_bytes: u64,
    pub cache_size_bytes: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryHistoryEntry {
    pub normalized_word: String,
    pub display_word: String,
    pub last_queried_at: String,
    pub query_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryLookupResult {
    pub word: String,
    pub normalized_word: String,
    pub entry: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryLookupCommandResult {
    pub lookup: DictionaryLookupResult,
    pub history: Vec<DictionaryHistoryEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryCommandResult {
    pub operation_id: String,
    pub state: DictionaryState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryUpdateStarted {
    pub operation_id: String,
    pub state: DictionaryState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryDownloadProgress {
    pub operation_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryVerifyProgress {
    pub operation_id: String,
    pub current: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryExtractProgress {
    pub operation_id: String,
    pub current: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryUpdateCompleted {
    pub operation_id: String,
    pub state: DictionaryState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryUpdateFailed {
    pub operation_id: String,
    pub message: String,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TranslationOutcome {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationCommandResult {
    pub outcome: TranslationOutcome,
    pub content: Option<String>,
    pub cache_hit: bool,
    pub message: Option<String>,
}

impl TranslationCommandResult {
    pub fn completed(content: impl Into<String>, cache_hit: bool) -> Self {
        Self {
            outcome: TranslationOutcome::Completed,
            content: Some(content.into()),
            cache_hit,
            message: None,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            outcome: TranslationOutcome::Cancelled,
            content: None,
            cache_hit: false,
            message: None,
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            outcome: TranslationOutcome::Failed,
            content: None,
            cache_hit: false,
            message: Some(message.into()),
        }
    }
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

impl DictionaryState {
    pub fn not_installed() -> Self {
        Self {
            status: DictionaryStatus::NotInstalled,
            installed_release: None,
            artifact_sha256: None,
            entry_count: None,
            distribution_schema_version: None,
            sqlite_schema_version: None,
            installed_at: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            database_bytes: 0,
            cache_size_bytes: 0,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TranslationCommandResult, TranslationOutcome};

    #[test]
    fn command_result_constructors_keep_terminal_fields_consistent() {
        assert_eq!(
            TranslationCommandResult::completed("译文", true),
            TranslationCommandResult {
                outcome: TranslationOutcome::Completed,
                content: Some("译文".to_string()),
                cache_hit: true,
                message: None,
            }
        );
        assert_eq!(
            TranslationCommandResult::cancelled(),
            TranslationCommandResult {
                outcome: TranslationOutcome::Cancelled,
                content: None,
                cache_hit: false,
                message: None,
            }
        );
        assert_eq!(
            TranslationCommandResult::failed("请求失败"),
            TranslationCommandResult {
                outcome: TranslationOutcome::Failed,
                content: None,
                cache_hit: false,
                message: Some("请求失败".to_string()),
            }
        );
    }

    #[test]
    fn command_result_serializes_to_frontend_contract() {
        let value =
            serde_json::to_value(TranslationCommandResult::completed("译文", true)).unwrap();
        assert_eq!(value["outcome"], "completed");
        assert_eq!(value["content"], "译文");
        assert_eq!(value["cacheHit"], true);
        assert_eq!(value["message"], serde_json::Value::Null);
    }
}
