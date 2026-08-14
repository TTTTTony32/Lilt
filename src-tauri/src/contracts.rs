use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_PROVIDER_ID: &str = "default";
pub const DEFAULT_PROMPT_ID: &str = "builtin-general";
pub const DEFAULT_THINKING_EFFORT: ThinkingEffort = ThinkingEffort::None;
pub const DEFAULT_GLOSSARY_ID: &str = "global";
pub const DEFAULT_HISTORY_RETENTION: i64 = 50;
pub const DEFAULT_CACHE_MAX_BYTES: i64 = 256 * 1024 * 1024;
pub const DEFAULT_WORD_AI_CACHE_ENABLED: bool = true;
pub const DEFAULT_PARAGRAPH_EXAMPLE_LOOKUP_ENABLED: bool = true;
pub const DEFAULT_SELECTION_SHORTCUT: &str = "Ctrl+Shift+L";
pub const DEFAULT_SELECTION_MODE: SelectionMode = SelectionMode::Shortcut;
pub const DEFAULT_SELECTION_WINDOW_WIDTH: i64 = 560;
pub const DEFAULT_SELECTION_WINDOW_HEIGHT: i64 = 320;
pub const MIN_SELECTION_WINDOW_WIDTH: i64 = 360;
pub const MAX_SELECTION_WINDOW_WIDTH: i64 = 1200;
pub const MIN_SELECTION_WINDOW_HEIGHT: i64 = 240;
pub const MAX_SELECTION_WINDOW_HEIGHT: i64 = 900;
pub const DICTIONARY_HISTORY_LIMIT: i64 = 20;
pub const DICTIONARY_DISTRIBUTION_SCHEMA_VERSION: &str = "distribution_entry_v5";
pub const DICTIONARY_SQLITE_SCHEMA_VERSION: &str = "distribution_sqlite_v1";
pub const WORD_EXAMPLE_PROTOCOL_VERSION: &str = "word-example-v1";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub history_retention: i64,
    pub cache_enabled: bool,
    pub cache_max_bytes: i64,
    pub cache_usage_bytes: i64,
    pub word_ai_cache_enabled: bool,
    pub paragraph_example_lookup_enabled: bool,
    pub selection_mode: SelectionMode,
    pub selection_shortcut: String,
    pub selection_window_width: i64,
    pub selection_window_height: i64,
    pub close_behavior: CloseBehavior,
}

pub fn parse_selection_window_dimension(
    value: Option<&str>,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> i64 {
    value
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| (*value >= minimum) && (*value <= maximum))
        .unwrap_or(default)
}

