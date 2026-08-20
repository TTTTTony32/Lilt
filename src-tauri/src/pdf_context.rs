use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const DOCUMENT_CONTEXT_SCHEMA_VERSION: u32 = 1;
pub const MAX_PREFLIGHT_SAMPLE_COUNT: usize = 12;
pub const MAX_PREFLIGHT_SAMPLE_CHARS: usize = 24_000;
pub const MAX_CONTEXT_FIELD_CHARS: usize = 8_000;
pub const MAX_CONTEXT_LIST_ITEMS: usize = 64;
pub const MAX_CONTEXT_ITEM_CHARS: usize = 1_000;
const MAX_BOUNDED_VALUE_DEPTH: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DocumentContext {
    pub schema_version: u32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "abstract", default)]
    pub abstract_text: Option<String>,
    #[serde(default)]
    pub document_type: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub headings: Vec<String>,
    #[serde(default)]
    pub key_terms: Vec<DocumentTerm>,
    #[serde(default)]
    pub abbreviations: Vec<DocumentAbbreviation>,
    #[serde(default)]
    pub translation_notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DocumentTerm {
    pub source: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DocumentAbbreviation {
    pub abbreviation: String,
    #[serde(default)]
    pub expanded: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

impl Default for DocumentContext {
    fn default() -> Self {
        Self {
            schema_version: DOCUMENT_CONTEXT_SCHEMA_VERSION,
            title: None,
            abstract_text: None,
            document_type: None,
            domain: None,
            headings: Vec::new(),
            key_terms: Vec::new(),
            abbreviations: Vec::new(),
            translation_notes: Vec::new(),
            context_hash: None,
        }
    }
}

impl DocumentContext {
    pub fn empty() -> Self {
        let mut context = Self::default();
        context.refresh_hash();
        context
    }

    pub fn from_model_output(value: &str) -> Result<Self, String> {
        let json_text = value
            .trim()
            .strip_prefix("```json")
            .unwrap_or(value.trim())
            .strip_prefix("```")
            .unwrap_or(value.trim())
            .strip_suffix("```")
            .unwrap_or(value.trim())
            .trim();
        let parsed = serde_json::from_str::<Value>(json_text)
            .map_err(|error| format!("Document Preflight JSON 解析失败：{error}"))?;
        Self::from_value(parsed)
    }

    pub fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "DocumentContext 必须是 JSON 对象".to_string())?;
        let schema_version = object
            .get("schema_version")
            .or_else(|| object.get("schemaVersion"))
            .and_then(Value::as_u64)
            .map(|value| value as u32)
            .unwrap_or(DOCUMENT_CONTEXT_SCHEMA_VERSION);
        if schema_version != DOCUMENT_CONTEXT_SCHEMA_VERSION {
            return Err(format!(
                "不支持的 DocumentContext schema 版本：{schema_version}"
            ));
        }

        let mut context = DocumentContext {
            schema_version,
            title: bounded_optional_string(object, &["title"], MAX_CONTEXT_FIELD_CHARS),
            abstract_text: bounded_optional_string(
                object,
                &["abstract", "abstract_text", "summary"],
                MAX_CONTEXT_FIELD_CHARS,
            ),
            document_type: bounded_optional_string(
                object,
                &["document_type", "documentType", "type"],
                MAX_CONTEXT_ITEM_CHARS,
            ),
            domain: bounded_optional_string(object, &["domain", "field"], MAX_CONTEXT_ITEM_CHARS),
            headings: bounded_string_array(
                object.get("headings"),
                MAX_CONTEXT_LIST_ITEMS,
                MAX_CONTEXT_ITEM_CHARS,
            ),
            key_terms: parse_terms(object.get("key_terms").or_else(|| object.get("keyTerms"))),
            abbreviations: parse_abbreviations(object.get("abbreviations")),
            translation_notes: bounded_string_array(
                object
                    .get("translation_notes")
                    .or_else(|| object.get("translationNotes")),
                MAX_CONTEXT_LIST_ITEMS,
                MAX_CONTEXT_ITEM_CHARS,
            ),
            context_hash: None,
        };
        context.refresh_hash();
        Ok(context)
    }

    pub fn refresh_hash(&mut self) {
        self.context_hash = None;
        let canonical = serde_json::to_vec(self).unwrap_or_default();
        let digest = Sha256::digest(canonical);
        self.context_hash = Some(format!("{digest:x}"));
    }

    pub fn hash(&self) -> &str {
        self.context_hash.as_deref().unwrap_or("")
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Object(Default::default()))
    }
}

pub fn hash_value(value: &Value) -> String {
    let canonical = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(canonical);
    format!("{digest:x}")
}

/// Bounds opaque PDF prompt fields before they are serialized into a Provider
/// request or a cache key. The Worker already applies the same policy, but the
/// Rust boundary must remain safe when an older or external Worker sends a
/// larger value.
pub fn bounded_value(value: &Value) -> Value {
    bounded_value_at(value, 0)
}

