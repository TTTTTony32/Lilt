use crate::contracts::TranslationRequest;
use crate::diagnostics;
use crate::pdf_protocol::{
    PDF_WORKER_PROTOCOL_VERSION, ProtocolErrorPayload, RustToWorkerMessage,
    TranslateRequestMessage, TranslateResponseMessage, TranslatedSegment,
    TranslationResponseOutcome, WorkerToRustMessage,
};
use crate::pdf_worker::{WorkerSession, WorkerSessionEvent};
use crate::{AppState, PreparedTranslation, TranslationCore, TranslationMode, prepare_translation};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const BABELDOC_ENGINE_VERSION: &str = "babeldoc-0.6.4";
const WORKER_EXIT_GRACE_PERIOD: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub(crate) struct PdfJobHandle {
    pub(crate) session: Arc<WorkerSession>,
    pub(crate) cancellation: CancellationToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfTranslationStarted {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PdfTranslationFinishedEvent {
    task_id: String,
    output_pdf: String,
    #[serde(default)]
    output_mode: Option<String>,
    #[serde(default)]
    page_count: Option<u32>,
    #[serde(default)]
    warnings: Vec<String>,
}

#[tauri::command]
pub async fn start_pdf_translation(
    app: AppHandle,
    state: State<'_, AppState>,
    file_path: String,
    pdf_options: Value,
) -> Result<PdfTranslationStarted, String> {
    let source = validate_input_pdf(&file_path)?;
    let task_id = Uuid::new_v4().to_string();
    let job_dir = state.data_dir.join("pdf_jobs").join(&task_id);
    let input_pdf = job_dir.join("input.pdf");
    let output_dir = job_dir.join("output");
    fs::create_dir_all(&output_dir).map_err(|error| format!("创建 PDF 任务目录失败：{error}"))?;
    fs::copy(source, &input_pdf).map_err(|error| format!("复制 PDF 到任务目录失败：{error}"))?;

    let options = normalize_pdf_options(pdf_options)?;
    let command = build_worker_command()?;
    let session = Arc::new(
        WorkerSession::spawn(command, task_id.clone()).map_err(|error| error.to_string())?,
    );
    let cancellation = CancellationToken::new();
    let start_message = RustToWorkerMessage::StartJob(crate::pdf_protocol::StartJobMessage {
        protocol_version: PDF_WORKER_PROTOCOL_VERSION,
        task_id: task_id.clone(),
        input_pdf: input_pdf.to_string_lossy().into_owned(),
        output_dir: output_dir.to_string_lossy().into_owned(),
        engine_version: BABELDOC_ENGINE_VERSION.to_string(),
        pdf_options: options,
    });
    session
        .send(start_message)
        .map_err(|error| format!("启动 PDF Worker 失败：{error}"))?;

    let handle = PdfJobHandle {
        session: session.clone(),
        cancellation: cancellation.clone(),
    };
    state
        .pdf_jobs
        .lock()
        .map_err(|_| "PDF 任务状态锁已损坏".to_string())?
        .insert(task_id.clone(), handle);

    app.emit(
        "pdf_translation_started",
        PdfTranslationStarted {
            task_id: task_id.clone(),
        },
    )
    .map_err(|error| format!("发送 PDF 翻译开始事件失败：{error}"))?;

    let state = state.inner().clone();
    let app_for_worker = app.clone();
    let loop_task_id = task_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_job_loop(
            app_for_worker,
            state,
            loop_task_id,
            session,
            cancellation,
            job_dir,
        )
    });

    Ok(PdfTranslationStarted { task_id })
}

#[tauri::command]
pub fn cancel_pdf_translation(state: State<'_, AppState>, task_id: String) -> Result<bool, String> {
    let handle = state
        .pdf_jobs
        .lock()
        .map_err(|_| "PDF 任务状态锁已损坏".to_string())?
        .get(task_id.trim())
        .cloned();
    let Some(handle) = handle else {
        return Ok(false);
    };
    handle.cancellation.cancel();
    handle
        .session
        .cancel("user_requested")
        .map_err(|error| format!("发送 PDF 取消请求失败：{error}"))?;
    diagnostics::info(format!(
        "command.pdf_translation.cancel_requested task_id={}",
        task_id.trim()
    ));
    Ok(true)
}

