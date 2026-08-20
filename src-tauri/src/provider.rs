use crate::{
    contracts::{ModelInfo, ThinkingEffort},
    diagnostics,
};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use url::Url;

const MODEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSLATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("Provider 地址无效：{0}")]
    InvalidConfig(String),
    #[error("Provider 认证失败，请检查 API Key")]
    Authentication,
    #[error("Provider 请求受到限流，请稍后重试")]
    RateLimited,
    #[error("Provider 暂时不可用：{0}")]
    Server(String),
    #[error("Provider 网络请求失败：{0}")]
    Network(String),
    #[error("Provider 请求超时：{0}")]
    Timeout(String),
    #[error("Provider 返回格式无法识别：{0}")]
    Protocol(String),
    #[error("翻译已取消")]
    Cancelled,
    #[error("桌面事件发送失败：{0}")]
    Event(String),
}

impl ProviderError {
    fn retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Server(_) | Self::Protocol(_))
    }
}

pub struct ChatStreamRequest<'a> {
    pub request_id: &'a str,
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub model_id: &'a str,
    pub system_prompt: &'a str,
    pub user_text: &'a str,
    pub cancel: &'a CancellationToken,
    pub operation: &'a str,
    pub thinking_effort: &'a ThinkingEffort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStreamResult {
    pub content: String,
    pub token_usage: Option<crate::pdf_protocol::TokenUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStreamActivity {
    Thinking,
    Content,
}

pub async fn fetch_models(base_url: &str, api_key: &str) -> Result<Vec<ModelInfo>, ProviderError> {
    let endpoint = endpoint(base_url, "models")?;
    let started_at = Instant::now();
    diagnostics::info(format!(
        "provider.fetch_models.start method=GET origin={} route=/models",
        safe_endpoint_origin(&endpoint)
    ));
    let response = client()
        .get(endpoint)
        .bearer_auth(api_key)
        .timeout(MODEL_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| request_error("模型列表", error))?;
    diagnostics::info(format!(
        "provider.fetch_models.response status={} elapsed_ms={}",
        response.status(),
        started_at.elapsed().as_millis()
    ));
    ensure_success(&response)?;
    let payload: Value = response
        .json()
        .await
        .map_err(|error| ProviderError::Protocol(error.to_string()))?;
    let models = parse_model_payload(&payload)?;
    diagnostics::info(format!(
        "provider.fetch_models.parsed model_count={} elapsed_ms={}",
        models.len(),
        started_at.elapsed().as_millis()
    ));
    Ok(models)
}

fn parse_model_payload(payload: &Value) -> Result<Vec<ModelInfo>, ProviderError> {
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Protocol("/models 响应缺少 data 数组".to_string()))?;
    let models = data
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| ModelInfo {
            id: id.to_string(),
            label: id.to_string(),
            source: "remote".to_string(),
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err(ProviderError::Protocol(
            "/models 响应没有可用模型".to_string(),
        ));
    }
    Ok(models)
}

pub async fn stream_chat_completion_with_usage<F>(
    request: ChatStreamRequest<'_>,
    on_delta: F,
) -> Result<ProviderStreamResult, ProviderError>
where
    F: FnMut(String) -> Result<(), ProviderError>,
{
    stream_chat_completion_with_usage_and_activity(request, on_delta, |_| {}).await
}

pub async fn stream_chat_completion_with_usage_and_activity<F, A>(
    request: ChatStreamRequest<'_>,
    mut on_delta: F,
    mut on_activity: A,
) -> Result<ProviderStreamResult, ProviderError>
where
    F: FnMut(String) -> Result<(), ProviderError>,
    A: FnMut(ProviderStreamActivity),
{
    let endpoint = endpoint(request.base_url, "chat/completions")?;
    let mut attempt = 0;
    loop {
        let mut started = false;
        diagnostics::info(format!(
            "provider.{}.start request_id={} attempt={} origin={} route=/chat/completions model={}",
            request.operation,
            request.request_id,
            attempt,
            safe_endpoint_origin(&endpoint),
            request.model_id
        ));
        match stream_once(
            &request,
            &endpoint,
            &mut started,
            &mut on_delta,
            &mut on_activity,
        )
        .await
        {
            Ok(result) => {
                diagnostics::info(format!(
                    "provider.{}.completed request_id={} attempt={} output_chars={}",
                    request.operation,
                    request.request_id,
                    attempt,
                    result.content.chars().count()
                ));
                return Ok(result);
            }
            Err(error) if !started && attempt == 0 && error.retryable() => {
                diagnostics::warn(format!(
                    "provider.{}.retry request_id={} reason={error}",
                    request.operation, request.request_id
                ));
                attempt += 1;
            }
            Err(error) => {
                diagnostics::error(format!(
                    "provider.{}.failed request_id={} started={} reason={error}",
                    request.operation, request.request_id, started
                ));
                return Err(error);
            }
        }
    }
}

