use crate::translation_core::TranslationMode;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

pub const PDF_WORKER_PROTOCOL_VERSION: u32 = 1;
pub const MAX_PROTOCOL_LINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("PDF Worker 协议消息为空")]
    EmptyLine,
    #[error("PDF Worker 协议消息不能包含多个 JSONL 帧")]
    MultipleFrames,
    #[error("PDF Worker 协议消息超过大小限制：{0} 字节")]
    MessageTooLarge(usize),
    #[error("PDF Worker 协议消息序列化失败：{0}")]
    Serialization(String),
    #[error("PDF Worker 协议消息解析失败：{0}")]
    Deserialization(String),
    #[error("PDF Worker 翻译模式不受支持：{0}")]
    UnsupportedMode(String),
    #[error("PDF Worker 协议版本不受支持：{0}")]
    UnsupportedProtocolVersion(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum RustToWorkerMessage {
    #[serde(rename = "START_JOB")]
    StartJob(StartJobMessage),
    #[serde(rename = "CANCEL_JOB")]
    CancelJob(CancelJobMessage),
    #[serde(rename = "TRANSLATE_RESPONSE")]
    TranslateResponse(TranslateResponseMessage),
    #[serde(rename = "DOCUMENT_PREFLIGHT_RESPONSE")]
    DocumentPreflightResponse(DocumentPreflightResponseMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct StartJobMessage {
    pub protocol_version: u32,
    pub task_id: String,
    pub input_pdf: String,
    pub output_dir: String,
    pub engine_version: String,
    #[serde(default)]
    pub pdf_options: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct CancelJobMessage {
    pub task_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TranslateResponseMessage {
    pub task_id: String,
    pub translation_request_id: String,
    pub outcome: TranslationResponseOutcome,
    #[serde(default)]
    pub translated_text: Option<String>,
    #[serde(default)]
    pub translated_segments: Vec<TranslatedSegment>,
    #[serde(default)]
    pub token_usage: Option<TokenUsage>,
    #[serde(default)]
    pub cache_hit: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub error: Option<ProtocolErrorPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DocumentPreflightResponseMessage {
    pub task_id: String,
    pub preflight_request_id: String,
    pub outcome: TranslationResponseOutcome,
    #[serde(default)]
    pub document_context: Value,
    #[serde(default)]
    pub context_hash: Option<String>,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub error: Option<ProtocolErrorPayload>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranslationResponseOutcome {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TranslatedSegment {
    pub segment_id: String,
    pub translated_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum WorkerToRustMessage {
    #[serde(rename = "JOB_STARTED")]
    JobStarted(JobStartedMessage),
    #[serde(rename = "STAGE_CHANGED")]
    StageChanged(StageChangedMessage),
    #[serde(rename = "PROGRESS")]
    Progress(ProgressMessage),
    #[serde(rename = "TRANSLATE_REQUEST")]
    TranslateRequest(TranslateRequestMessage),
    #[serde(rename = "DOCUMENT_PREFLIGHT_REQUEST")]
    DocumentPreflightRequest(DocumentPreflightRequestMessage),
    #[serde(rename = "TOKEN_USAGE")]
    TokenUsage(TokenUsageMessage),
    #[serde(rename = "WARNING")]
    Warning(WarningMessage),
    #[serde(rename = "FINISHED")]
    Finished(FinishedMessage),
    #[serde(rename = "CANCELLED")]
    Cancelled(CancelledMessage),
    #[serde(rename = "ERROR")]
    Error(ErrorMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct JobStartedMessage {
    pub protocol_version: u32,
    pub task_id: String,
    #[serde(default)]
    pub worker_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct StageChangedMessage {
    pub task_id: String,
    pub stage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ProgressMessage {
    pub task_id: String,
    pub stage: String,
    #[serde(default)]
    pub current: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub fraction: Option<f64>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TranslateRequestMessage {
    pub task_id: String,
    pub translation_request_id: String,
    pub mode: String,
    pub source_language: String,
    pub target_language: String,
    pub segments: Vec<TranslationSegment>,
    #[serde(default)]
    pub document_context: Value,
    #[serde(default)]
    pub context_before: Value,
    #[serde(default)]
    pub context_after: Value,
    #[serde(default)]
    pub task_terms: Value,
    #[serde(default)]
    pub abbreviations: Value,
    #[serde(default)]
    pub engine_constraints: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct DocumentPreflightRequestMessage {
    pub task_id: String,
    pub preflight_request_id: String,
    pub source_language: String,
    pub target_language: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub samples: Vec<TranslationSegment>,
    #[serde(default)]
    pub engine_constraints: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TranslationSegment {
    pub segment_id: String,
    pub source_text: String,
    #[serde(default)]
    pub placeholders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TokenUsageMessage {
    pub task_id: String,
    #[serde(default)]
    pub translation_request_id: Option<String>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TokenUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct WarningMessage {
    pub task_id: String,
    #[serde(default)]
    pub translation_request_id: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FinishedMessage {
    pub task_id: String,
    pub output_pdf: String,
    #[serde(default)]
    pub output_mode: Option<String>,
    #[serde(default)]
    pub page_count: Option<u32>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct CancelledMessage {
    pub task_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ErrorMessage {
    pub task_id: String,
    #[serde(default)]
    pub translation_request_id: Option<String>,
    pub error: ProtocolErrorPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ProtocolErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

pub fn encode_jsonl<T: Serialize>(message: &T) -> Result<String, ProtocolError> {
    let json = serde_json::to_string(message)
        .map_err(|error| ProtocolError::Serialization(error.to_string()))?;
    let byte_len = json.len() + 1;
    if byte_len > MAX_PROTOCOL_LINE_BYTES {
        return Err(ProtocolError::MessageTooLarge(byte_len));
    }
    let mut line = json;
    line.push('\n');
    Ok(line)
}

pub fn decode_jsonl<T: DeserializeOwned>(line: &str) -> Result<T, ProtocolError> {
    let byte_len = line.len();
    if byte_len > MAX_PROTOCOL_LINE_BYTES {
        return Err(ProtocolError::MessageTooLarge(byte_len));
    }

    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.trim().is_empty() {
        return Err(ProtocolError::EmptyLine);
    }
    if line
        .chars()
        .any(|character| character == '\r' || character == '\n')
    {
        return Err(ProtocolError::MultipleFrames);
    }

    serde_json::from_str(line).map_err(|error| ProtocolError::Deserialization(error.to_string()))
}

pub fn encode_rust_message(message: &RustToWorkerMessage) -> Result<String, ProtocolError> {
    encode_jsonl(message)
}

pub fn decode_rust_message(line: &str) -> Result<RustToWorkerMessage, ProtocolError> {
    let message = decode_jsonl(line)?;
    if let RustToWorkerMessage::StartJob(start) = &message
        && start.protocol_version != PDF_WORKER_PROTOCOL_VERSION
    {
        return Err(ProtocolError::UnsupportedProtocolVersion(
            start.protocol_version,
        ));
    }
    Ok(message)
}

pub fn encode_worker_message(message: &WorkerToRustMessage) -> Result<String, ProtocolError> {
    encode_jsonl(message)
}

pub fn decode_worker_message(line: &str) -> Result<WorkerToRustMessage, ProtocolError> {
    let message = decode_jsonl(line)?;
    if let WorkerToRustMessage::JobStarted(start) = &message
        && start.protocol_version != PDF_WORKER_PROTOCOL_VERSION
    {
        return Err(ProtocolError::UnsupportedProtocolVersion(
            start.protocol_version,
        ));
    }
    if let WorkerToRustMessage::TranslateRequest(request) = &message
        && TranslationMode::from_wire_mode(&request.mode).is_none()
    {
        return Err(ProtocolError::UnsupportedMode(request.mode.clone()));
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::{
        CancelJobMessage, DocumentPreflightRequestMessage, DocumentPreflightResponseMessage,
        FinishedMessage, JobStartedMessage, MAX_PROTOCOL_LINE_BYTES, PDF_WORKER_PROTOCOL_VERSION,
        ProtocolError, ProtocolErrorPayload, RustToWorkerMessage, StartJobMessage, TokenUsage,
        TokenUsageMessage, TranslateRequestMessage, TranslateResponseMessage, TranslatedSegment,
        TranslationResponseOutcome, TranslationSegment, WarningMessage, WorkerToRustMessage,
        decode_rust_message, decode_worker_message, encode_rust_message, encode_worker_message,
    };
    use serde_json::json;

    #[test]
    fn start_job_round_trips_as_one_jsonl_frame() {
        let message = RustToWorkerMessage::StartJob(StartJobMessage {
            protocol_version: PDF_WORKER_PROTOCOL_VERSION,
            task_id: "task-1".to_string(),
            input_pdf: "jobs/task-1/input.pdf".to_string(),
            output_dir: "jobs/task-1/output".to_string(),
            engine_version: "babeldoc-0.6.4".to_string(),
            pdf_options: json!({"output_mode": "bilingual"}),
        });

        let line = encode_rust_message(&message).expect("start job should serialize");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.contains("\"type\":\"START_JOB\""));

        let decoded = decode_rust_message(&line).expect("start job should deserialize");
        assert_eq!(decoded, message);
    }

    #[test]
    fn worker_translate_request_preserves_segment_order_and_ids() {
        let message = WorkerToRustMessage::TranslateRequest(TranslateRequestMessage {
            task_id: "task-1".to_string(),
            translation_request_id: "request-2".to_string(),
            mode: "pdf_segment".to_string(),
            source_language: "en".to_string(),
            target_language: "zh-CN".to_string(),
            segments: vec![
                TranslationSegment {
                    segment_id: "p2-s1".to_string(),
                    source_text: "second".to_string(),
                    placeholders: vec!["formula-1".to_string()],
                },
                TranslationSegment {
                    segment_id: "p1-s4".to_string(),
                    source_text: "first in the response order".to_string(),
                    placeholders: Vec::new(),
                },
            ],
            document_context: json!({"page": 2}),
            context_before: json!({}),
            context_after: json!({}),
            task_terms: json!([]),
            abbreviations: json!([]),
            engine_constraints: json!({"response_format": "json"}),
        });

        let line = encode_worker_message(&message).expect("translate request should serialize");
        let decoded = decode_worker_message(&line).expect("translate request should deserialize");
        assert_eq!(decoded, message);
        assert!(line.contains("\"translation_request_id\":\"request-2\""));
    }

    #[test]
    fn response_keeps_cache_and_segment_metadata() {
        let message = RustToWorkerMessage::TranslateResponse(TranslateResponseMessage {
            task_id: "task-1".to_string(),
            translation_request_id: "request-1".to_string(),
            outcome: TranslationResponseOutcome::Completed,
            translated_text: Some("译文".to_string()),
            translated_segments: vec![TranslatedSegment {
                segment_id: "p1-s1".to_string(),
                translated_text: "译文".to_string(),
            }],
            token_usage: Some(TokenUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(4),
                total_tokens: Some(14),
            }),
            cache_hit: true,
            warnings: vec!["使用缓存".to_string()],
            error: None,
        });

        let encoded = encode_rust_message(&message).expect("response should serialize");
        let decoded = decode_rust_message(&encoded).expect("response should deserialize");
        assert_eq!(decoded, message);
    }

    #[test]
    fn preflight_request_and_response_round_trip_with_context_fields() {
        let request =
            WorkerToRustMessage::DocumentPreflightRequest(DocumentPreflightRequestMessage {
                task_id: "task-1".to_string(),
                preflight_request_id: "preflight-1".to_string(),
                source_language: "en".to_string(),
                target_language: "zh-CN".to_string(),
                metadata: json!({"title": "A paper"}),
                samples: vec![TranslationSegment {
                    segment_id: "p1-s1".to_string(),
                    source_text: "A sample".to_string(),
                    placeholders: Vec::new(),
                }],
                engine_constraints: json!({"preserve_placeholders": true}),
            });
        let decoded_request = decode_worker_message(
            &encode_worker_message(&request).expect("preflight request should serialize"),
        )
        .expect("preflight request should deserialize");
        assert_eq!(decoded_request, request);

        let response =
            RustToWorkerMessage::DocumentPreflightResponse(DocumentPreflightResponseMessage {
                task_id: "task-1".to_string(),
                preflight_request_id: "preflight-1".to_string(),
                outcome: TranslationResponseOutcome::Completed,
                document_context: json!({"schema_version": 1, "title": "A paper"}),
                context_hash: Some("hash".to_string()),
                degraded: false,
                warnings: Vec::new(),
                error: None,
            });
        let decoded_response = decode_rust_message(
            &encode_rust_message(&response).expect("preflight response should serialize"),
        )
        .expect("preflight response should deserialize");
        assert_eq!(decoded_response, response);
    }

    #[test]
    fn optional_worker_fields_can_be_omitted() {
        let line = r#"{"type":"JOB_STARTED","protocol_version":1,"task_id":"task-1"}"#;
        let decoded = decode_worker_message(line).expect("optional fields should default");
        assert_eq!(
            decoded,
            WorkerToRustMessage::JobStarted(JobStartedMessage {
                protocol_version: 1,
                task_id: "task-1".to_string(),
                worker_version: None,
            })
        );
    }

    #[test]
    fn decoder_accepts_crlf_but_rejects_multiple_frames() {
        let line = format!(
            "{}\r\n",
            r#"{"type":"CANCEL_JOB","task_id":"task-1","reason":"user_requested"}"#
        );
        let decoded = decode_rust_message(&line).expect("CRLF should be accepted");
        assert_eq!(
            decoded,
            RustToWorkerMessage::CancelJob(CancelJobMessage {
                task_id: "task-1".to_string(),
                reason: "user_requested".to_string(),
            })
        );

        let multiple = format!(
            "{}\n{}",
            r#"{"type":"CANCEL_JOB","task_id":"task-1","reason":"one"}"#,
            r#"{"type":"CANCEL_JOB","task_id":"task-1","reason":"two"}"#
        );
        let error = decode_rust_message(&multiple).expect_err("multiple frames must be rejected");
        assert_eq!(error, ProtocolError::MultipleFrames);
    }

    #[test]
    fn decoder_rejects_empty_and_oversized_lines() {
        assert_eq!(
            decode_rust_message(" \t\n").expect_err("line is empty"),
            ProtocolError::EmptyLine
        );
        let oversized = "x".repeat(MAX_PROTOCOL_LINE_BYTES + 1);
        assert_eq!(
            decode_rust_message(&oversized).expect_err("line is oversized"),
            ProtocolError::MessageTooLarge(MAX_PROTOCOL_LINE_BYTES + 1)
        );
    }

    #[test]
    fn decoder_rejects_unknown_translation_modes() {
        let line = r#"{"type":"TRANSLATE_REQUEST","task_id":"task-1","translation_request_id":"request-1","mode":"future_mode","source_language":"en","target_language":"zh-CN","segments":[]}"#;
        assert_eq!(
            decode_worker_message(line).expect_err("unknown mode must be rejected"),
            ProtocolError::UnsupportedMode("future_mode".to_string())
        );
    }

    #[test]
    fn decoder_rejects_unknown_protocol_versions() {
        let line = r#"{"type":"JOB_STARTED","protocol_version":2,"task_id":"task-1"}"#;
        assert_eq!(
            decode_worker_message(line).expect_err("unknown protocol must be rejected"),
            ProtocolError::UnsupportedProtocolVersion(2)
        );
    }

    #[test]
    fn event_payloads_use_stable_snake_case_wire_fields() {
        let message = WorkerToRustMessage::Warning(WarningMessage {
            task_id: "task-1".to_string(),
            translation_request_id: Some("request-1".to_string()),
            code: "placeholder_mismatch".to_string(),
            message: "占位符不匹配".to_string(),
        });
        let line = encode_worker_message(&message).expect("warning should serialize");
        assert!(line.contains("\"translation_request_id\""));
        assert!(!line.contains("translationRequestId"));

        let finished = WorkerToRustMessage::Finished(FinishedMessage {
            task_id: "task-1".to_string(),
            output_pdf: "jobs/task-1/output/translated.pdf".to_string(),
            output_mode: Some("bilingual".to_string()),
            page_count: Some(3),
            warnings: Vec::new(),
        });
        assert!(
            encode_worker_message(&finished)
                .expect("finished should serialize")
                .contains("\"output_pdf\"")
        );
    }

    #[test]
    fn token_usage_and_error_payloads_round_trip() {
        let message = WorkerToRustMessage::TokenUsage(TokenUsageMessage {
            task_id: "task-1".to_string(),
            translation_request_id: Some("request-1".to_string()),
            usage: TokenUsage {
                prompt_tokens: Some(8),
                completion_tokens: Some(5),
                total_tokens: Some(13),
            },
        });
        let decoded = decode_worker_message(
            &encode_worker_message(&message).expect("token usage should serialize"),
        )
        .expect("token usage should deserialize");
        assert_eq!(decoded, message);

        let error = ProtocolErrorPayload {
            code: "worker_crashed".to_string(),
            message: "Worker 已退出".to_string(),
            retryable: false,
        };
        let encoded = serde_json::to_string(&error).expect("error payload should serialize");
        assert!(encoded.contains("\"retryable\":false"));
    }

    #[test]
    fn response_error_payload_is_available_without_transmitting_secrets() {
        let message = RustToWorkerMessage::TranslateResponse(TranslateResponseMessage {
            task_id: "task-1".to_string(),
            translation_request_id: "request-3".to_string(),
            outcome: TranslationResponseOutcome::Failed,
            translated_text: None,
            translated_segments: Vec::new(),
            token_usage: None,
            cache_hit: false,
            warnings: Vec::new(),
            error: Some(ProtocolErrorPayload {
                code: "provider_failed".to_string(),
                message: "Provider 请求失败".to_string(),
                retryable: true,
            }),
        });
        let line = encode_rust_message(&message).expect("failed response should serialize");
        assert!(!line.contains("api_key"));
        assert_eq!(
            decode_rust_message(&line).expect("failed response should deserialize"),
            message
        );
    }
}