fn run_job_loop(
    app: AppHandle,
    state: AppState,
    task_id: String,
    session: Arc<WorkerSession>,
    cancellation: CancellationToken,
    job_dir: PathBuf,
) {
    let mut keep_output = false;
    loop {
        match session.recv_timeout(Duration::from_millis(100)) {
            Ok(WorkerSessionEvent::Message(message)) => {
                if let Some(should_keep_output) = handle_worker_message(
                    &app,
                    &state,
                    &task_id,
                    &session,
                    &cancellation,
                    &job_dir,
                    message,
                ) {
                    keep_output = should_keep_output;
                    break;
                }
            }
            Ok(WorkerSessionEvent::ProtocolError(message)) => {
                emit_pdf_failed(&app, &task_id, "protocol_error", &message);
                break;
            }
            Ok(WorkerSessionEvent::WorkerExited(exit_code)) => {
                emit_pdf_failed(
                    &app,
                    &task_id,
                    "worker_exited",
                    &format!("PDF Worker 异常退出，退出码：{}", exit_code.unwrap_or(-1)),
                );
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                emit_pdf_failed(
                    &app,
                    &task_id,
                    "worker_channel_closed",
                    "PDF Worker 通道已关闭",
                );
                break;
            }
        }
    }

    cancellation.cancel();
    session.close_writer();
    wait_for_worker_exit(&session);
    if !keep_output {
        let _ = fs::remove_dir_all(&job_dir);
    }
    if let Ok(mut jobs) = state.pdf_jobs.lock() {
        jobs.remove(&task_id);
    }
}

fn handle_worker_message(
    app: &AppHandle,
    state: &AppState,
    task_id: &str,
    session: &Arc<WorkerSession>,
    cancellation: &CancellationToken,
    job_dir: &Path,
    message: WorkerToRustMessage,
) -> Option<bool> {
    match message {
        WorkerToRustMessage::JobStarted(started) => {
            let _ = app.emit("pdf_translation_stage", started);
        }
        WorkerToRustMessage::StageChanged(stage) => {
            let _ = app.emit("pdf_translation_stage", stage);
        }
        WorkerToRustMessage::Progress(progress) => {
            let _ = app.emit("pdf_translation_progress", progress);
        }
        WorkerToRustMessage::TranslateRequest(request) => {
            let state = state.clone();
            let session = session.clone();
            let cancellation = cancellation.child_token();
            tauri::async_runtime::spawn(async move {
                let response = translate_pdf_request(&state, request, cancellation).await;
                if let Err(error) = session.send(RustToWorkerMessage::TranslateResponse(response)) {
                    diagnostics::warn(format!(
                        "pdf.translation_response.send_failed error={error}"
                    ));
                }
            });
        }
        WorkerToRustMessage::TokenUsage(usage) => {
            let _ = app.emit("pdf_translation_token_usage", usage);
        }
        WorkerToRustMessage::Warning(warning) => {
            let _ = app.emit("pdf_translation_warning", warning);
        }
        WorkerToRustMessage::Finished(finished) => {
            if validate_output_pdf(&finished.output_pdf, job_dir).is_err() {
                emit_pdf_failed(
                    app,
                    task_id,
                    "invalid_output",
                    "PDF Worker 返回的输出文件无效",
                );
                return Some(false);
            } else {
                let event = PdfTranslationFinishedEvent {
                    task_id: finished.task_id,
                    output_pdf: finished.output_pdf,
                    output_mode: finished.output_mode,
                    page_count: finished.page_count,
                    warnings: finished.warnings,
                };
                let _ = app.emit("pdf_translation_finished", event);
                return Some(true);
            }
        }
        WorkerToRustMessage::Cancelled(cancelled) => {
            cancellation.cancel();
            let _ = app.emit("pdf_translation_cancelled", cancelled);
            return Some(false);
        }
        WorkerToRustMessage::Error(error) => {
            emit_pdf_failed(app, task_id, &error.error.code, &error.error.message);
            return Some(false);
        }
    }
    None
}