async fn stream_once(
    request: &ChatStreamRequest<'_>,
    endpoint: &str,
    started: &mut bool,
    on_delta: &mut impl FnMut(String) -> Result<(), ProviderError>,
    on_activity: &mut impl FnMut(ProviderStreamActivity),
) -> Result<ProviderStreamResult, ProviderError> {
    if request.cancel.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let response = tokio::select! {
        _ = request.cancel.cancelled() => return Err(ProviderError::Cancelled),
        result = client()
            .post(endpoint)
            .bearer_auth(request.api_key)
            .header("Accept", "text/event-stream")
            .timeout(TRANSLATION_REQUEST_TIMEOUT)
            .json(&chat_completion_payload(request))
            .send() => result.map_err(|error| request_error("翻译", error))?,
    };
    diagnostics::info(format!(
        "provider.{}.response request_id={} status={}",
        request.operation,
        request.request_id,
        response.status()
    ));
    ensure_success(&response)?;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut translated = String::new();
    let mut token_usage = None;
    loop {
        let chunk = tokio::select! {
            _ = request.cancel.cancelled() => return Err(ProviderError::Cancelled),
            chunk = timeout(STREAM_IDLE_TIMEOUT, stream.next()) => {
                chunk.map_err(|_| ProviderError::Timeout("流式响应空闲".to_string()))?
            },
        };
        let Some(chunk) = chunk else { break };
        let bytes = chunk.map_err(|error| ProviderError::Network(error.to_string()))?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(position) = buffer.find('\n') {
            let line = buffer[..position].trim_end_matches('\r').to_string();
            buffer.drain(..=position);
            let parsed = parse_sse_details(&line)?;
            if let Some(usage) = parsed.token_usage {
                token_usage = Some(usage);
            }
            for activity in parsed.activities {
                *started = true;
                on_activity(activity);
            }
            if let Some(content) = parsed.content {
                if content.is_empty() {
                    continue;
                }
                *started = true;
                on_activity(ProviderStreamActivity::Content);
                translated.push_str(&content);
                on_delta(content)?;
            }
        }
    }
    if !buffer.trim().is_empty() {
        let parsed = parse_sse_details(buffer.trim())?;
        if let Some(usage) = parsed.token_usage {
            token_usage = Some(usage);
        }
        for activity in parsed.activities {
            *started = true;
            on_activity(activity);
        }
        if let Some(content) = parsed.content {
            if !content.is_empty() {
                *started = true;
                on_activity(ProviderStreamActivity::Content);
                translated.push_str(&content);
                on_delta(content)?;
            }
        }
    }
    if translated.is_empty() {
        return Err(ProviderError::Protocol("流式响应没有返回译文".to_string()));
    }
    Ok(ProviderStreamResult {
        content: translated,
        token_usage,
    })
}

fn chat_completion_payload(request: &ChatStreamRequest<'_>) -> Value {
    json!({
        "model": request.model_id,
        "stream": true,
        "stream_options": { "include_usage": true },
        "reasoning_effort": request.thinking_effort.as_str(),
        "messages": [
            { "role": "system", "content": request.system_prompt },
            { "role": "user", "content": request.user_text }
        ]
    })
}

#[cfg(test)]
fn parse_sse_line(line: &str) -> Result<Option<String>, ProviderError> {
    Ok(parse_sse_details(line)?.content)
}

struct ParsedSseLine {
    content: Option<String>,
    token_usage: Option<crate::pdf_protocol::TokenUsage>,
    activities: Vec<ProviderStreamActivity>,
}

fn parse_sse_details(line: &str) -> Result<ParsedSseLine, ProviderError> {
    let Some(payload) = line.strip_prefix("data:") else {
        return Ok(ParsedSseLine {
            content: None,
            token_usage: None,
            activities: Vec::new(),
        });
    };
    let payload = payload.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return Ok(ParsedSseLine {
            content: None,
            token_usage: None,
            activities: Vec::new(),
        });
    }
    let value: Value = serde_json::from_str(payload)
        .map_err(|error| ProviderError::Protocol(format!("JSON 解析失败：{error}")))?;
    let token_usage = value.get("usage").and_then(parse_token_usage);
    let delta = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta").or_else(|| choice.get("message")));

    let Some(delta) = delta else {
        return Ok(ParsedSseLine {
            content: None,
            token_usage,
            activities: Vec::new(),
        });
    };

    let mut activities = Vec::new();
    for field in ["reasoning_content", "reasoning", "thinking"] {
        if delta.get(field).and_then(non_empty_stream_text).is_some()
            && !activities.contains(&ProviderStreamActivity::Thinking)
        {
            activities.push(ProviderStreamActivity::Thinking);
        }
    }
    let content = delta.get("content").and_then(stream_text);
    Ok(ParsedSseLine {
        content,
        token_usage,
        activities,
    })
}