fn bounded_value_at(value: &Value, depth: usize) -> Value {
    if depth >= MAX_BOUNDED_VALUE_DEPTH {
        return match value {
            Value::Null => Value::Null,
            Value::Bool(value) => Value::Bool(*value),
            Value::Number(value) => Value::Number(value.clone()),
            Value::String(value) => {
                Value::String(value.chars().take(MAX_CONTEXT_ITEM_CHARS).collect())
            }
            Value::Array(_) | Value::Object(_) => Value::Null,
        };
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(value) => {
            Value::String(value.chars().take(MAX_CONTEXT_FIELD_CHARS).collect())
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(MAX_CONTEXT_LIST_ITEMS)
                .map(|value| bounded_value_at(value, depth + 1))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .take(MAX_CONTEXT_LIST_ITEMS)
                .map(|(key, value)| (key.clone(), bounded_value_at(value, depth + 1)))
                .collect(),
        ),
    }
}

fn bounded_optional_string(
    object: &serde_json::Map<String, Value>,
    names: &[&str],
    maximum: usize,
) -> Option<String> {
    names
        .iter()
        .find_map(|name| object.get(*name).and_then(Value::as_str))
        .map(|value| value.trim().chars().take(maximum).collect())
        .filter(|value: &String| !value.is_empty())
}

fn bounded_string_array(
    value: Option<&Value>,
    maximum_items: usize,
    maximum_chars: usize,
) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| item.trim().chars().take(maximum_chars).collect::<String>())
                .filter(|item| !item.is_empty())
                .take(maximum_items)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_terms(value: Option<&Value>) -> Vec<DocumentTerm> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let object = item.as_object()?;
                    let source = object
                        .get("source")
                        .or_else(|| object.get("term"))
                        .or_else(|| object.get("original"))
                        .and_then(Value::as_str)?
                        .trim()
                        .chars()
                        .take(MAX_CONTEXT_ITEM_CHARS)
                        .collect::<String>();
                    if source.is_empty() {
                        return None;
                    }
                    Some(DocumentTerm {
                        source,
                        target: bounded_optional_string(
                            object,
                            &["target", "translation"],
                            MAX_CONTEXT_ITEM_CHARS,
                        ),
                        source_kind: bounded_optional_string(
                            object,
                            &["source_kind", "kind"],
                            MAX_CONTEXT_ITEM_CHARS,
                        ),
                        confidence: bounded_confidence(object.get("confidence")),
                        note: bounded_optional_string(
                            object,
                            &["note", "reason"],
                            MAX_CONTEXT_ITEM_CHARS,
                        ),
                    })
                })
                .take(MAX_CONTEXT_LIST_ITEMS)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_abbreviations(value: Option<&Value>) -> Vec<DocumentAbbreviation> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let object = item.as_object()?;
                    let abbreviation = object
                        .get("abbreviation")
                        .or_else(|| object.get("short"))
                        .and_then(Value::as_str)?
                        .trim()
                        .chars()
                        .take(MAX_CONTEXT_ITEM_CHARS)
                        .collect::<String>();
                    if abbreviation.is_empty() {
                        return None;
                    }
                    Some(DocumentAbbreviation {
                        abbreviation,
                        expanded: bounded_optional_string(
                            object,
                            &["expanded", "expansion", "full_form"],
                            MAX_CONTEXT_ITEM_CHARS,
                        ),
                        target: bounded_optional_string(
                            object,
                            &["target", "translation"],
                            MAX_CONTEXT_ITEM_CHARS,
                        ),
                        confidence: bounded_confidence(object.get("confidence")),
                    })
                })
                .take(MAX_CONTEXT_LIST_ITEMS)
                .collect()
        })
        .unwrap_or_default()
}

fn bounded_confidence(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
}

#[cfg(test)]
mod tests {
    use super::{DOCUMENT_CONTEXT_SCHEMA_VERSION, DocumentContext, bounded_value, hash_value};
    use serde_json::json;

    #[test]
    fn context_normalization_accepts_aliases_and_adds_stable_hash() {
        let context = DocumentContext::from_value(json!({
            "title": "A paper",
            "summary": "Summary",
            "keyTerms": [{"term": "cache", "translation": "缓存"}],
            "abbreviations": [{"short": "API", "expansion": "Application Programming Interface"}]
        }))
        .expect("context should normalize");
        assert_eq!(context.schema_version, DOCUMENT_CONTEXT_SCHEMA_VERSION);
        assert_eq!(context.abstract_text.as_deref(), Some("Summary"));
        assert_eq!(context.key_terms[0].source, "cache");
        assert_eq!(context.abbreviations[0].abbreviation, "API");
        assert_eq!(context.hash().len(), 64);
    }

    #[test]
    fn context_hash_changes_when_context_changes() {
        let first = DocumentContext::from_value(json!({"title": "one"})).unwrap();
        let second = DocumentContext::from_value(json!({"title": "two"})).unwrap();
        assert_ne!(first.hash(), second.hash());
        assert_eq!(hash_value(&json!({"a": 1})), hash_value(&json!({"a": 1})));
    }

    #[test]
    fn malformed_context_is_rejected() {
        assert!(DocumentContext::from_value(json!("not an object")).is_err());
        assert!(DocumentContext::from_value(json!({"schema_version": 2})).is_err());
    }

    #[test]
    fn opaque_prompt_values_are_bounded_recursively() {
        let value = json!({
            "long": "x".repeat(10_000),
            "nested": [{"value": "y".repeat(10_000)}],
        });
        let bounded = bounded_value(&value);
        assert_eq!(bounded["long"].as_str().unwrap().chars().count(), 8_000);
        assert_eq!(
            bounded["nested"][0]["value"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            8_000
        );
    }
}