async fn translate_pdf_request(
    state: &AppState,
    request: TranslateRequestMessage,
    cancellation: CancellationToken,
) -> TranslateResponseMessage {
    let task_id = request.task_id.clone();
    let translation_request_id = request.translation_request_id.clone();
    let failed = |code: &str, message: String| TranslateResponseMessage {
        task_id: task_id.clone(),
        translation_request_id: translation_request_id.clone(),
        outcome: TranslationResponseOutcome::Failed,
        translated_text: None,
        translated_segments: Vec::new(),
        token_usage: None,
        cache_hit: false,
        warnings: Vec::new(),
        error: Some(ProtocolErrorPayload {
            code: code.to_string(),
            message,
            retryable: false,
        }),
    };

    if request.segments.is_empty() {
        return failed(
            "invalid_request",
            "TRANSLATE_REQUEST 至少需要一个段落".to_string(),
        );
    }
    let source_text = match request_source_text(&request) {
        Ok(value) => value,
        Err(error) => return failed("invalid_request", error),
    };
    let provider = match state.database.lock() {
        Ok(connection) => match crate::db::get_provider(&connection) {
            Ok(value) => value,
            Err(error) => return failed("provider_config", error),
        },
        Err(_) => return failed("state_lock", "应用数据库锁已损坏".to_string()),
    };
    let request_contract = TranslationRequest {
        request_id: translation_request_id.clone(),
        source_text: source_text.clone(),
        source_language: request.source_language.clone(),
        target_language: request.target_language.clone(),
        model_id: provider.model_id.clone(),
        prompt_id: provider.prompt_id.clone(),
    };
    let prepared = match prepare_translation(state, &request_contract, &source_text) {
        Ok(value) => value,
        Err(error) => return failed("translation_prepare_failed", error),
    };
    let api_key = match crate::secrets::load_api_key(&prepared.provider.id) {
        Ok(Some(value)) => value,
        Ok(None) => {
            return failed(
                "missing_api_key",
                "尚未配置 API Key，请在设置中保存 Provider".to_string(),
            );
        }
        Err(error) => return failed("api_key_failed", error),
    };
    let system_prompt = pdf_system_prompt(&prepared, &request.engine_constraints);
    let translated = TranslationCore::stream(
        crate::translation_core::StreamRequest {
            request_id: &translation_request_id,
            base_url: &prepared.provider.base_url,
            api_key: &api_key,
            model_id: &prepared.provider.model_id,
            system_prompt: &system_prompt,
            user_text: &source_text,
            cancel: &cancellation,
            mode: TranslationMode::PdfSegment,
            thinking_effort: &prepared.provider.thinking_effort,
        },
        |_| Ok(()),
    )
    .await;
    let translated = match translated {
        Ok(value) => value,
        Err(crate::provider::ProviderError::Cancelled) => {
            return TranslateResponseMessage {
                task_id,
                translation_request_id,
                outcome: TranslationResponseOutcome::Cancelled,
                translated_text: None,
                translated_segments: Vec::new(),
                token_usage: None,
                cache_hit: false,
                warnings: Vec::new(),
                error: None,
            };
        }
        Err(error) => return failed("provider_failed", error.to_string()),
    };
    match build_translation_response(&request, translated) {
        Ok((translated_text, translated_segments)) => TranslateResponseMessage {
            task_id,
            translation_request_id,
            outcome: TranslationResponseOutcome::Completed,
            translated_text,
            translated_segments,
            token_usage: None,
            cache_hit: false,
            warnings: Vec::new(),
            error: None,
        },
        Err(error) => failed("response_validation_failed", error),
    }
}