pub fn clamp_selection_window_dimension(
    value: f64,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> i64 {
    if !value.is_finite() {
        return default;
    }
    value.round().clamp(minimum as f64, maximum as f64) as i64
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionMode {
    Shortcut,
    Automatic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CloseBehavior {
    Ask,
    Exit,
    Tray,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingEffort {
    None,
    Low,
    Medium,
    High,
}

impl Default for ThinkingEffort {
    fn default() -> Self {
        Self::None
    }
}

impl ThinkingEffort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Self {
        match value {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionTrigger {
    Shortcut,
    Automatic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionAnchor {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionTriggerNotice {
    pub trigger_id: String,
    pub trigger: SelectionTrigger,
    pub anchor: Option<SelectionAnchor>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionNotice {
    pub request_id: String,
    pub trigger_id: String,
    pub trigger: SelectionTrigger,
    pub anchor: Option<SelectionAnchor>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionUnavailable {
    pub request_id: Option<String>,
    pub trigger_id: String,
    pub trigger: SelectionTrigger,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionStatusChanged {
    pub mode: SelectionMode,
    pub shortcut: String,
    pub shortcut_registered: bool,
    pub ui_automation_ready: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionRuntimeStatus {
    pub mode: SelectionMode,
    pub shortcut: String,
    pub shortcut_registered: bool,
    pub ui_automation_ready: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionRequestPayload {
    pub request_id: String,
    pub source_text: String,
    pub source_language: String,
    pub target_language: String,
    pub trigger: SelectionTrigger,
    pub anchor: Option<SelectionAnchor>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSettingsResult {
    pub settings: AppSettings,
    pub status: SelectionRuntimeStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    pub prompt_id: String,
    pub thinking_effort: ThinkingEffort,
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
pub struct PersonalDictionaryEntry {
    pub normalized_canonical_word: String,
    pub canonical_word: String,
    pub lookup_word: String,
    pub saved_at: String,
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
    pub personal_dictionary: Vec<PersonalDictionaryEntry>,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DictionaryMatchType {
    Exact,
    Form,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryLookupResult {
    pub word: String,
    pub normalized_word: String,
    pub canonical_word: String,
    pub match_type: DictionaryMatchType,
    pub entry: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryLookupCandidate {
    pub canonical_word: String,
    pub normalized_canonical_word: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphExample {
    pub example_id: i64,
    pub source_text: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryLookupCommandResult {
    pub lookup: Option<DictionaryLookupResult>,
    pub candidates: Vec<DictionaryLookupCandidate>,
    pub example: Option<ParagraphExample>,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordExampleRequest {
    pub request_id: String,
    pub example_id: i64,
    pub word: String,
    pub canonical_word: String,
    pub target_language: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordExampleStarted {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordExampleTranslationDelta {
    pub request_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordExamplePosDelta {
    pub request_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordExampleCompleted {
    pub request_id: String,
    pub translation: String,
    pub part_of_speech: String,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordExampleCancelled {
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordExampleFailed {
    pub request_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WordExampleOutcome {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WordExampleCommandResult {
    pub outcome: WordExampleOutcome,
    pub translation: Option<String>,
    pub part_of_speech: Option<String>,
    pub cache_hit: bool,
    pub message: Option<String>,
}

impl WordExampleCommandResult {
    pub fn completed(
        translation: impl Into<String>,
        part_of_speech: impl Into<String>,
        cache_hit: bool,
    ) -> Self {
        Self {
            outcome: WordExampleOutcome::Completed,
            translation: Some(translation.into()),
            part_of_speech: Some(part_of_speech.into()),
            cache_hit,
            message: None,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            outcome: WordExampleOutcome::Cancelled,
            translation: None,
            part_of_speech: None,
            cache_hit: false,
            message: None,
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            outcome: WordExampleOutcome::Failed,
            translation: None,
            part_of_speech: None,
            cache_hit: false,
            message: Some(message.into()),
        }
    }
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
    pub thinking_effort: ThinkingEffort,
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
    use super::{
        clamp_selection_window_dimension, parse_selection_window_dimension, ThinkingEffort,
        TranslationCommandResult, TranslationOutcome, DEFAULT_SELECTION_WINDOW_HEIGHT,
        DEFAULT_SELECTION_WINDOW_WIDTH, MAX_SELECTION_WINDOW_HEIGHT, MAX_SELECTION_WINDOW_WIDTH,
        MIN_SELECTION_WINDOW_HEIGHT, MIN_SELECTION_WINDOW_WIDTH,
    };

    #[test]
    fn selection_window_dimensions_parse_and_clamp_at_the_contract_boundary() {
        assert_eq!(
            parse_selection_window_dimension(
                Some("800"),
                DEFAULT_SELECTION_WINDOW_WIDTH,
                MIN_SELECTION_WINDOW_WIDTH,
                MAX_SELECTION_WINDOW_WIDTH,
            ),
            800
        );
        assert_eq!(
            parse_selection_window_dimension(
                Some("not-a-size"),
                DEFAULT_SELECTION_WINDOW_WIDTH,
                MIN_SELECTION_WINDOW_WIDTH,
                MAX_SELECTION_WINDOW_WIDTH,
            ),
            DEFAULT_SELECTION_WINDOW_WIDTH
        );
        assert_eq!(
            parse_selection_window_dimension(
                Some("120"),
                DEFAULT_SELECTION_WINDOW_WIDTH,
                MIN_SELECTION_WINDOW_WIDTH,
                MAX_SELECTION_WINDOW_WIDTH,
            ),
            DEFAULT_SELECTION_WINDOW_WIDTH
        );
        assert_eq!(
            clamp_selection_window_dimension(
                1_500.4,
                DEFAULT_SELECTION_WINDOW_WIDTH,
                MIN_SELECTION_WINDOW_WIDTH,
                MAX_SELECTION_WINDOW_WIDTH,
            ),
            MAX_SELECTION_WINDOW_WIDTH
        );
        assert_eq!(
            clamp_selection_window_dimension(
                120.0,
                DEFAULT_SELECTION_WINDOW_HEIGHT,
                MIN_SELECTION_WINDOW_HEIGHT,
                MAX_SELECTION_WINDOW_HEIGHT,
            ),
            MIN_SELECTION_WINDOW_HEIGHT
        );
        assert_eq!(
            clamp_selection_window_dimension(
                f64::NAN,
                DEFAULT_SELECTION_WINDOW_HEIGHT,
                MIN_SELECTION_WINDOW_HEIGHT,
                MAX_SELECTION_WINDOW_HEIGHT,
            ),
            DEFAULT_SELECTION_WINDOW_HEIGHT
        );
    }

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

    #[test]
    fn thinking_effort_serializes_to_the_provider_wire_values() {
        for (effort, expected) in [
            (ThinkingEffort::None, "none"),
            (ThinkingEffort::Low, "low"),
            (ThinkingEffort::Medium, "medium"),
            (ThinkingEffort::High, "high"),
        ] {
            assert_eq!(serde_json::to_value(effort).unwrap(), expected);
            assert_eq!(effort.as_str(), expected);
        }
    }
}
