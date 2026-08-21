use crate::contracts::TranslationRequest;
use crate::diagnostics;
use crate::pdf_protocol::{
    DocumentPreflightAcceptedMessage, DocumentPreflightActivityMessage,
    DocumentPreflightActivityPhase, DocumentPreflightRequestMessage,
    DocumentPreflightResponseMessage, DocumentPreflightTimeoutMessage, PDF_WORKER_PROTOCOL_VERSION,
    ProtocolErrorPayload, RustToWorkerMessage, TranslateRequestMessage, TranslateResponseMessage,
    TranslatedSegment, TranslationResponseOutcome, WorkerToRustMessage,
};
use crate::pdf_worker::{WorkerSession, WorkerSessionEvent};
use crate::{
    AppState, PdfPromptContext, PreparedTranslation, TranslationCore, TranslationMode,
    prepare_pdf_translation,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const WORKER_EXIT_GRACE_PERIOD: Duration = Duration::from_secs(3);

type PreflightActivityHandler =
    Box<dyn FnMut(crate::provider::ProviderStreamActivity) + Send + 'static>;

#[derive(Clone)]
pub(crate) struct PdfJobHandle {
    pub(crate) session: Arc<WorkerSession>,
    pub(crate) cancellation: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PreflightKey {
    task_id: String,
    preflight_request_id: String,
}

struct PreflightEntry {
    cancellation: CancellationToken,
    terminal: bool,
    completion: Option<PreflightCompletion>,
}

struct PreflightCompletion {
    event_name: &'static str,
    event: PdfPreflightCompletedEvent,
}

#[derive(Clone, Default)]
struct PreflightRegistry {
    entries: Arc<Mutex<HashMap<PreflightKey, PreflightEntry>>>,
}

impl PreflightRegistry {
    fn register(&self, key: PreflightKey, cancellation: CancellationToken) -> bool {
        let Ok(mut entries) = self.entries.lock() else {
            return false;
        };
        if entries.contains_key(&key) {
            return false;
        }
        entries.insert(
            key,
            PreflightEntry {
                cancellation,
                terminal: false,
                completion: None,
            },
        );
        true
    }

    fn invalidate(&self, key: &PreflightKey) -> bool {
        let Ok(mut entries) = self.entries.lock() else {
            return false;
        };
        let Some(entry) = entries.get_mut(key) else {
            return false;
        };
        if entry.terminal {
            return false;
        }
        entry.terminal = true;
        entry.cancellation.cancel();
        true
    }

    fn store_completion(
        &self,
        key: &PreflightKey,
        event_name: &'static str,
        event: PdfPreflightCompletedEvent,
    ) -> bool {
        let Ok(mut entries) = self.entries.lock() else {
            return false;
        };
        let Some(entry) = entries.get_mut(key) else {
            return false;
        };
        if entry.terminal || entry.completion.is_some() {
            return false;
        }
        entry.completion = Some(PreflightCompletion { event_name, event });
        true
    }

    fn accept_completion(&self, key: &PreflightKey) -> Option<PreflightCompletion> {
        let Ok(mut entries) = self.entries.lock() else {
            return None;
        };
        let entry = entries.get_mut(key)?;
        if entry.terminal {
            return None;
        }
        let completion = entry.completion.take()?;
        entry.terminal = true;
        Some(completion)
    }

    fn invalidate_all(&self) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        for entry in entries.values_mut() {
            if !entry.terminal {
                entry.terminal = true;
                entry.cancellation.cancel();
            }
        }
    }
}

#[derive(Debug, Clone)]
struct PdfTranslationPersistence {
    cache_key: String,
    source_text: String,
    translated_text: String,
    source_language: String,
    target_language: String,
    provider: crate::contracts::ProviderRecord,
    prompt_id: String,
    glossary_version: i64,
    cache_enabled: bool,
    cache_max_bytes: i64,
    history_retention: i64,
    cache_hit: bool,
    index_examples: bool,
}

struct PdfTranslationRequestResult {
    response: TranslateResponseMessage,
    persistence: Option<PdfTranslationPersistence>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PdfPreflightCompletedEvent {
    task_id: String,
    preflight_request_id: String,
    context: Value,
    context_hash: Option<String>,
    warnings: Vec<String>,
    degraded: bool,
    applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
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
    let _engine_transition = state
        .pdf_engine_transition
        .lock()
        .map_err(|_| "PDF Engine 切换状态锁已损坏".to_string())?;
    if state
        .pdf_engine_preparing
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("PDF Engine 正在准备或更新，请稍候".to_string());
    }
    let source = validate_input_pdf(&file_path)?;
    let options = normalize_pdf_options(pdf_options)?;
    let command = crate::pdf_engine::build_worker_command(&state.data_dir)?;
    let task_id = Uuid::new_v4().to_string();
    let job_dir = state.data_dir.join("pdf_jobs").join(&task_id);
    let input_pdf = job_dir.join("input.pdf");
    let output_dir = job_dir.join("output");
    fs::create_dir_all(&output_dir).map_err(|error| format!("创建 PDF 任务目录失败：{error}"))?;
    if let Err(error) = fs::copy(source, &input_pdf) {
        let _ = fs::remove_dir_all(&job_dir);
        return Err(format!("复制 PDF 到任务目录失败：{error}"));
    }

    let session = match WorkerSession::spawn(command, task_id.clone()) {
        Ok(session) => Arc::new(session),
        Err(error) => {
            let _ = fs::remove_dir_all(&job_dir);
            return Err(error.to_string());
        }
    };
    let cancellation = CancellationToken::new();
    let persistence = Arc::new(Mutex::new(Vec::new()));
    let start_message = RustToWorkerMessage::StartJob(crate::pdf_protocol::StartJobMessage {
        protocol_version: PDF_WORKER_PROTOCOL_VERSION,
        task_id: task_id.clone(),
        input_pdf: input_pdf.to_string_lossy().into_owned(),
        output_dir: output_dir.to_string_lossy().into_owned(),
        engine_version: crate::pdf_engine::BABELDOC_ENGINE_VERSION.to_string(),
        pdf_options: options,
    });
    session.send(start_message).map_err(|error| {
        let _ = fs::remove_dir_all(&job_dir);
        format!("启动 PDF Worker 失败：{error}")
    })?;

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
            persistence,
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
    persistence: Arc<Mutex<Vec<PdfTranslationPersistence>>>,
) {
    let mut keep_output = false;
    let preflight_registry = PreflightRegistry::default();
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
                    &persistence,
                    &preflight_registry,
                    *message,
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
    preflight_registry.invalidate_all();
    session.close_writer();
    wait_for_worker_exit(&session);
    if !keep_output {
        let _ = fs::remove_dir_all(&job_dir);
    }
    if let Ok(mut jobs) = state.pdf_jobs.lock() {
        jobs.remove(&task_id);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_worker_message(
    app: &AppHandle,
    state: &AppState,
    task_id: &str,
    session: &Arc<WorkerSession>,
    cancellation: &CancellationToken,
    job_dir: &Path,
    persistence: &Arc<Mutex<Vec<PdfTranslationPersistence>>>,
    preflight_registry: &PreflightRegistry,
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
        WorkerToRustMessage::DocumentPreflightRequest(request) => {
            if request.task_id != task_id {
                diagnostics::warn(format!(
                    "pdf.preflight.request_ignored task_id={} request_task_id={} preflight_request_id={} reason=task_mismatch",
                    task_id, request.task_id, request.preflight_request_id
                ));
                return None;
            }
            let request_id = request.preflight_request_id.clone();
            let key = PreflightKey {
                task_id: request.task_id.clone(),
                preflight_request_id: request_id.clone(),
            };
            let cancellation = cancellation.child_token();
            if !preflight_registry.register(key.clone(), cancellation.clone()) {
                emit_preflight_diagnostic(
                    app,
                    &key.task_id,
                    &key.preflight_request_id,
                    "preflight_duplicate_request",
                    "重复的 PDF 预检请求已忽略",
                );
                return None;
            }
            let _ = app.emit(
                "pdf_translation_preflight_started",
                json!({
                    "taskId": request.task_id,
                    "preflightRequestId": request_id,
                }),
            );
            let state = state.clone();
            let session = session.clone();
            let app = app.clone();
            let registry = preflight_registry.clone();
            let activity_task_id = task_id.to_string();
            let activity_request_id = request.preflight_request_id.clone();
            let activity_session = session.clone();
            let activity_app = app.clone();
            let mut last_phase = None;
            let activity_handler: PreflightActivityHandler = Box::new(move |activity| {
                let phase = match activity {
                    crate::provider::ProviderStreamActivity::Thinking => {
                        DocumentPreflightActivityPhase::Thinking
                    }
                    crate::provider::ProviderStreamActivity::Content => {
                        DocumentPreflightActivityPhase::Streaming
                    }
                };
                if last_phase == Some(phase) {
                    return;
                }
                last_phase = Some(phase);
                let message = RustToWorkerMessage::DocumentPreflightActivity(
                    DocumentPreflightActivityMessage {
                        task_id: activity_task_id.clone(),
                        preflight_request_id: activity_request_id.clone(),
                        phase,
                    },
                );
                if let Err(error) = activity_session.send(message) {
                    diagnostics::warn(format!(
                        "pdf.preflight_activity.send_failed task_id={} preflight_request_id={} phase={:?} error={error}",
                        activity_task_id, activity_request_id, phase
                    ));
                }
                let phase_name = match phase {
                    DocumentPreflightActivityPhase::Thinking => "thinking",
                    DocumentPreflightActivityPhase::Streaming => "streaming",
                };
                let _ = activity_app.emit(
                    "pdf_translation_preflight_activity",
                    json!({
                        "taskId": activity_task_id,
                        "preflightRequestId": activity_request_id,
                        "phase": phase_name,
                    }),
                );
            });
            tauri::async_runtime::spawn(async move {
                let result = translate_pdf_preflight_for_job(
                    &state,
                    request,
                    cancellation,
                    Some(activity_handler),
                )
                .await;
                let result_key = PreflightKey {
                    task_id: result.task_id.clone(),
                    preflight_request_id: result.preflight_request_id.clone(),
                };
                let event = PdfPreflightCompletedEvent {
                    task_id: result.task_id.clone(),
                    preflight_request_id: result.preflight_request_id.clone(),
                    context: result.document_context.clone(),
                    context_hash: result.context_hash.clone(),
                    warnings: result.warnings.clone(),
                    degraded: result.degraded,
                    applied: !result.degraded
                        && result.outcome == TranslationResponseOutcome::Completed,
                    message: result.warnings.first().cloned(),
                };
                let event_name = match result.outcome {
                    TranslationResponseOutcome::Completed if result.degraded => {
                        "pdf_translation_preflight_degraded"
                    }
                    TranslationResponseOutcome::Completed => "pdf_translation_preflight_completed",
                    TranslationResponseOutcome::Cancelled | TranslationResponseOutcome::Failed => {
                        "pdf_translation_preflight_failed"
                    }
                };
                if !registry.store_completion(&result_key, event_name, event) {
                    emit_preflight_diagnostic(
                        &app,
                        &result_key.task_id,
                        &result_key.preflight_request_id,
                        "preflight_late_response",
                        "已失效的 PDF 预检响应被忽略",
                    );
                    return;
                }
                if let Err(error) =
                    session.send(RustToWorkerMessage::DocumentPreflightResponse(result))
                {
                    registry.invalidate(&result_key);
                    diagnostics::warn(format!("pdf.preflight_response.send_failed error={error}"));
                }
            });
        }
        WorkerToRustMessage::DocumentPreflightTimeout(timeout) => {
            handle_preflight_timeout(app, task_id, preflight_registry, timeout);
        }
        WorkerToRustMessage::DocumentPreflightAccepted(accepted) => {
            handle_preflight_accepted(app, task_id, preflight_registry, accepted);
        }
        WorkerToRustMessage::TranslateRequest(request) => {
            let state = state.clone();
            let session = session.clone();
            let app = app.clone();
            let cancellation = cancellation.child_token();
            let persistence = persistence.clone();
            tauri::async_runtime::spawn(async move {
                let result = translate_pdf_request_for_job(&state, request, cancellation).await;
                if let Some(record) = result.persistence
                    && let Ok(mut records) = persistence.lock()
                {
                    records.push(record);
                }
                for warning in &result.response.warnings {
                    let _ = app.emit(
                        "pdf_translation_quality_diagnostic",
                        json!({
                            "taskId": result.response.task_id,
                            "translationRequestId": result.response.translation_request_id,
                            "severity": "warning",
                            "ruleId": "translation_quality",
                            "message": warning,
                        }),
                    );
                }
                if let Err(error) =
                    session.send(RustToWorkerMessage::TranslateResponse(result.response))
                {
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
            if let Some(preflight_request_id) = warning.preflight_request_id.clone() {
                let warning_message = warning.message;
                let _ = app.emit(
                    "pdf_translation_preflight_warning",
                    json!({
                        "taskId": warning.task_id,
                        "preflightRequestId": preflight_request_id,
                        "warnings": [warning_message.clone()],
                        "degraded": true,
                        "applied": false,
                        "message": warning_message,
                    }),
                );
            } else {
                let _ = app.emit("pdf_translation_warning", warning);
            }
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
                let records = match persistence.lock() {
                    Ok(records) => records.clone(),
                    Err(_) => {
                        emit_pdf_failed(
                            app,
                            task_id,
                            "persistence_state_failed",
                            "PDF 翻译提交状态锁已损坏",
                        );
                        return Some(false);
                    }
                };
                if let Err(error) = commit_pdf_persistence(state, &records) {
                    emit_pdf_failed(app, task_id, "persistence_failed", &error);
                    return Some(false);
                }
                for record in &records {
                    if record.cache_enabled && record.index_examples {
                        crate::schedule_example_index(state, &record.cache_key);
                    }
                }
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

fn handle_preflight_timeout(
    app: &AppHandle,
    task_id: &str,
    registry: &PreflightRegistry,
    timeout: DocumentPreflightTimeoutMessage,
) {
    if timeout.task_id != task_id {
        diagnostics::warn(format!(
            "pdf.preflight.timeout_ignored task_id={} timeout_task_id={} preflight_request_id={} reason=task_mismatch",
            task_id, timeout.task_id, timeout.preflight_request_id
        ));
        return;
    }
    if timeout.reason != "no_response" {
        emit_preflight_diagnostic(
            app,
            &timeout.task_id,
            &timeout.preflight_request_id,
            "preflight_invalid_timeout_reason",
            "PDF 预检超时原因无法识别，已忽略通知",
        );
        return;
    }
    let key = PreflightKey {
        task_id: timeout.task_id.clone(),
        preflight_request_id: timeout.preflight_request_id.clone(),
    };
    if !registry.invalidate(&key) {
        emit_preflight_diagnostic(
            app,
            &key.task_id,
            &key.preflight_request_id,
            "preflight_late_timeout",
            "已结束的 PDF 预检超时通知被忽略",
        );
        return;
    }

    let context = crate::pdf_context::DocumentContext::empty();
    let warning = "PDF 预检在模型产生响应前超时，已使用空文档上下文继续翻译";
    let _ = app.emit(
        "pdf_translation_preflight_degraded",
        PdfPreflightCompletedEvent {
            task_id: key.task_id.clone(),
            preflight_request_id: key.preflight_request_id.clone(),
            context: context.to_value(),
            context_hash: Some(context.hash().to_string()),
            warnings: vec![warning.to_string()],
            degraded: true,
            applied: false,
            message: Some(warning.to_string()),
        },
    );
    emit_preflight_diagnostic(
        app,
        &key.task_id,
        &key.preflight_request_id,
        "preflight_timeout",
        "PDF 预检无响应超时，已取消对应 Provider 请求",
    );
}

fn handle_preflight_accepted(
    app: &AppHandle,
    task_id: &str,
    registry: &PreflightRegistry,
    accepted: DocumentPreflightAcceptedMessage,
) {
    if accepted.task_id != task_id {
        diagnostics::warn(format!(
            "pdf.preflight.accepted_ignored task_id={} accepted_task_id={} preflight_request_id={} reason=task_mismatch",
            task_id, accepted.task_id, accepted.preflight_request_id
        ));
        return;
    }
    let key = PreflightKey {
        task_id: accepted.task_id.clone(),
        preflight_request_id: accepted.preflight_request_id.clone(),
    };
    let Some(completion) = registry.accept_completion(&key) else {
        emit_preflight_diagnostic(
            app,
            &key.task_id,
            &key.preflight_request_id,
            "preflight_late_acceptance",
            "已结束的 PDF 预检响应确认被忽略",
        );
        return;
    };
    let _ = app.emit(completion.event_name, completion.event);
}

fn emit_preflight_diagnostic(
    app: &AppHandle,
    task_id: &str,
    preflight_request_id: &str,
    rule_id: &str,
    message: &str,
) {
    diagnostics::warn(format!(
        "pdf.preflight.diagnostic task_id={} preflight_request_id={} rule_id={} reason={}",
        task_id, preflight_request_id, rule_id, message
    ));
    let _ = app.emit(
        "pdf_translation_diagnostic",
        json!({
            "taskId": task_id,
            "preflightRequestId": preflight_request_id,
            "severity": "warning",
            "ruleId": rule_id,
            "message": message,
        }),
    );
}

async fn translate_pdf_request_for_job(
    state: &AppState,
    request: TranslateRequestMessage,
    cancellation: CancellationToken,
) -> PdfTranslationRequestResult {
    translate_pdf_request_inner(state, request, cancellation, None).await
}

async fn translate_pdf_preflight_for_job(
    state: &AppState,
    request: DocumentPreflightRequestMessage,
    cancellation: CancellationToken,
    activity_handler: Option<PreflightActivityHandler>,
) -> DocumentPreflightResponseMessage {
    translate_pdf_preflight_inner(state, request, cancellation, None, activity_handler).await
}

#[cfg(test)]
async fn translate_pdf_preflight_with_api_key(
    state: &AppState,
    request: DocumentPreflightRequestMessage,
    cancellation: CancellationToken,
    api_key: String,
) -> DocumentPreflightResponseMessage {
    translate_pdf_preflight_inner(state, request, cancellation, Some(api_key), None).await
}

async fn translate_pdf_preflight_inner(
    state: &AppState,
    request: DocumentPreflightRequestMessage,
    cancellation: CancellationToken,
    api_key_override: Option<String>,
    mut activity_handler: Option<PreflightActivityHandler>,
) -> DocumentPreflightResponseMessage {
    let task_id = request.task_id.clone();
    let request_id = request.preflight_request_id.clone();
    let empty_context = crate::pdf_context::DocumentContext::empty();
    let degraded_task_id = task_id.clone();
    let degraded_request_id = request_id.clone();
    let degraded_context = empty_context.clone();
    let degraded_response = move |warnings: Vec<String>| DocumentPreflightResponseMessage {
        task_id: degraded_task_id.clone(),
        preflight_request_id: degraded_request_id.clone(),
        outcome: TranslationResponseOutcome::Completed,
        document_context: degraded_context.to_value(),
        context_hash: Some(degraded_context.hash().to_string()),
        degraded: true,
        warnings,
        error: None,
    };

    if cancellation.is_cancelled() {
        return DocumentPreflightResponseMessage {
            task_id,
            preflight_request_id: request_id,
            outcome: TranslationResponseOutcome::Cancelled,
            document_context: Value::Null,
            context_hash: None,
            degraded: true,
            warnings: Vec::new(),
            error: None,
        };
    }

    let provider = match state.database.lock() {
        Ok(connection) => match crate::db::get_provider(&connection) {
            Ok(value) => value,
            Err(error) => {
                return degraded_response(vec![format!("预检读取 Provider 失败：{error}")]);
            }
        },
        Err(_) => {
            return degraded_response(vec!["预检读取 Provider 失败：数据库锁已损坏".to_string()]);
        }
    };
    let source_text = preflight_source_text(&request);
    if source_text.is_empty() {
        return degraded_response(vec![
            "PDF 未提供可用于预检的文本样本，已使用空文档上下文".to_string(),
        ]);
    }
    let request_contract = TranslationRequest {
        request_id: request.preflight_request_id.clone(),
        source_text: source_text.clone(),
        source_language: request.source_language.clone(),
        target_language: request.target_language.clone(),
        model_id: provider.model_id.clone(),
        prompt_id: provider.prompt_id.clone(),
    };
    let empty = Value::Object(Default::default());
    let pdf_context = crate::PdfPromptContext::new(
        TranslationMode::PdfPreflight,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        &request.engine_constraints,
    );
    let prepared = match crate::prepare_pdf_translation(
        state,
        &request_contract,
        &source_text,
        &pdf_context,
    ) {
        Ok(value) => value,
        Err(error) => return degraded_response(vec![format!("文档预检准备失败：{error}")]),
    };
    let api_key = match api_key_override {
        Some(value) => value,
        None => match crate::secrets::load_api_key(&prepared.provider.id) {
            Ok(Some(value)) => value,
            Ok(None) => {
                return degraded_response(vec!["尚未配置 API Key，文档预检已跳过".to_string()]);
            }
            Err(error) => {
                return degraded_response(vec![format!(
                    "读取 API Key 失败，文档预检已跳过：{error}"
                )]);
            }
        },
    };
    let system_prompt = format!(
        "{}\n\n你正在执行 PDF 文档预检。只返回 JSON 对象，不要返回 Markdown。字段必须包括 schema_version、title、abstract、document_type、domain、headings、key_terms、abbreviations、translation_notes。key_terms 的元素使用 source、target、source_kind、confidence、note；abbreviations 的元素使用 abbreviation、expanded、target、confidence。无法确认的字段使用 null 或空数组。",
        prepared.system_prompt
    );
    let translated = TranslationCore::stream_with_usage_and_activity(
        crate::translation_core::StreamRequest {
            request_id: &request.preflight_request_id,
            base_url: &prepared.provider.base_url,
            api_key: &api_key,
            model_id: &prepared.provider.model_id,
            system_prompt: &system_prompt,
            user_text: &source_text,
            cancel: &cancellation,
            mode: TranslationMode::PdfPreflight,
            thinking_effort: &prepared.provider.thinking_effort,
        },
        |_| Ok(()),
        |activity| {
            if let Some(handler) = activity_handler.as_mut() {
                handler(activity);
            }
        },
    )
    .await;
    let translated = match translated {
        Ok(value) => value,
        Err(crate::provider::ProviderError::Cancelled) => {
            return DocumentPreflightResponseMessage {
                task_id,
                preflight_request_id: request_id,
                outcome: TranslationResponseOutcome::Cancelled,
                document_context: Value::Null,
                context_hash: None,
                degraded: true,
                warnings: Vec::new(),
                error: None,
            };
        }
        Err(error) => {
            return degraded_response(vec![format!(
                "文档预检请求失败，已继续无上下文翻译：{error}"
            )]);
        }
    };
    match crate::pdf_context::DocumentContext::from_model_output(&translated.content) {
        Ok(context) => DocumentPreflightResponseMessage {
            task_id,
            preflight_request_id: request_id,
            outcome: TranslationResponseOutcome::Completed,
            document_context: context.to_value(),
            context_hash: Some(context.hash().to_string()),
            degraded: false,
            warnings: Vec::new(),
            error: None,
        },
        Err(error) => degraded_response(vec![format!(
            "文档预检结果无法解析，已使用空上下文：{error}"
        )]),
    }
}

fn preflight_source_text(request: &DocumentPreflightRequestMessage) -> String {
    let mut samples = Vec::new();
    let mut used_chars = 0usize;
    for sample in request
        .samples
        .iter()
        .take(crate::pdf_context::MAX_PREFLIGHT_SAMPLE_COUNT)
    {
        if used_chars >= crate::pdf_context::MAX_PREFLIGHT_SAMPLE_CHARS {
            break;
        }
        let remaining = crate::pdf_context::MAX_PREFLIGHT_SAMPLE_CHARS - used_chars;
        let text = sample
            .source_text
            .chars()
            .take(remaining)
            .collect::<String>();
        if text.trim().is_empty() {
            continue;
        }
        used_chars += text.chars().count();
        samples.push(
            json!({"id": sample.segment_id, "text": text, "placeholders": sample.placeholders}),
        );
    }
    serde_json::to_string(&json!({
        "metadata": request.metadata,
        "samples": samples,
        "source_language": request.source_language,
        "target_language": request.target_language,
    }))
    .unwrap_or_default()
}

#[cfg(test)]
async fn translate_pdf_request_with_api_key(
    state: &AppState,
    request: TranslateRequestMessage,
    cancellation: CancellationToken,
    api_key: String,
) -> TranslateResponseMessage {
    translate_pdf_request_inner(state, request, cancellation, Some(api_key))
        .await
        .response
}

async fn translate_pdf_request_inner(
    state: &AppState,
    request: TranslateRequestMessage,
    cancellation: CancellationToken,
    api_key_override: Option<String>,
) -> PdfTranslationRequestResult {
    let task_id = request.task_id.clone();
    let translation_request_id = request.translation_request_id.clone();
    let failed = |code: &str, message: String| PdfTranslationRequestResult {
        response: TranslateResponseMessage {
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
        },
        persistence: None,
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
    let pdf_context = PdfPromptContext::new(
        TranslationMode::PdfSegment,
        &request.document_context,
        &request.context_before,
        &request.context_after,
        &request.task_terms,
        &request.abbreviations,
        &request.engine_constraints,
    );
    let prepared =
        match prepare_pdf_translation(state, &request_contract, &source_text, &pdf_context) {
            Ok(value) => value,
            Err(error) => return failed("translation_prepare_failed", error),
        };
    if prepared.cache_enabled {
        let cached = match state.database.lock() {
            Ok(connection) => match crate::db::find_cache(&connection, &prepared.cache_key) {
                Ok(value) => value,
                Err(error) => return failed("cache_lookup_failed", error),
            },
            Err(_) => return failed("state_lock", "应用数据库锁已损坏".to_string()),
        };
        if let Some(cached) = cached {
            let (translated_text, translated_segments) =
                match build_translation_response(&request, cached.translated_text.clone()) {
                    Ok(value) => value,
                    Err(error) => return failed("cache_validation_failed", error),
                };
            let warnings =
                quality_warnings(&request, &translated_segments, &prepared.glossary_terms);
            return PdfTranslationRequestResult {
                response: TranslateResponseMessage {
                    task_id,
                    translation_request_id,
                    outcome: TranslationResponseOutcome::Completed,
                    translated_text,
                    translated_segments,
                    token_usage: None,
                    cache_hit: true,
                    warnings,
                    error: None,
                },
                persistence: Some(make_pdf_persistence(
                    &prepared,
                    &request,
                    source_text,
                    cached.translated_text,
                    true,
                )),
            };
        }
    }
    let api_key = match api_key_override {
        Some(value) => value,
        None => match crate::secrets::load_api_key(&prepared.provider.id) {
            Ok(Some(value)) => value,
            Ok(None) => {
                return failed(
                    "missing_api_key",
                    "尚未配置 API Key，请在设置中保存 Provider".to_string(),
                );
            }
            Err(error) => return failed("api_key_failed", error),
        },
    };
    let system_prompt = prepared.system_prompt.clone();
    let translated = TranslationCore::stream_with_usage(
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
            return PdfTranslationRequestResult {
                response: TranslateResponseMessage {
                    task_id,
                    translation_request_id,
                    outcome: TranslationResponseOutcome::Cancelled,
                    translated_text: None,
                    translated_segments: Vec::new(),
                    token_usage: None,
                    cache_hit: false,
                    warnings: Vec::new(),
                    error: None,
                },
                persistence: None,
            };
        }
        Err(error) => return failed("provider_failed", error.to_string()),
    };
    let translated_for_persistence = translated.content.clone();
    let token_usage = translated.token_usage.clone();
    match build_translation_response(&request, translated.content) {
        Ok((translated_text, translated_segments)) => PdfTranslationRequestResult {
            response: TranslateResponseMessage {
                task_id,
                translation_request_id,
                outcome: TranslationResponseOutcome::Completed,
                translated_text,
                warnings: quality_warnings(
                    &request,
                    &translated_segments,
                    &prepared.glossary_terms,
                ),
                translated_segments,
                token_usage,
                cache_hit: false,
                error: None,
            },
            persistence: Some(make_pdf_persistence(
                &prepared,
                &request,
                source_text,
                translated_for_persistence,
                false,
            )),
        },
        Err(error) => failed("response_validation_failed", error),
    }
}

fn make_pdf_persistence(
    prepared: &PreparedTranslation,
    request: &TranslateRequestMessage,
    source_text: String,
    translated_text: String,
    cache_hit: bool,
) -> PdfTranslationPersistence {
    PdfTranslationPersistence {
        cache_key: prepared.cache_key.clone(),
        source_text,
        translated_text,
        source_language: request.source_language.clone(),
        target_language: request.target_language.clone(),
        provider: prepared.provider.clone(),
        prompt_id: prepared.prompt.id.clone(),
        glossary_version: prepared.glossary_version,
        cache_enabled: prepared.cache_enabled,
        cache_max_bytes: prepared.cache_max_bytes,
        history_retention: prepared.history_retention,
        cache_hit,
        index_examples: request.segments.len() == 1,
    }
}

fn commit_pdf_persistence(
    state: &AppState,
    records: &[PdfTranslationPersistence],
) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    let mut cache_max_bytes = None;
    let mut history_retention = None;
    for record in records {
        if record.cache_enabled && !record.cache_hit {
            let cache = crate::db::CacheRecord {
                cache_key: &record.cache_key,
                source_text: &record.source_text,
                translated_text: &record.translated_text,
                source_language: &record.source_language,
                target_language: &record.target_language,
                provider: &record.provider,
                prompt_id: &record.prompt_id,
                glossary_version: record.glossary_version,
            };
            if record.index_examples {
                crate::db::save_cache(&connection, &cache)?;
            } else {
                crate::db::save_pdf_cache(&connection, &cache)?;
            }
            cache_max_bytes = Some(record.cache_max_bytes);
        }
        let history = crate::db::HistoryRecord {
            source_text: &record.source_text,
            translated_text: &record.translated_text,
            source_language: &record.source_language,
            target_language: &record.target_language,
            provider: &record.provider,
            prompt_id: &record.prompt_id,
            glossary_version: record.glossary_version,
            cache_hit: record.cache_hit,
        };
        crate::db::insert_history(&connection, &history)?;
        history_retention = Some(record.history_retention);
    }
    if let Some(max_bytes) = cache_max_bytes {
        crate::db::prune_cache(&connection, max_bytes)?;
    }
    if let Some(retention) = history_retention {
        crate::db::prune_history(&connection, retention)?;
    }
    Ok(())
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

fn quality_warnings(
    request: &TranslateRequestMessage,
    translated_segments: &[TranslatedSegment],
    glossary_terms: &[crate::contracts::GlossaryTerm],
) -> Vec<String> {
    let mut warnings = Vec::new();
    for segment in translated_segments {
        let Some(source) = request
            .segments
            .iter()
            .find(|candidate| candidate.segment_id == segment.segment_id)
        else {
            continue;
        };
        if segment.translated_text.trim().is_empty() {
            warnings.push(format!("段落 {} 的译文为空", segment.segment_id));
        }
        let source_lower = source.source_text.to_lowercase();
        let translated_lower = segment.translated_text.to_lowercase();
        for (original, expected) in constrained_term_pairs(&request.task_terms)
            .into_iter()
            .chain(constrained_abbreviation_pairs(&request.abbreviations))
        {
            if !source_lower.contains(&original.to_lowercase()) {
                continue;
            }
            if glossary_terms.iter().any(|term| {
                source_lower.contains(&term.source.to_lowercase())
                    && term.source.eq_ignore_ascii_case(&original)
            }) {
                continue;
            }
            if !translated_lower.contains(&expected.to_lowercase()) {
                warnings.push(format!(
                    "段落 {} 可能未遵循 {} 的任务约束：{}",
                    segment.segment_id, original, expected
                ));
            }
        }
    }
    warnings
}

fn constrained_items<'a>(value: &'a Value, key: &'a str) -> Option<&'a Vec<Value>> {
    value
        .as_array()
        .or_else(|| value.get(key).and_then(Value::as_array))
}

fn constrained_term_pairs(value: &Value) -> Vec<(String, String)> {
    let Some(items) = constrained_items(value, "key_terms") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let original = object
                .get("source")
                .or_else(|| object.get("term"))
                .or_else(|| object.get("original"))
                .and_then(Value::as_str)?
                .trim();
            let expected = object
                .get("target")
                .or_else(|| object.get("translation"))
                .and_then(Value::as_str)?
                .trim();
            if original.is_empty() || expected.is_empty() {
                return None;
            }
            Some((original.to_string(), expected.to_string()))
        })
        .collect()
}

fn constrained_abbreviation_pairs(value: &Value) -> Vec<(String, String)> {
    let Some(items) = constrained_items(value, "abbreviations") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let original = object
                .get("source")
                .or_else(|| object.get("abbreviation"))
                .or_else(|| object.get("short"))
                .and_then(Value::as_str)?
                .trim();
            let expected = object
                .get("target")
                .or_else(|| object.get("translation"))
                .and_then(Value::as_str)?
                .trim();
            if original.is_empty() || expected.is_empty() {
                return None;
            }
            Some((original.to_string(), expected.to_string()))
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{
        PdfPreflightCompletedEvent, PreflightKey, PreflightRegistry, build_translation_response,
        commit_pdf_persistence, constrained_abbreviation_pairs, normalize_pdf_options,
        quality_warnings, request_source_text, translate_pdf_preflight_with_api_key,
        translate_pdf_request_inner, translate_pdf_request_with_api_key, validate_input_pdf,
        validate_output_pdf,
    };
    use crate::AppState;
    use crate::StartupRuntime;
    use crate::contracts::{GlossaryTerm, ThinkingEffort};
    use crate::pdf_protocol::{
        DocumentPreflightRequestMessage, DocumentPreflightResponseMessage, FinishedMessage,
        RustToWorkerMessage, StartJobMessage, TranslateRequestMessage, TranslationResponseOutcome,
        TranslationSegment, WorkerToRustMessage,
    };
    use crate::pdf_worker::{WorkerSession, WorkerSessionEvent};
    use rusqlite::Connection;
    use serde_json::{Value, json};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
    };
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn preflight_registry_matches_task_and_request_and_blocks_late_claims() {
        let registry = PreflightRegistry::default();
        let first_key = PreflightKey {
            task_id: "task-1".to_string(),
            preflight_request_id: "preflight-1".to_string(),
        };
        let first_cancellation = CancellationToken::new();
        assert!(registry.register(first_key.clone(), first_cancellation.clone()));
        assert!(!registry.register(first_key.clone(), CancellationToken::new()));

        assert!(registry.invalidate(&first_key));
        assert!(first_cancellation.is_cancelled());
        assert!(!registry.store_completion(
            &first_key,
            "pdf_translation_preflight_completed",
            PdfPreflightCompletedEvent {
                task_id: "task-1".to_string(),
                preflight_request_id: "preflight-1".to_string(),
                context: Value::Null,
                context_hash: None,
                warnings: Vec::new(),
                degraded: false,
                applied: true,
                message: None,
            },
        ));

        let second_key = PreflightKey {
            task_id: "task-2".to_string(),
            preflight_request_id: "preflight-1".to_string(),
        };
        let second_cancellation = CancellationToken::new();
        assert!(registry.register(second_key.clone(), second_cancellation.clone()));
        assert!(registry.store_completion(
            &second_key,
            "pdf_translation_preflight_completed",
            PdfPreflightCompletedEvent {
                task_id: "task-2".to_string(),
                preflight_request_id: "preflight-1".to_string(),
                context: Value::Null,
                context_hash: None,
                warnings: Vec::new(),
                degraded: false,
                applied: true,
                message: None,
            },
        ));
        assert!(registry.accept_completion(&second_key).is_some());
        assert!(!second_cancellation.is_cancelled());
        assert!(!registry.invalidate(&second_key));

        let late_key = PreflightKey {
            task_id: "task-3".to_string(),
            preflight_request_id: "preflight-1".to_string(),
        };
        let late_cancellation = CancellationToken::new();
        assert!(registry.register(late_key.clone(), late_cancellation.clone()));
        assert!(registry.store_completion(
            &late_key,
            "pdf_translation_preflight_completed",
            PdfPreflightCompletedEvent {
                task_id: "task-3".to_string(),
                preflight_request_id: "preflight-1".to_string(),
                context: Value::Null,
                context_hash: None,
                warnings: Vec::new(),
                degraded: false,
                applied: true,
                message: None,
            },
        ));
        assert!(registry.invalidate(&late_key));
        assert!(late_cancellation.is_cancelled());
        assert!(registry.accept_completion(&late_key).is_none());
    }

    fn request(segments: Vec<TranslationSegment>) -> TranslateRequestMessage {
        TranslateRequestMessage {
            task_id: "task-1".to_string(),
            translation_request_id: "request-1".to_string(),
            mode: "pdf_segment".to_string(),
            source_language: "en".to_string(),
            target_language: "zh-CN".to_string(),
            segments,
            document_context: json!({}),
            context_before: json!({}),
            context_after: json!({}),
            task_terms: json!([]),
            abbreviations: json!([]),
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
    fn quality_diagnostics_respect_global_terms_and_ignore_abbreviation_expansions() {
        let mut translation_request = request(vec![TranslationSegment {
            segment_id: "p1-s1".to_string(),
            source_text: "cache API".to_string(),
            placeholders: Vec::new(),
        }]);
        translation_request.task_terms = json!([
            {"source": "cache", "target": "临时存储"}
        ]);
        translation_request.abbreviations = json!([
            {"abbreviation": "API", "expanded": "Application Programming Interface"}
        ]);
        let translated = vec![crate::pdf_protocol::TranslatedSegment {
            segment_id: "p1-s1".to_string(),
            translated_text: "缓存 API".to_string(),
        }];
        let glossary = vec![GlossaryTerm {
            id: "global-cache".to_string(),
            source: "cache".to_string(),
            target: "缓存".to_string(),
            note: None,
        }];
        assert!(quality_warnings(&translation_request, &translated, &glossary).is_empty());
        assert!(
            constrained_abbreviation_pairs(&json!([
                {"abbreviation": "API", "expanded": "Application Programming Interface"}
            ]))
            .is_empty()
        );
    }

    #[test]
    fn options_fill_default_languages_without_overwriting_explicit_values() {
        let options = normalize_pdf_options(json!({"target_language":"ja"}))
            .expect("options should be an object");
        assert_eq!(options["source_language"], "en");
        assert_eq!(options["target_language"], "ja");
    }

    #[test]
    fn input_and_output_pdf_paths_are_confined_to_expected_files() {
        let temp = TestTempDir::new();
        let input = temp.path().join("input.PDF");
        std::fs::write(&input, b"%PDF-1.7").unwrap();
        assert!(validate_input_pdf(input.to_str().unwrap()).is_ok());
        assert!(validate_input_pdf("missing.txt").is_err());

        let output_dir = temp.path().join("output");
        std::fs::create_dir_all(&output_dir).unwrap();
        let output = output_dir.join("translated.pdf");
        std::fs::write(&output, b"%PDF-1.7").unwrap();
        assert!(validate_output_pdf(output.to_str().unwrap(), temp.path()).is_ok());
        let outside = temp.path().join("outside.pdf");
        std::fs::write(&outside, b"%PDF-1.7").unwrap();
        assert!(validate_output_pdf(outside.to_str().unwrap(), &output_dir).is_err());
    }

    #[tokio::test]
    async fn cancelled_pdf_translation_stages_no_persistence() {
        let connection = Connection::open_in_memory().unwrap();
        crate::db::migrate(&connection).unwrap();
        let state = AppState::new(
            connection,
            std::env::temp_dir(),
            Arc::new(StartupRuntime::new()),
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = translate_pdf_request_inner(
            &state,
            request(vec![TranslationSegment {
                segment_id: "p1-s1".to_string(),
                source_text: "hello".to_string(),
                placeholders: Vec::new(),
            }]),
            cancellation,
            Some("test-key".to_string()),
        )
        .await;

        assert_eq!(
            result.response.outcome,
            crate::pdf_protocol::TranslationResponseOutcome::Cancelled
        );
        assert!(result.persistence.is_none());
    }

    #[tokio::test]
    async fn pdf_translation_request_reaches_the_shared_provider_core() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let bytes_read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..bytes_read]).to_lowercase();
            assert!(request.starts_with("post /v1/chat/completions"));
            assert!(request.contains("authorization: bearer test-key"));
            assert!(request.contains("\"reasoning_effort\":\"none\""));

            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n",
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

        let connection = Connection::open_in_memory().unwrap();
        crate::db::migrate(&connection).unwrap();
        crate::db::save_provider(
            &connection,
            &format!("http://{address}/v1"),
            "stub-model",
            ThinkingEffort::None,
        )
        .unwrap();
        let state = AppState::new(
            connection,
            std::env::temp_dir(),
            Arc::new(StartupRuntime::new()),
        );
        let result = translate_pdf_request_inner(
            &state,
            request(vec![TranslationSegment {
                segment_id: "p1-s1".to_string(),
                source_text: "hello".to_string(),
                placeholders: Vec::new(),
            }]),
            CancellationToken::new(),
            Some("test-key".to_string()),
        )
        .await;

        server.join().unwrap();
        let persistence = result
            .persistence
            .clone()
            .expect("completed request should be staged for persistence");
        {
            let connection = state.database.lock().unwrap();
            assert!(
                crate::db::find_cache(&connection, &persistence.cache_key)
                    .unwrap()
                    .is_none()
            );
            let history_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM translation_history", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(history_count, 0);
        }
        commit_pdf_persistence(&state, std::slice::from_ref(&persistence)).unwrap();
        let response = result.response;
        assert_eq!(
            response.outcome,
            crate::pdf_protocol::TranslationResponseOutcome::Completed
        );
        assert_eq!(response.translated_text.as_deref(), Some("你好"));
        assert_eq!(response.translated_segments[0].segment_id, "p1-s1");
        assert_eq!(
            response.token_usage,
            Some(crate::pdf_protocol::TokenUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(4),
                total_tokens: Some(14),
            })
        );
    }

    #[tokio::test]
    async fn pdf_preflight_reaches_the_shared_provider_core_and_normalizes_context() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (provider_ready_tx, provider_ready_rx) = sync_channel(0);
        let (provider_request_tx, provider_request_rx) = sync_channel(1);
        let server = std::thread::spawn(move || {
            let context = serde_json::to_string(&json!({
                "schema_version": 1,
                "title": "A paper",
                "abstract": "A short abstract",
                "key_terms": [{
                    "source": "cache",
                    "target": "缓存",
                    "confidence": 0.9
                }],
                "abbreviations": [{
                    "abbreviation": "API",
                    "expanded": "Application Programming Interface",
                    "target": "接口",
                    "confidence": 0.8
                }]
            }))
            .unwrap();
            let body = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{"content":context}}]})
            );
            listener
                .set_nonblocking(false)
                .expect("provider stub listener should be blocking");
            provider_ready_tx
                .send(())
                .expect("provider test should still be waiting for readiness");
            let (mut stream, _) = listener.accept().expect("accept provider stub request");
            let request = read_http_request(&mut stream).expect("read provider stub request");
            let request = String::from_utf8_lossy(&request).to_lowercase();
            assert!(request.starts_with("post /v1/chat/completions"));
            assert!(request.contains("authorization: bearer test-key"));
            provider_request_tx
                .send(())
                .expect("provider test should still be waiting for the request");
            write_http_response(
                &mut stream,
                "HTTP/1.1 200 OK",
                "text/event-stream",
                body.as_bytes(),
            );
        });
        provider_ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("provider stub should start before the request");

        let connection = Connection::open_in_memory().unwrap();
        crate::db::migrate(&connection).unwrap();
        crate::db::save_provider(
            &connection,
            &format!("http://{address}/v1"),
            "stub-model",
            ThinkingEffort::None,
        )
        .unwrap();
        let state = AppState::new(
            connection,
            std::env::temp_dir(),
            Arc::new(StartupRuntime::new()),
        );
        let response = translate_pdf_preflight_with_api_key(
            &state,
            DocumentPreflightRequestMessage {
                task_id: "task-1".to_string(),
                preflight_request_id: "preflight-1".to_string(),
                source_language: "en".to_string(),
                target_language: "zh-CN".to_string(),
                metadata: json!({"file_name": "fixture.pdf"}),
                samples: vec![TranslationSegment {
                    segment_id: "p1-s1".to_string(),
                    source_text: "A representative paragraph".to_string(),
                    placeholders: Vec::new(),
                }],
                engine_constraints: json!({"response_format": "json"}),
            },
            CancellationToken::new(),
            "test-key".to_string(),
        )
        .await;

        provider_request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("provider stub should receive the client request");
        server.join().unwrap();
        assert_eq!(response.outcome, TranslationResponseOutcome::Completed);
        assert!(!response.degraded);
        assert_eq!(response.document_context["title"], "A paper");
        assert_eq!(response.document_context["schema_version"], 1);
        assert_eq!(response.context_hash.as_deref().map(str::len), Some(64));
    }

    #[test]
    #[ignore = "requires a staged external PDF Engine runtime"]
    fn optional_rust_worker_e2e_uses_an_external_engine_and_shared_core() {
        let engine_root = std::env::var_os("LILT_PDF_ENGINE_ROOT")
            .map(PathBuf::from)
            .expect("LILT_PDF_ENGINE_ROOT is required for the Rust PDF E2E test");
        let command = crate::pdf_engine::build_worker_command_from_root(&engine_root)
            .expect("external PDF Engine should be valid");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(
            "reference-projects/PDFMathTranslate-next/test/file/translate.cli.plain.text.pdf",
        );
        assert!(
            fixture.is_file(),
            "missing PDF fixture: {}",
            fixture.display()
        );

        let temp = TestTempDir::new();
        let input_pdf = temp.path().join("input.pdf");
        let output_dir = temp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("create output dir");
        std::fs::copy(&fixture, &input_pdf).expect("copy PDF fixture");

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind provider stub");
        let provider_address = listener.local_addr().expect("provider address");
        let provider_stop = Arc::new(AtomicBool::new(false));
        let provider_stop_for_thread = provider_stop.clone();
        let provider_thread = thread::spawn(move || {
            run_provider_stub(listener, provider_stop_for_thread);
        });

        let connection = Connection::open_in_memory().expect("open database");
        crate::db::migrate(&connection).expect("migrate database");
        crate::db::save_provider(
            &connection,
            &format!("http://{provider_address}/v1"),
            "stub-model",
            ThinkingEffort::None,
        )
        .expect("save test provider");
        let state = AppState::new(
            connection,
            temp.path().to_path_buf(),
            Arc::new(StartupRuntime::new()),
        );

        let task_id = "rust-pdf-e2e".to_string();
        let session = WorkerSession::spawn(command, task_id.clone()).expect("spawn Worker");
        session
            .send(RustToWorkerMessage::StartJob(StartJobMessage {
                protocol_version: crate::pdf_protocol::PDF_WORKER_PROTOCOL_VERSION,
                task_id: task_id.clone(),
                input_pdf: input_pdf.to_string_lossy().into_owned(),
                output_dir: output_dir.to_string_lossy().into_owned(),
                engine_version: crate::pdf_engine::BABELDOC_ENGINE_VERSION.to_string(),
                pdf_options: json!({
                    "source_language": "en",
                    "target_language": "zh-CN",
                    "output_mode": "bilingual"
                }),
            }))
            .expect("send START_JOB");

        let runtime = tokio::runtime::Runtime::new().expect("create test runtime");
        let deadline = Instant::now() + Duration::from_secs(180);
        let output_pdf = loop {
            assert!(Instant::now() < deadline, "Rust PDF E2E timed out");
            match session.recv_timeout(Duration::from_millis(250)) {
                Ok(WorkerSessionEvent::Message(message)) => match *message {
                    WorkerToRustMessage::DocumentPreflightRequest(request) => {
                        session
                            .send(RustToWorkerMessage::DocumentPreflightResponse(
                                DocumentPreflightResponseMessage {
                                    task_id: request.task_id,
                                    preflight_request_id: request.preflight_request_id,
                                    outcome: TranslationResponseOutcome::Completed,
                                    document_context: json!({
                                        "schema_version": 1,
                                        "title": "Rust PDF E2E fixture",
                                        "key_terms": [],
                                        "abbreviations": [],
                                        "translation_notes": []
                                    }),
                                    context_hash: Some("rust-pdf-e2e-context".to_string()),
                                    degraded: false,
                                    warnings: Vec::new(),
                                    error: None,
                                },
                            ))
                            .expect("send DOCUMENT_PREFLIGHT_RESPONSE");
                    }
                    WorkerToRustMessage::TranslateRequest(request) => {
                        let response = runtime.block_on(translate_pdf_request_with_api_key(
                            &state,
                            request,
                            CancellationToken::new(),
                            "test-key".to_string(),
                        ));
                        session
                            .send(RustToWorkerMessage::TranslateResponse(response))
                            .expect("send TRANSLATE_RESPONSE");
                    }
                    WorkerToRustMessage::Finished(FinishedMessage { output_pdf, .. }) => {
                        break PathBuf::from(output_pdf);
                    }
                    WorkerToRustMessage::Error(error) => {
                        panic!("Worker returned an error: {}", error.error.message);
                    }
                    _ => {}
                },
                Ok(WorkerSessionEvent::WorkerExited(code)) => {
                    panic!("Worker exited before FINISHED: {code:?}");
                }
                Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("Worker event channel closed before FINISHED");
                }
            }
        };

        session.close_writer();
        let _ = session.terminate();
        provider_stop.store(true, Ordering::Release);
        let _ = provider_thread.join();
        assert!(
            output_pdf.is_file(),
            "missing output PDF: {}",
            output_pdf.display()
        );
        let bytes = std::fs::read(output_pdf).expect("read output PDF");
        assert!(bytes.starts_with(b"%PDF-"), "output is not a PDF");
    }

    struct TestTempDir(PathBuf);

    impl TestTempDir {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("lilt-pdf-jobs-{suffix}"));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run_provider_stub(listener: TcpListener, stop: Arc<AtomicBool>) {
        listener.set_nonblocking(true).expect("set listener mode");
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => respond_to_provider_request(&mut stream),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    }

    fn respond_to_provider_request(stream: &mut TcpStream) {
        let Some(request) = read_http_request(stream) else {
            return;
        };
        let lower = String::from_utf8_lossy(&request).to_lowercase();
        if !lower.contains("authorization: bearer test-key") {
            write_http_response(stream, "401 Unauthorized", "application/json", b"{}");
            return;
        }
        let Some(body_start) = request.windows(4).position(|value| value == b"\r\n\r\n") else {
            return;
        };
        let body = &request[body_start + 4..];
        let payload: serde_json::Value = match serde_json::from_slice(body) {
            Ok(payload) => payload,
            Err(_) => {
                write_http_response(stream, "400 Bad Request", "application/json", b"{}");
                return;
            }
        };
        let source = payload["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default();
        let translated = translate_stub_source(source);
        let sse = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":translated}}]})
        );
        write_http_response(stream, "200 OK", "text/event-stream", sse.as_bytes());
    }

    fn translate_stub_source(source: &str) -> String {
        if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(source) {
            let translated = items
                .into_iter()
                .map(|item| {
                    json!({
                        "id": item["id"].clone(),
                        "output": format!("【Stub】{}", item["input"].as_str().unwrap_or_default())
                    })
                })
                .collect::<Vec<_>>();
            return serde_json::to_string(&translated).expect("encode stub batch");
        }
        format!("【Stub】{source}")
    }

    fn read_http_request(stream: &mut TcpStream) -> Option<Vec<u8>> {
        stream.set_nonblocking(false).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .ok()?;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let count = stream.read(&mut chunk).ok()?;
            if count == 0 {
                return None;
            }
            bytes.extend_from_slice(&chunk[..count]);
            let Some(header_end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            if let Some(content_length) = content_length {
                let required = header_end + 4 + content_length;
                if bytes.len() >= required {
                    bytes.truncate(required);
                    return Some(bytes);
                }
                continue;
            }
            let chunked = headers.lines().any(|line| {
                let Some((name, value)) = line.split_once(':') else {
                    return false;
                };
                name.trim().eq_ignore_ascii_case("transfer-encoding")
                    && value
                        .split(',')
                        .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
            });
            if chunked {
                if let Some(body) = decode_chunked_body(&bytes[header_end + 4..]) {
                    let mut request = bytes[..header_end + 4].to_vec();
                    request.extend_from_slice(&body);
                    return Some(request);
                }
                continue;
            }
            return None;
        }
    }

    fn decode_chunked_body(body: &[u8]) -> Option<Vec<u8>> {
        let mut offset = 0;
        let mut decoded = Vec::new();
        loop {
            let line_end = body[offset..]
                .windows(2)
                .position(|value| value == b"\r\n")?
                + offset;
            let size = usize::from_str_radix(
                body[offset..line_end]
                    .split(|byte| *byte == b';')
                    .next()
                    .and_then(|value| std::str::from_utf8(value).ok())?
                    .trim(),
                16,
            )
            .ok()?;
            offset = line_end + 2;
            if size == 0 {
                if body.get(offset..)?.starts_with(b"\r\n") {
                    return Some(decoded);
                }
                body[offset..]
                    .windows(4)
                    .position(|value| value == b"\r\n\r\n")?;
                return Some(decoded);
            }
            let data_end = offset.checked_add(size)?;
            if body.len() < data_end + 2 || &body[data_end..data_end + 2] != b"\r\n" {
                return None;
            }
            decoded.extend_from_slice(&body[offset..data_end]);
            offset = data_end + 2;
        }
    }

    fn write_http_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
    }
}