fn request_source_text(request: &TranslateRequestMessage) -> Result<String, String> {
    if request.segments.len() == 1 {
        return Ok(request.segments[0].source_text.clone());
    }
    let items = request
        .segments
        .iter()
        .map(|segment| {
            json!({
                "id": segment.segment_id,
                "input": segment.source_text,
                "placeholders": segment.placeholders,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&items).map_err(|error| format!("构造批量翻译请求失败：{error}"))
}

fn pdf_system_prompt(prepared: &PreparedTranslation, constraints: &Value) -> String {
    let constraints = serde_json::to_string(constraints).unwrap_or_else(|_| "{}".to_string());
    format!(
        "{}\n\nPDF 引擎约束：{}\n保留所有公式、富文本和占位符。批量请求必须返回与输入相同的 JSON 数组，每项保留 id 并在 output 中给出译文。",
        prepared.system_prompt, constraints
    )
}

fn build_translation_response(
    request: &TranslateRequestMessage,
    translated: String,
) -> Result<(Option<String>, Vec<TranslatedSegment>), String> {
    if request.segments.len() == 1 {
        validate_placeholders(&request.segments[0].placeholders, &translated)?;
        return Ok((
            Some(translated.clone()),
            vec![TranslatedSegment {
                segment_id: request.segments[0].segment_id.clone(),
                translated_text: translated,
            }],
        ));
    }
    let json_text = translated
        .trim()
        .strip_prefix("```json")
        .unwrap_or(translated.trim())
        .strip_suffix("```")
        .unwrap_or(translated.trim())
        .trim();
    let values: Vec<Value> = serde_json::from_str(json_text)
        .map_err(|error| format!("批量译文 JSON 解析失败：{error}"))?;
    let mut by_id = HashMap::new();
    for item in values {
        let id = item
            .get("id")
            .map(value_id)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "批量译文缺少段落 ID".to_string())?;
        let output = item
            .get("output")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("批量译文缺少 output：{id}"))?
            .to_string();
        if by_id.insert(id.clone(), output).is_some() {
            return Err(format!("批量译文包含重复段落 ID：{id}"));
        }
    }
    let mut segments = Vec::with_capacity(request.segments.len());
    for segment in &request.segments {
        let translated_text = by_id
            .remove(&segment.segment_id)
            .ok_or_else(|| format!("批量译文缺少段落：{}", segment.segment_id))?;
        validate_placeholders(&segment.placeholders, &translated_text)?;
        segments.push(TranslatedSegment {
            segment_id: segment.segment_id.clone(),
            translated_text,
        });
    }
    if !by_id.is_empty() {
        return Err("批量译文包含未知段落 ID".to_string());
    }
    Ok((None, segments))
}

fn validate_placeholders(placeholders: &[String], translated: &str) -> Result<(), String> {
    for placeholder in placeholders {
        if !translated.contains(placeholder) {
            return Err(format!("译文缺少占位符：{placeholder}"));
        }
    }
    Ok(())
}

fn value_id(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn wait_for_worker_exit(session: &WorkerSession) {
    let deadline = std::time::Instant::now() + WORKER_EXIT_GRACE_PERIOD;
    while std::time::Instant::now() < deadline {
        match session.recv_timeout(Duration::from_millis(50)) {
            Ok(WorkerSessionEvent::WorkerExited(_)) => return,
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
    let _ = session.terminate();
}

fn emit_pdf_failed(app: &AppHandle, task_id: &str, code: &str, message: &str) {
    let _ = app.emit(
        "pdf_translation_failed",
        json!({"taskId": task_id, "code": code, "message": message}),
    );
    diagnostics::error(format!(
        "command.pdf_translation.failed task_id={} code={} message={}",
        task_id, code, message
    ));
}

fn validate_input_pdf(file_path: &str) -> Result<&Path, String> {
    let path = Path::new(file_path.trim());
    if path.as_os_str().is_empty() {
        return Err("PDF 文件路径不能为空".to_string());
    }
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
    {
        return Err("PDF 文件扩展名必须为 .pdf".to_string());
    }
    if !path.is_file() {
        return Err("PDF 路径必须指向普通文件".to_string());
    }
    Ok(path)
}

fn validate_output_pdf(file_path: &str, job_dir: &Path) -> Result<(), String> {
    let path = Path::new(file_path);
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
    {
        return Err("PDF 输出文件扩展名无效".to_string());
    }
    let output = path
        .canonicalize()
        .map_err(|error| format!("PDF 输出文件不存在：{error}"))?;
    let root = job_dir
        .canonicalize()
        .map_err(|error| format!("PDF 任务目录不存在：{error}"))?;
    if !output.starts_with(&root) || !output.is_file() {
        return Err("PDF 输出文件不在当前任务目录内".to_string());
    }
    Ok(())
}

fn normalize_pdf_options(options: Value) -> Result<Value, String> {
    let mut object = options
        .as_object()
        .cloned()
        .ok_or_else(|| "PDF 选项必须是对象".to_string())?;
    object
        .entry("source_language".to_string())
        .or_insert_with(|| Value::String("en".to_string()));
    object
        .entry("target_language".to_string())
        .or_insert_with(|| Value::String("zh-CN".to_string()));
    Ok(Value::Object(object))
}

fn build_worker_command() -> Result<Command, String> {
    let worker_script = std::env::var_os("LILT_PDF_WORKER_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("python_worker")
                .join("worker.py")
        });
    if !worker_script.is_file() {
        return Err("PDF Worker 脚本不存在，请检查 PDF Engine 安装".to_string());
    }
    let mut command = if let Some(python) = std::env::var_os("LILT_PDF_PYTHON") {
        let mut command = Command::new(python);
        command.arg(worker_script);
        command
    } else {
        let uv = std::env::var_os("LILT_PDF_UV").unwrap_or_else(|| "uv".into());
        let mut command = Command::new(uv);
        command.args(["tool", "run", "--from", "BabelDOC==0.6.4", "python"]);
        command.arg(worker_script);
        command
    };
    command
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    if let Some(python_path) = std::env::var_os("LILT_PDF_PYTHONPATH") {
        command.env("PYTHONPATH", python_path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::{build_translation_response, normalize_pdf_options, request_source_text};
    use crate::pdf_protocol::{TranslateRequestMessage, TranslationSegment};
    use serde_json::json;

    fn request(segments: Vec<TranslationSegment>) -> TranslateRequestMessage {
        TranslateRequestMessage {
            task_id: "task-1".to_string(),
            translation_request_id: "request-1".to_string(),
            mode: "pdf_segment".to_string(),
            source_language: "en".to_string(),
            target_language: "zh-CN".to_string(),
            segments,
            document_context: json!({}),
            engine_constraints: json!({}),
        }
    }

    #[test]
    fn batch_source_text_keeps_segment_ids() {
        let source = request_source_text(&request(vec![
            TranslationSegment {
                segment_id: "p1-s1".to_string(),
                source_text: "one".to_string(),
                placeholders: Vec::new(),
            },
            TranslationSegment {
                segment_id: "p1-s2".to_string(),
                source_text: "two".to_string(),
                placeholders: Vec::new(),
            },
        ]))
        .expect("batch source should serialize");
        assert!(source.contains("p1-s1"));
        assert!(source.contains("p1-s2"));
    }

    #[test]
    fn batch_response_rejects_missing_placeholder() {
        let error = build_translation_response(
            &request(vec![TranslationSegment {
                segment_id: "p1-s1".to_string(),
                source_text: "hello".to_string(),
                placeholders: vec!["{v1}".to_string()],
            }]),
            "你好".to_string(),
        )
        .expect_err("placeholder must be preserved");
        assert!(error.contains("占位符"));
    }

    #[test]
    fn options_fill_default_languages_without_overwriting_explicit_values() {
        let options = normalize_pdf_options(json!({"target_language":"ja"}))
            .expect("options should be an object");
        assert_eq!(options["source_language"], "en");
        assert_eq!(options["target_language"], "ja");
    }
}