fn stream_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(|part| {
                    part.as_str()
                        .or_else(|| part.get("text").and_then(Value::as_str))
                        .or_else(|| part.get("content").and_then(Value::as_str))
                })
                .collect::<String>();
            Some(text)
        }
        Value::Object(object) => object
            .get("text")
            .or_else(|| object.get("content"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

fn non_empty_stream_text(value: &Value) -> Option<String> {
    stream_text(value).filter(|text| !text.is_empty())
}

fn parse_token_usage(value: &Value) -> Option<crate::pdf_protocol::TokenUsage> {
    Some(crate::pdf_protocol::TokenUsage {
        prompt_tokens: value.get("prompt_tokens").and_then(Value::as_u64),
        completion_tokens: value.get("completion_tokens").and_then(Value::as_u64),
        total_tokens: value.get("total_tokens").and_then(Value::as_u64),
    })
}

fn client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .expect("reqwest client configuration is static")
}

fn request_error(operation: &str, error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout(format!("{operation}请求"))
    } else {
        ProviderError::Network(format!("{operation}请求：{error}"))
    }
}

fn endpoint(base_url: &str, path: &str) -> Result<String, ProviderError> {
    let normalized = normalize_base_url(base_url)?;
    Ok(format!("{normalized}/{path}"))
}

pub fn safe_endpoint_origin(endpoint: &str) -> String {
    let Ok(parsed) = Url::parse(endpoint) else {
        return "<invalid>".to_string();
    };
    let host = parsed.host_str().unwrap_or("<unknown>");
    let port = parsed
        .port()
        .map(|value| format!(":{value}"))
        .unwrap_or_default();
    format!("{}://{host}{port}", parsed.scheme())
}

pub fn normalize_base_url(base_url: &str) -> Result<String, ProviderError> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(ProviderError::InvalidConfig(
            "Base URL 不能为空".to_string(),
        ));
    }
    let parsed =
        Url::parse(trimmed).map_err(|error| ProviderError::InvalidConfig(error.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ProviderError::InvalidConfig(
            "Base URL 必须使用 http 或 https".to_string(),
        ));
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ProviderError::InvalidConfig(
            "Base URL 不应包含账号、密码、查询参数或片段".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn ensure_success(response: &reqwest::Response) -> Result<(), ProviderError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Authentication),
        StatusCode::TOO_MANY_REQUESTS => Err(ProviderError::RateLimited),
        status if status.is_server_error() => Err(ProviderError::Server(status.to_string())),
        status if !status.is_success() => Err(ProviderError::Protocol(format!("HTTP {status}"))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatStreamRequest, ProviderError, ProviderStreamActivity, normalize_base_url,
        parse_model_payload, parse_sse_details, parse_sse_line, stream_chat_completion_with_usage,
        stream_chat_completion_with_usage_and_activity,
    };
    use crate::contracts::ThinkingEffort;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn normalizes_provider_base_url() {
        assert_eq!(
            normalize_base_url(" https://example.com/v1/// ").unwrap(),
            "https://example.com/v1"
        );
    }

    #[test]
    fn rejects_non_http_provider_url() {
        assert!(matches!(
            normalize_base_url("file:///tmp/provider"),
            Err(ProviderError::InvalidConfig(_))
        ));
    }

    #[test]
    fn rejects_provider_url_with_query_parameters() {
        assert!(matches!(
            normalize_base_url("https://example.com/v1?api_key=secret"),
            Err(ProviderError::InvalidConfig(_))
        ));
    }

    #[test]
    fn endpoint_logs_only_origin_without_path_or_query() {
        assert_eq!(
            super::safe_endpoint_origin("https://example.com/private-token/v1/models"),
            "https://example.com"
        );
    }

    #[test]
    fn parses_non_empty_model_ids_only() {
        let models = parse_model_payload(&json!({
            "data": [{"id": " model-a "}, {"id": ""}, {"name": "missing-id"}]
        }))
        .unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "model-a");
    }

    #[test]
    fn rejects_model_payload_without_data_array() {
        assert!(matches!(
            parse_model_payload(&json!({"models": []})),
            Err(ProviderError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn fetches_models_from_a_mock_provider() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let bytes_read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..bytes_read]).to_lowercase();
            assert!(request.starts_with("get /v1/models"));
            assert!(request.contains("authorization: bearer test-key"));
            let body = r#"{"data":[{"id":"mock-model"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let models = super::fetch_models(&format!("http://{address}/v1"), "test-key")
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(models[0].id, "mock-model");
    }

    #[tokio::test]
    async fn streams_translation_from_an_openai_compatible_stub() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let bytes_read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..bytes_read]).to_lowercase();
            assert!(request.starts_with("post /v1/chat/completions"));
            assert!(request.contains("authorization: bearer test-key"));
            assert!(request.contains("\"model\":\"stub-model\""));
            assert!(request.contains("\"reasoning_effort\":\"none\""));

            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"本地\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\" Provider\"}}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":4,\"total_tokens\":14}}\n\n",
                "data: [DONE]\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let cancellation = tokio_util::sync::CancellationToken::new();
        let mut deltas = Vec::new();
        let result = stream_chat_completion_with_usage(
            ChatStreamRequest {
                request_id: "request-1",
                base_url: &format!("http://{address}/v1"),
                api_key: "test-key",
                model_id: "stub-model",
                system_prompt: "system",
                user_text: "hello",
                cancel: &cancellation,
                operation: "pdf_segment",
                thinking_effort: &ThinkingEffort::None,
            },
            |delta| {
                deltas.push(delta);
                Ok(())
            },
        )
        .await
        .unwrap();

        server.join().unwrap();
        assert_eq!(result.content, "本地 Provider");
        assert_eq!(
            result.token_usage,
            Some(crate::pdf_protocol::TokenUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(4),
                total_tokens: Some(14),
            })
        );
        assert_eq!(deltas, vec!["本地", " Provider"]);
    }

    #[tokio::test]
    async fn reports_reasoning_and_content_activity_without_changing_deltas() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).unwrap();
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先分析\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"结果\"}}]}\n\n",
                "data: [DONE]\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let cancellation = tokio_util::sync::CancellationToken::new();
        let mut activities = Vec::new();
        let mut deltas = Vec::new();
        let result = stream_chat_completion_with_usage_and_activity(
            ChatStreamRequest {
                request_id: "preflight-1",
                base_url: &format!("http://{address}/v1"),
                api_key: "test-key",
                model_id: "stub-model",
                system_prompt: "system",
                user_text: "hello",
                cancel: &cancellation,
                operation: "pdf_preflight",
                thinking_effort: &ThinkingEffort::None,
            },
            |delta| {
                deltas.push(delta);
                Ok(())
            },
            |activity| activities.push(activity),
        )
        .await
        .unwrap();

        server.join().unwrap();
        assert_eq!(result.content, "结果");
        assert_eq!(deltas, vec!["结果"]);
        assert_eq!(
            activities,
            vec![
                ProviderStreamActivity::Thinking,
                ProviderStreamActivity::Content
            ]
        );
    }

    #[test]
    fn parses_openai_stream_delta() {
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"content":"你好"}}]}"#).unwrap(),
            Some("你好".to_string())
        );
    }

    #[test]
    fn parses_done_marker_as_no_content() {
        assert_eq!(parse_sse_line("data: [DONE]").unwrap(), None);
    }

    #[test]
    fn parses_usage_frame_without_content() {
        let frame = parse_sse_details(
            r#"data: {"choices":[],"usage":{"prompt_tokens":8,"completion_tokens":3,"total_tokens":11}}"#,
        )
        .unwrap();
        assert!(frame.content.is_none());
        assert_eq!(
            frame.token_usage,
            Some(crate::pdf_protocol::TokenUsage {
                prompt_tokens: Some(8),
                completion_tokens: Some(3),
                total_tokens: Some(11),
            })
        );
    }

    #[test]
    fn parses_reasoning_fields_as_thinking_activity_without_leaking_text() {
        let frame =
            parse_sse_details(r#"data: {"choices":[{"delta":{"reasoning_content":"先分析"}}]}"#)
                .unwrap();
        assert!(frame.content.is_none());
        assert_eq!(frame.activities, vec![ProviderStreamActivity::Thinking]);
    }

    #[test]
    fn parses_reasoning_and_content_activities_in_one_frame() {
        let frame = parse_sse_details(
            r#"data: {"choices":[{"delta":{"thinking":"分析","content":"结果"}}]}"#,
        )
        .unwrap();
        assert_eq!(frame.content.as_deref(), Some("结果"));
        assert_eq!(frame.activities, vec![ProviderStreamActivity::Thinking]);
    }

    #[test]
    fn rejects_invalid_sse_json() {
        assert!(matches!(
            parse_sse_details("data: {invalid-json}"),
            Err(ProviderError::Protocol(_))
        ));
    }

    #[test]
    fn chat_completion_request_serializes_every_reasoning_effort() {
        let cancellation = tokio_util::sync::CancellationToken::new();
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
        ] {
            let request = super::ChatStreamRequest {
                request_id: "request",
                base_url: "https://example.com/v1",
                api_key: "key",
                model_id: "model",
                system_prompt: "system",
                user_text: "user",
                cancel: &cancellation,
                operation: "test",
                thinking_effort: &effort,
            };
            let payload = super::chat_completion_payload(&request);
            assert_eq!(payload["reasoning_effort"], effort.as_str());
        }
    }
}
