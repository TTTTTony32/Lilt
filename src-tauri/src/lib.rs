mod contracts;
mod db;
mod diagnostics;
mod dictionary;
mod examples;
mod provider;
mod secrets;
mod selection;

use contracts::{
    AppSnapshot, DictionaryCommandResult, DictionaryLookupCandidate, DictionaryLookupCommandResult,
    DictionaryState, GlossaryTerm, ModelInfo, ParagraphExample, Prompt, ProviderConfig,
    SelectionMode, SelectionNotice, SelectionRequestPayload, SelectionRuntimeStatus,
    SelectionSettingsResult, TranslationCancelled, TranslationCommandResult, TranslationCompleted,
    TranslationFailed, TranslationRequest, TranslationStarted, WordExampleCancelled,
    WordExampleCommandResult, WordExampleCompleted, WordExampleFailed, WordExamplePosDelta,
    WordExampleRequest, WordExampleStarted, WordExampleTranslationDelta, DEFAULT_PROVIDER_ID,
    DICTIONARY_HISTORY_LIMIT, WORD_EXAMPLE_PROTOCOL_VERSION,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewWindow, WindowEvent};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const STARTUP_BACKGROUND_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

struct StartupGate {
    first_paint: AtomicBool,
}

impl StartupGate {
    fn new() -> Self {
        Self {
            first_paint: AtomicBool::new(false),
        }
    }

    fn open(&self) {
        self.first_paint.store(true, Ordering::Release);
    }

    async fn wait(&self, timeout: std::time::Duration) -> bool {
        if self.first_paint.load(Ordering::Acquire) {
            return true;
        }
        let deadline = Instant::now() + timeout;
        loop {
            if self.first_paint.load(Ordering::Acquire) {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            tokio::time::sleep(remaining.min(std::time::Duration::from_millis(16))).await;
        }
    }
}

struct StartupRuntime {
    started_at: Instant,
    gate: StartupGate,
}

impl StartupRuntime {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            gate: StartupGate::new(),
        }
    }

    fn log(&self, stage: &str, window: &str) {
        diagnostics::info(format!(
            "startup.stage stage={stage} window={window} elapsed_ms={}",
            self.started_at.elapsed().as_millis()
        ));
    }

    fn report_frontend_stage(&self, stage: &str, window: &str) -> Result<(), String> {
        if !matches!(stage, "webview_script" | "dom_mounted" | "first_paint") {
            return Err("无效的启动阶段".to_string());
        }
        self.log(stage, window);
        if stage == "first_paint" {
            self.gate.open();
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) database: Arc<Mutex<Connection>>,
    pub(crate) cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) dictionary_update: Arc<Mutex<Option<String>>>,
    pub(crate) dictionary_store: Arc<Mutex<dictionary::DictionaryStore>>,
    pub(crate) dictionary_initialising: Arc<AtomicBool>,
    pub(crate) selection: selection::SelectionService,
    startup: Arc<StartupRuntime>,
}

impl AppState {
    fn new(connection: Connection, data_dir: PathBuf, startup: Arc<StartupRuntime>) -> Self {
        let cancellations = Arc::new(Mutex::new(HashMap::new()));
        let selection = selection::SelectionService::new(cancellations.clone());
        let dictionary_store = dictionary::DictionaryStore::new(data_dir.join("dictionary"));
        diagnostics::info("dictionary.store.deferred source=startup");
        Self {
            database: Arc::new(Mutex::new(connection)),
            cancellations,
            data_dir: Arc::new(data_dir),
            dictionary_update: Arc::new(Mutex::new(None)),
            dictionary_store: Arc::new(Mutex::new(dictionary_store)),
            dictionary_initialising: Arc::new(AtomicBool::new(false)),
            selection,
            startup,
        }
    }

    fn dictionary_dir(&self) -> PathBuf {
        self.data_dir.join("dictionary")
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .on_window_event(|window, event| {
            if window.label() == "selection" && matches!(event, WindowEvent::Focused(false)) {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    state.selection.hide_window();
                }
            }
        })
        .setup(|app| {
            let startup = Arc::new(StartupRuntime::new());
            startup.log("setup.begin", "main");
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| std::io::Error::other(format!("无法定位应用数据目录：{error}")))?;
            initialise_database(&data_dir).map_err(std::io::Error::other)?;
            startup.log("data_dir.completed", "main");
            let connection = Connection::open(data_dir.join("app.sqlite"))
                .map_err(|error| std::io::Error::other(format!("打开应用数据库失败：{error}")))?;
            startup.log("database.opened", "main");
            db::migrate(&connection).map_err(std::io::Error::other)?;
            startup.log("database.migrated", "main");
            let state = AppState::new(connection, data_dir, startup.clone());
            startup.log("state.created", "main");
            let (selection_mode, selection_shortcut) = {
                let connection = state
                    .database
                    .lock()
                    .map_err(|_| std::io::Error::other("应用数据库锁已损坏"))?;
                db::get_selection_settings(&connection).map_err(std::io::Error::other)?
            };
            state.selection.attach_app(app.handle().clone());
            app.manage(state.clone());
            schedule_selection_initialisation(
                state.clone(),
                app.handle().clone(),
                selection_mode,
                selection_shortcut,
            );
            startup.log("selection.scheduled", "main");
            schedule_dictionary_initialisation(state.clone());
            schedule_pending_example_indexes(state);
            startup.log("background.scheduled", "main");
            startup.log("setup.completed", "main");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            report_startup_stage,
            get_app_snapshot,
            save_provider_config,
            fetch_models,
            save_app_settings,
            configure_selection,
            get_selection_status,
            set_selection_language,
            selection_window_ready,
            get_selection_request,
            open_selection_in_main,
            dismiss_selection,
            translate,
            cancel_translation,
            upsert_glossary_term,
            delete_glossary_term,
            query_dictionary,
            generate_word_example,
            cancel_word_example,
            get_dictionary_state,
            update_dictionary,
            clear_dictionary_history
        ])
        .build(tauri::generate_context!())
        .expect("error while building Lilt");
    app.run(|app, event| {
        if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
            if let Some(state) = app.try_state::<AppState>() {
                state.selection.shutdown();
            }
        }
    });
}

fn initialise_database(data_dir: &PathBuf) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|error| format!("创建应用数据目录失败：{error}"))
}

#[tauri::command]
fn report_startup_stage(
    window: WebviewWindow,
    state: State<'_, AppState>,
    stage: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("启动阶段只允许主窗口上报".to_string());
    }
    state.startup.report_frontend_stage(&stage, window.label())
}

#[tauri::command]
fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let snapshot_data = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        read_snapshot_data(&connection)?
    };
    let updating = state
        .dictionary_update
        .lock()
        .map_err(|_| "词典更新状态锁已损坏".to_string())?
        .is_some();
    let mut dictionary = if state.dictionary_initialising.load(Ordering::Acquire) {
        dictionary::deferred_state(
            &state.dictionary_dir(),
            snapshot_data.dictionary_installation.as_ref(),
        )
    } else {
        state
            .dictionary_store
            .lock()
            .map_err(|_| "词典存储锁已损坏".to_string())?
            .state(snapshot_data.dictionary_installation.as_ref())
    };
    if updating {
        dictionary.status = contracts::DictionaryStatus::Updating;
        dictionary.error = None;
    }
    Ok(AppSnapshot {
        settings: snapshot_data.settings,
        provider: snapshot_data.provider,
        models: snapshot_data.models,
        prompts: snapshot_data.prompts,
        glossary_terms: snapshot_data.glossary_terms,
        history: snapshot_data.history,
        cache_stats: snapshot_data.cache_stats,
        dictionary,
        dictionary_history: snapshot_data.dictionary_history,
    })
}

struct SnapshotData {
    settings: contracts::AppSettings,
    provider: contracts::ProviderConfig,
    models: Vec<ModelInfo>,
    prompts: Vec<Prompt>,
    glossary_terms: Vec<GlossaryTerm>,
    history: Vec<contracts::HistoryEntry>,
    cache_stats: contracts::CacheStats,
    dictionary_installation: Option<db::DictionaryInstallation>,
    dictionary_history: Vec<contracts::DictionaryHistoryEntry>,
}

fn read_snapshot_data(connection: &Connection) -> Result<SnapshotData, String> {
    let settings = db::get_settings(connection)?;
    let provider = db::get_provider(connection)?;
    let has_api_key = secrets::load_api_key(&provider.id).ok().flatten().is_some();
    let models = db::list_models(connection)?;
    let prompts = db::list_prompts(connection)?;
    let glossary_terms = db::list_glossary_terms(connection)?;
    let history = db::get_history(connection, settings.history_retention)?;
    let cache_stats = db::get_cache_stats(connection, settings.cache_max_bytes)?;
    let dictionary_installation = db::get_dictionary_installation(connection)?;
    let dictionary_history = db::list_dictionary_history(connection, DICTIONARY_HISTORY_LIMIT)?;
    Ok(SnapshotData {
        settings,
        provider: ProviderConfig {
            id: provider.id,
            name: provider.name,
            base_url: provider.base_url,
            model_id: provider.model_id,
            prompt_id: provider.prompt_id,
            has_api_key,
        },
        models,
        prompts,
        glossary_terms,
        history,
        cache_stats,
        dictionary_installation,
        dictionary_history,
    })
}

#[tauri::command]
fn save_provider_config(
    state: State<'_, AppState>,
    base_url: String,
    model_id: String,
    prompt_id: String,
    api_key: Option<String>,
) -> Result<(), String> {
    let normalized_url =
        provider::normalize_base_url(&base_url).map_err(|error| error.to_string())?;
    let model_id = model_id.trim();
    let prompt_id = prompt_id.trim();
    if model_id.is_empty() || prompt_id.is_empty() {
        return Err("Model ID 和 Prompt 不能为空".to_string());
    }
    if api_key.is_some() {
        secrets::save_api_key(DEFAULT_PROVIDER_ID, api_key.as_deref())?;
    }
    diagnostics::info(format!(
        "command.save_provider_config origin={} model={} api_key_provided={}",
        provider::safe_endpoint_origin(&normalized_url),
        model_id,
        api_key.is_some()
    ));
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::save_provider(&connection, &normalized_url, model_id, prompt_id)
}

#[tauri::command]
async fn fetch_models(
    state: State<'_, AppState>,
    base_url: Option<String>,
    api_key: Option<String>,
) -> Result<Vec<ModelInfo>, String> {
    let (saved_base_url, provider_id) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let provider = db::get_provider(&connection)?;
        (provider.base_url, provider.id)
    };
    let base_url = base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&saved_base_url);
    let base_url = match provider::normalize_base_url(base_url) {
        Ok(value) => value,
        Err(error) => {
            diagnostics::error(format!(
                "command.fetch_models.invalid_base_url reason={error}"
            ));
            return Err(error.to_string());
        }
    };
    let (api_key, key_source) = match api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => (value.to_string(), "draft"),
        None => match secrets::load_api_key(&provider_id) {
            Ok(Some(value)) => (value, "stored"),
            Ok(None) => {
                diagnostics::error("command.fetch_models.missing_api_key");
                return Err("尚未配置 API Key".to_string());
            }
            Err(error) => {
                diagnostics::error(format!(
                    "command.fetch_models.api_key_failed reason={error}"
                ));
                return Err(error);
            }
        },
    };
    diagnostics::info(format!(
        "command.fetch_models.start provider_id={} origin={} api_key_source={key_source}",
        provider_id,
        provider::safe_endpoint_origin(&base_url)
    ));
    let models = provider::fetch_models(&base_url, &api_key)
        .await
        .map_err(|error| {
            diagnostics::error(format!("command.fetch_models.failed reason={error}"));
            error.to_string()
        })?;
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::replace_models(&connection, &models).map_err(|error| {
        diagnostics::error(format!(
            "command.fetch_models.persist_failed reason={error}"
        ));
        error
    })?;
    diagnostics::info(format!(
        "command.fetch_models.completed model_count={}",
        models.len()
    ));
    Ok(models)
}

#[tauri::command]
fn save_app_settings(
    state: State<'_, AppState>,
    history_retention: i64,
    cache_enabled: bool,
    cache_max_bytes: i64,
    word_ai_cache_enabled: bool,
    paragraph_example_lookup_enabled: bool,
) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::save_settings(
        &connection,
        history_retention,
        cache_enabled,
        cache_max_bytes,
        word_ai_cache_enabled,
        paragraph_example_lookup_enabled,
    )
}

#[tauri::command]
fn configure_selection(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: SelectionMode,
    shortcut: String,
) -> Result<SelectionSettingsResult, String> {
    let previous = state.selection.status();
    let status = state.selection.configure(&app, mode, &shortcut)?;
    let settings = match state.database.lock() {
        Ok(connection) => {
            if let Err(error) = db::save_selection_settings(&connection, mode, shortcut.trim()) {
                let _ = state
                    .selection
                    .configure(&app, previous.mode, &previous.shortcut);
                return Err(error);
            }
            db::get_settings(&connection)?
        }
        Err(_) => {
            let _ = state
                .selection
                .configure(&app, previous.mode, &previous.shortcut);
            return Err("应用数据库锁已损坏".to_string());
        }
    };
    Ok(SelectionSettingsResult { settings, status })
}

#[tauri::command]
fn get_selection_status(state: State<'_, AppState>) -> Result<SelectionRuntimeStatus, String> {
    Ok(state.selection.status())
}

#[tauri::command]
fn set_selection_language(
    state: State<'_, AppState>,
    source_language: String,
    target_language: String,
) -> Result<(), String> {
    state
        .selection
        .set_language(source_language, target_language);
    Ok(())
}

#[tauri::command]
fn selection_window_ready(state: State<'_, AppState>) -> Result<Option<SelectionNotice>, String> {
    Ok(state.selection.window_ready())
}

#[tauri::command]
fn get_selection_request(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<SelectionRequestPayload, String> {
    state.selection.get_request(&request_id)
}

#[tauri::command]
fn open_selection_in_main(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    state.selection.open_in_main(&request_id)
}

#[tauri::command]
fn dismiss_selection(state: State<'_, AppState>, request_id: Option<String>) -> Result<(), String> {
    state.selection.dismiss(request_id.as_deref());
    Ok(())
}

#[tauri::command]
async fn translate(
    app: AppHandle,
    state: State<'_, AppState>,
    request: TranslationRequest,
) -> Result<TranslationCommandResult, String> {
    translate_impl(app, state.inner().clone(), request).await
}

async fn translate_impl(
    app: AppHandle,
    state: AppState,
    request: TranslationRequest,
) -> Result<TranslationCommandResult, String> {
    let request_id = if request.request_id.trim().is_empty() {
        Uuid::new_v4().to_string()
    } else {
        request.request_id.clone()
    };
    let source_text = request.source_text.trim().to_string();
    if source_text.is_empty() {
        return emit_failed(&app, &request_id, "原文不能为空");
    }
    if source_text.chars().count() > 100_000 {
        return emit_failed(&app, &request_id, "单次翻译最多支持 100,000 个字符");
    }
    diagnostics::info(format!(
        "command.translate.start request_id={} source_chars={} source_language={} target_language={} model={}",
        request_id,
        source_text.chars().count(),
        request.source_language,
        request.target_language,
        request.model_id
    ));
    let cancellation = CancellationToken::new();
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| "取消状态锁已损坏".to_string())?;
        cancellations.insert(request_id.clone(), cancellation.clone());
    }
    if let Err(error) = app.emit(
        "translation_started",
        TranslationStarted {
            request_id: request_id.clone(),
        },
    ) {
        unregister_request(&state, &request_id);
        return Err(format!("发送翻译状态失败：{error}"));
    }

    let preparation = prepare_translation(&state, &request, &source_text);
    let prepared = match preparation {
        Ok(value) => value,
        Err(error) => {
            diagnostics::error(format!(
                "command.translate.prepare_failed request_id={} reason={error}",
                request_id
            ));
            unregister_request(&state, &request_id);
            return emit_failed(&app, &request_id, &error);
        }
    };

    if prepared.cache_enabled {
        let cached = {
            let connection = state
                .database
                .lock()
                .map_err(|_| "应用数据库锁已损坏".to_string())?;
            match db::find_cache(&connection, &prepared.cache_key) {
                Ok(value) => value,
                Err(error) => {
                    diagnostics::error(format!(
                        "command.translate.cache_lookup_failed request_id={} reason={error}",
                        request_id
                    ));
                    unregister_request(&state, &request_id);
                    return emit_failed(&app, &request_id, &error);
                }
            }
        };
        if let Some(cached) = cached {
            diagnostics::info(format!(
                "command.translate.cache_hit request_id={request_id}"
            ));
            let persist = {
                let connection = state
                    .database
                    .lock()
                    .map_err(|_| "应用数据库锁已损坏".to_string())?;
                let history = db::HistoryRecord {
                    source_text: &source_text,
                    translated_text: &cached.translated_text,
                    source_language: &request.source_language,
                    target_language: &request.target_language,
                    provider: &prepared.provider,
                    prompt_id: &prepared.prompt.id,
                    glossary_version: prepared.glossary_version,
                    cache_hit: true,
                };
                db::insert_history(&connection, &history)
                    .and_then(|_| db::prune_history(&connection, prepared.history_retention))
            };
            unregister_request(&state, &request_id);
            if let Err(error) = persist {
                diagnostics::error(format!(
                    "command.translate.persistence_failed request_id={} reason={error}",
                    request_id
                ));
                return emit_failed(&app, &request_id, &error);
            }
            let content = cached.translated_text;
            schedule_example_index(&state, &prepared.cache_key);
            app.emit(
                "translation_completed",
                TranslationCompleted {
                    request_id: request_id.clone(),
                    content: content.clone(),
                    cache_hit: true,
                },
            )
            .map_err(|error| format!("发送翻译结果失败：{error}"))?;
            diagnostics::info(format!(
                "command.translate.completed request_id={} cache_hit=true",
                request_id
            ));
            return Ok(TranslationCommandResult::completed(content, true));
        }
        diagnostics::info(format!(
            "command.translate.cache_miss request_id={}",
            request_id
        ));
    }

    let api_key = match secrets::load_api_key(&prepared.provider.id) {
        Ok(Some(value)) => value,
        Ok(None) => {
            diagnostics::error(format!(
                "command.translate.missing_api_key request_id={}",
                request_id
            ));
            unregister_request(&state, &request_id);
            return emit_failed(
                &app,
                &request_id,
                "尚未配置 API Key，请在设置中保存 Provider",
            );
        }
        Err(error) => {
            diagnostics::error(format!(
                "command.translate.api_key_failed request_id={} reason={error}",
                request_id
            ));
            unregister_request(&state, &request_id);
            return emit_failed(&app, &request_id, &error);
        }
    };

    diagnostics::info(format!(
        "command.translate.provider_request request_id={} provider_id={} model={}",
        request_id, prepared.provider.id, prepared.provider.model_id
    ));
    let translated = provider::translate_stream(provider::StreamRequest {
        app: &app,
        request_id: &request_id,
        base_url: &prepared.provider.base_url,
        api_key: &api_key,
        model_id: &prepared.provider.model_id,
        system_prompt: &prepared.system_prompt,
        user_text: &source_text,
        cancel: &cancellation,
    })
    .await;
    unregister_request(&state, &request_id);

    let translated = match translated {
        Ok(value) => value,
        Err(provider::ProviderError::Cancelled) => {
            diagnostics::info(format!(
                "command.translate.cancelled request_id={}",
                request_id
            ));
            app.emit("translation_cancelled", TranslationCancelled { request_id })
                .map_err(|error| format!("发送取消状态失败：{error}"))?;
            return Ok(TranslationCommandResult::cancelled());
        }
        Err(error) => {
            diagnostics::error(format!(
                "command.translate.provider_failed request_id={} reason={error}",
                request_id
            ));
            return emit_failed(&app, &request_id, &error.to_string());
        }
    };

    let persistence = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let cache_result = if prepared.cache_enabled {
            let cache = db::CacheRecord {
                cache_key: &prepared.cache_key,
                source_text: &source_text,
                translated_text: &translated,
                source_language: &request.source_language,
                target_language: &request.target_language,
                provider: &prepared.provider,
                prompt_id: &prepared.prompt.id,
                glossary_version: prepared.glossary_version,
            };
            db::save_cache(&connection, &cache)
                .and_then(|_| db::prune_cache(&connection, prepared.cache_max_bytes))
        } else {
            Ok(())
        };
        cache_result.and_then(|_| {
            let history = db::HistoryRecord {
                source_text: &source_text,
                translated_text: &translated,
                source_language: &request.source_language,
                target_language: &request.target_language,
                provider: &prepared.provider,
                prompt_id: &prepared.prompt.id,
                glossary_version: prepared.glossary_version,
                cache_hit: false,
            };
            db::insert_history(&connection, &history)
                .and_then(|_| db::prune_history(&connection, prepared.history_retention))
        })
    };
    if let Err(error) = persistence {
        diagnostics::error(format!(
            "command.translate.persistence_failed request_id={} reason={error}",
            request_id
        ));
        return emit_failed(&app, &request_id, &error);
    }
    if prepared.cache_enabled {
        schedule_example_index(&state, &prepared.cache_key);
    }
    let content = translated;
    let output_chars = content.chars().count();
    app.emit(
        "translation_completed",
        TranslationCompleted {
            request_id: request_id.clone(),
            content: content.clone(),
            cache_hit: false,
        },
    )
    .map_err(|error| format!("发送翻译结果失败：{error}"))?;
    diagnostics::info(format!(
        "command.translate.completed request_id={} cache_hit=false output_chars={}",
        request_id, output_chars
    ));
    Ok(TranslationCommandResult::completed(content, false))
}

struct PreparedTranslation {
    provider: contracts::ProviderRecord,
    prompt: Prompt,
    system_prompt: String,
    cache_key: String,
    cache_enabled: bool,
    cache_max_bytes: i64,
    history_retention: i64,
    glossary_version: i64,
}

fn prepare_translation(
    state: &AppState,
    request: &TranslationRequest,
    source_text: &str,
) -> Result<PreparedTranslation, String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    let settings = db::get_settings(&connection)?;
    let mut provider = db::get_provider(&connection)?;
    provider.model_id = request.model_id.trim().to_string();
    provider.prompt_id = request.prompt_id.trim().to_string();
    if provider.model_id.is_empty() || provider.prompt_id.is_empty() {
        return Err("Model ID 和 Prompt 不能为空".to_string());
    }
    provider.base_url =
        provider::normalize_base_url(&provider.base_url).map_err(|error| error.to_string())?;
    let prompt = db::get_prompt(&connection, &provider.prompt_id)?;
    let terms = db::list_glossary_terms(&connection)?;
    let glossary_version = db::glossary_version(&connection)?;
    let system_prompt = build_system_prompt(&prompt.content, &terms, source_text);
    let cache_key = make_cache_key(&CacheKeyInput {
        base_url: &provider.base_url,
        provider_id: &provider.id,
        model_id: &provider.model_id,
        prompt_id: &prompt.id,
        prompt_version: prompt.version,
        glossary_version,
        source_language: &request.source_language,
        target_language: &request.target_language,
        source_text,
    });
    Ok(PreparedTranslation {
        provider,
        prompt,
        system_prompt,
        cache_key,
        cache_enabled: settings.cache_enabled,
        cache_max_bytes: settings.cache_max_bytes,
        history_retention: settings.history_retention,
        glossary_version,
    })
}

fn build_system_prompt(base_prompt: &str, terms: &[GlossaryTerm], source_text: &str) -> String {
    let source_lower = source_text.to_lowercase();
    let mut hits = terms
        .iter()
        .filter(|term| source_lower.contains(&term.source.to_lowercase()))
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .source
            .len()
            .cmp(&left.source.len())
            .then_with(|| left.source.cmp(&right.source))
    });
    if hits.is_empty() {
        return base_prompt.to_string();
    }
    let glossary = hits
        .iter()
        .map(|term| format!("- {}：{}", term.source, term.target))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{base_prompt}\n\n仅在原文命中以下术语时遵循对应译法：\n{glossary}")
}

struct CacheKeyInput<'a> {
    base_url: &'a str,
    provider_id: &'a str,
    model_id: &'a str,
    prompt_id: &'a str,
    prompt_version: i64,
    glossary_version: i64,
    source_language: &'a str,
    target_language: &'a str,
    source_text: &'a str,
}

fn make_cache_key(input: &CacheKeyInput<'_>) -> String {
    let canonical = format!(
        "base={}\nprovider={}\nmodel={}\nprompt={}@{}\nglossary={}\nsource_language={}\ntarget_language={}\nsource={}",
        input.base_url,
        input.provider_id,
        input.model_id,
        input.prompt_id,
        input.prompt_version,
        input.glossary_version,
        input.source_language,
        input.target_language,
        input.source_text,
    );
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

#[tauri::command]
fn cancel_translation(state: State<'_, AppState>, request_id: String) -> Result<bool, String> {
    let still_active = cancel_request(state.inner(), &request_id)?;
    if still_active {
        diagnostics::info(format!(
            "command.translate.cancel_requested request_id={request_id}"
        ));
    } else {
        diagnostics::warn(format!(
            "command.translate.cancel_missing request_id={request_id}"
        ));
    }
    Ok(still_active)
}

#[tauri::command]
fn cancel_word_example(state: State<'_, AppState>, request_id: String) -> Result<bool, String> {
    let still_active = cancel_request(state.inner(), &request_id)?;
    if still_active {
        diagnostics::info(format!(
            "command.word_example.cancel_requested request_id={request_id}"
        ));
    } else {
        diagnostics::warn(format!(
            "command.word_example.cancel_missing request_id={request_id}"
        ));
    }
    Ok(still_active)
}

fn cancel_request(state: &AppState, request_id: &str) -> Result<bool, String> {
    let cancellation = state
        .cancellations
        .lock()
        .map_err(|_| "取消状态锁已损坏".to_string())?
        .get(request_id)
        .cloned();
    if let Some(token) = cancellation {
        token.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

fn schedule_selection_initialisation(
    state: AppState,
    app: AppHandle,
    mode: SelectionMode,
    shortcut: String,
) {
    tauri::async_runtime::spawn(async move {
        state.selection.start_worker();
        if let Err(error) = state.selection.configure(&app, mode, &shortcut) {
            diagnostics::error(format!("selection.configure.startup_failed reason={error}"));
            return;
        }
        state.startup.log("selection.ready", "main");
    });
}

fn schedule_dictionary_initialisation(state: AppState) {
    state.dictionary_initialising.store(true, Ordering::Release);
    tauri::async_runtime::spawn(async move {
        let first_paint = state.startup.gate.wait(STARTUP_BACKGROUND_WAIT).await;
        if !first_paint {
            diagnostics::warn("startup.gate.timeout task=dictionary");
        }
        state.startup.log("background.dictionary.begin", "main");
        let worker_state = state.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let mut store = worker_state
                .dictionary_store
                .lock()
                .map_err(|_| "词典存储锁已损坏".to_string())?;
            if store.is_runtime_ready() {
                return Ok(());
            }
            store
                .open_runtime("startup")
                .map_err(|error| error.to_string())
        })
        .await;
        state
            .dictionary_initialising
            .store(false, Ordering::Release);
        match result {
            Ok(Ok(())) => diagnostics::info("dictionary.store.ready source=startup"),
            Ok(Err(error)) if error == "词典未安装，请在设置中下载词典" => {
                diagnostics::info("dictionary.store.not_installed source=startup");
            }
            Ok(Err(error)) => diagnostics::error(format!(
                "dictionary.store.failed source=startup error={error}"
            )),
            Err(error) => diagnostics::error(format!(
                "dictionary.store.worker_failed source=startup error={error}"
            )),
        }
    });
}

fn schedule_pending_example_indexes(state: AppState) {
    tauri::async_runtime::spawn(async move {
        let first_paint = state.startup.gate.wait(STARTUP_BACKGROUND_WAIT).await;
        if !first_paint {
            diagnostics::warn("startup.gate.timeout task=examples");
        }
        state.startup.log("background.examples.begin", "main");

        let mut cursor = None;
        loop {
            let batch = match tauri::async_runtime::spawn_blocking({
                let state = state.clone();
                let cursor = cursor.clone();
                move || {
                    let connection = state
                        .database
                        .lock()
                        .map_err(|_| "应用数据库锁已损坏".to_string())?;
                    db::enqueue_missing_example_indexes(&connection, cursor.as_deref(), 128)
                }
            })
            .await
            {
                Ok(Ok(batch)) => batch,
                Ok(Err(error)) => {
                    diagnostics::error(format!(
                        "examples.index_backfill.enqueue_failed reason={error}"
                    ));
                    return;
                }
                Err(error) => {
                    diagnostics::error(format!(
                        "examples.index_backfill.enqueue_worker_failed reason={error}"
                    ));
                    return;
                }
            };
            let Some(last_key) = batch.last_key else {
                break;
            };
            diagnostics::info(format!(
                "examples.index_backfill.enqueued count={} has_cursor=true",
                batch.inserted
            ));
            cursor = Some(last_key);
            tokio::task::yield_now().await;
        }

        let keys = match tauri::async_runtime::spawn_blocking({
            let state = state.clone();
            move || {
                let connection = state
                    .database
                    .lock()
                    .map_err(|_| "应用数据库锁已损坏".to_string())?;
                db::list_pending_example_indexes(&connection, 128)
            }
        })
        .await
        {
            Ok(Ok(keys)) => keys,
            Ok(Err(error)) => {
                diagnostics::error(format!(
                    "examples.index_backfill.list_failed reason={error}"
                ));
                return;
            }
            Err(error) => {
                diagnostics::error(format!(
                    "examples.index_backfill.list_worker_failed reason={error}"
                ));
                return;
            }
        };
        for cache_key in keys {
            index_example_cache(state.clone(), cache_key).await;
            tokio::task::yield_now().await;
        }
        state.startup.log("background.examples.completed", "main");
    });
}

fn schedule_example_index(state: &AppState, cache_key: &str) {
    let state = state.clone();
    let cache_key = cache_key.to_string();
    tauri::async_runtime::spawn(async move {
        index_example_cache(state, cache_key).await;
    });
}

async fn index_example_cache(state: AppState, cache_key: String) {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        db::index_translation_cache(&connection, &cache_key)
    })
    .await;
    match result {
        Ok(Ok(())) => diagnostics::info("examples.index.completed"),
        Ok(Err(error)) => diagnostics::error(format!("examples.index.failed reason={error}")),
        Err(error) => diagnostics::error(format!("examples.index.worker_failed reason={error}")),
    }
}

#[tauri::command]
fn upsert_glossary_term(
    state: State<'_, AppState>,
    source: String,
    target: String,
    note: Option<String>,
) -> Result<(), String> {
    let source = source.trim();
    let target = target.trim();
    if source.is_empty() || target.is_empty() {
        return Err("原文术语和译文不能为空".to_string());
    }
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::upsert_glossary_term(&connection, None, source, target, note.as_deref())
}

#[tauri::command]
fn delete_glossary_term(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::delete_glossary_term(&connection, &id)
}

#[tauri::command]
fn query_dictionary(
    state: State<'_, AppState>,
    word: String,
    canonical_word: Option<String>,
) -> Result<DictionaryLookupCommandResult, String> {
    let request_id = Uuid::new_v4().to_string();
    let started = Instant::now();
    let display_word = word.trim().to_string();
    diagnostics::info(format!(
        "command.dictionary.query.start request_id={request_id}"
    ));

    if display_word.is_empty() {
        let error = dictionary::DictionaryError::EmptyInput;
        diagnostics::error(format!(
            "command.dictionary.query.failed request_id={request_id} stage=input elapsed_ms={} error_kind={}",
            started.elapsed().as_millis(),
            error.kind()
        ));
        return Err(error.to_string());
    }

    let update_guard = state
        .dictionary_update
        .lock()
        .map_err(|_| "词典更新状态锁已损坏".to_string())?;
    if update_guard.is_some() {
        let error = dictionary::DictionaryError::Updating;
        diagnostics::error(format!(
            "command.dictionary.query.failed request_id={request_id} stage=update_guard elapsed_ms={} error_kind={}",
            started.elapsed().as_millis(),
            error.kind()
        ));
        return Err(error.to_string());
    }

    let resolution = {
        let mut store = state
            .dictionary_store
            .lock()
            .map_err(|_| "词典存储锁已损坏".to_string())?;
        if !store.is_runtime_ready() {
            store
                .open_runtime("first_query")
                .map_err(|error| error.to_string())?;
        }
        match store.resolve(&display_word, canonical_word.as_deref()) {
            Ok(resolution) => resolution,
            Err(error) => {
                diagnostics::error(format!(
                    "command.dictionary.query.failed request_id={request_id} stage=lookup elapsed_ms={} error_kind={}",
                    started.elapsed().as_millis(),
                    error.kind()
                ));
                return Err(error.to_string());
            }
        }
    };

    let list_history = || -> Result<Vec<contracts::DictionaryHistoryEntry>, String> {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        db::list_dictionary_history(&connection, DICTIONARY_HISTORY_LIMIT)
    };

    let (measurement, candidates) = match resolution {
        dictionary::DictionaryLookupResolution::Found(measurement) => {
            (Some(measurement), Vec::new())
        }
        dictionary::DictionaryLookupResolution::Ambiguous(candidates) => {
            let history = list_history()?;
            diagnostics::info(format!(
                "command.dictionary.query.ambiguous request_id={request_id} candidate_count={} total_ms={}",
                candidates.len(),
                started.elapsed().as_millis()
            ));
            return Ok(DictionaryLookupCommandResult {
                lookup: None,
                candidates: candidates
                    .into_iter()
                    .map(
                        |(canonical_word, normalized_canonical_word)| DictionaryLookupCandidate {
                            canonical_word,
                            normalized_canonical_word,
                        },
                    )
                    .collect(),
                example: None,
                history,
            });
        }
        dictionary::DictionaryLookupResolution::NotFound => {
            let history = list_history()?;
            diagnostics::info(format!(
                "command.dictionary.query.not_found request_id={request_id} total_ms={}",
                started.elapsed().as_millis()
            ));
            return Ok(DictionaryLookupCommandResult {
                lookup: None,
                candidates: Vec::new(),
                example: None,
                history,
            });
        }
    };
    let lookup = measurement.expect("dictionary resolution must contain a measurement");

    let history_started = Instant::now();
    let (history, example) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        db::record_dictionary_query(
            &connection,
            &lookup.result.normalized_word,
            &lookup.result.word,
        )?;
        let history = db::list_dictionary_history(&connection, DICTIONARY_HISTORY_LIMIT)?;
        let settings = db::get_settings(&connection)?;
        let example = if settings.paragraph_example_lookup_enabled {
            db::find_latest_example(&connection, &lookup.result.normalized_word)?.map(|record| {
                ParagraphExample {
                    example_id: record.example_id,
                    source_text: record.source_text,
                    created_at: record.created_at,
                }
            })
        } else {
            None
        };
        (history, example)
    };
    let history_elapsed = history_started.elapsed();
    diagnostics::info(format!(
        "command.dictionary.query.completed request_id={request_id} connection_source=reused sql_ms={} json_decode_ms={} history_ms={} snapshot_refresh=skipped total_ms={}",
        lookup.sql_elapsed.as_millis(),
        lookup.json_decode_elapsed.as_millis(),
        history_elapsed.as_millis(),
        started.elapsed().as_millis()
    ));
    Ok(DictionaryLookupCommandResult {
        lookup: Some(lookup.result),
        candidates,
        example,
        history,
    })
}

#[tauri::command]
async fn generate_word_example(
    app: AppHandle,
    state: State<'_, AppState>,
    request: WordExampleRequest,
) -> Result<WordExampleCommandResult, String> {
    generate_word_example_impl(app, state.inner().clone(), request).await
}

async fn generate_word_example_impl(
    app: AppHandle,
    state: AppState,
    request: WordExampleRequest,
) -> Result<WordExampleCommandResult, String> {
    let request_id = if request.request_id.trim().is_empty() {
        Uuid::new_v4().to_string()
    } else {
        request.request_id.clone()
    };
    let word = request.word.trim().to_string();
    let canonical_word = request.canonical_word.trim().to_string();
    let target_language = request.target_language.trim().to_string();
    if word.is_empty() {
        return emit_word_failed(&app, &request_id, "查询词形不能为空");
    }
    if canonical_word.is_empty() {
        return emit_word_failed(&app, &request_id, "规范词头不能为空");
    }
    if target_language.is_empty() || request.example_id <= 0 {
        return emit_word_failed(&app, &request_id, "例句请求参数无效");
    }

    diagnostics::info(format!(
        "command.word_example.start request_id={} example_id={} target_language={}",
        request_id, request.example_id, target_language
    ));
    let cancellation = CancellationToken::new();
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| "取消状态锁已损坏".to_string())?;
        cancellations.insert(request_id.clone(), cancellation.clone());
    }
    if let Err(error) = app.emit(
        "word_example_started",
        WordExampleStarted {
            request_id: request_id.clone(),
        },
    ) {
        unregister_request(&state, &request_id);
        return Err(format!("发送单词例句状态失败：{error}"));
    }

    let prepared = match prepare_word_example(
        &state,
        request.example_id,
        &word,
        &canonical_word,
        &target_language,
    ) {
        Ok(value) => value,
        Err(error) => {
            unregister_request(&state, &request_id);
            return emit_word_failed(&app, &request_id, &error);
        }
    };

    if prepared.cache_enabled {
        let cached = {
            let connection = state
                .database
                .lock()
                .map_err(|_| "应用数据库锁已损坏".to_string())?;
            match db::find_word_ai_cache(&connection, &prepared.cache_key) {
                Ok(value) => value,
                Err(error) => {
                    diagnostics::error(format!(
                        "command.word_example.cache_lookup_failed request_id={} reason={error}",
                        request_id
                    ));
                    drop(connection);
                    unregister_request(&state, &request_id);
                    return emit_word_failed(&app, &request_id, &error);
                }
            }
        };
        if let Some(cached) = cached {
            unregister_request(&state, &request_id);
            app.emit(
                "word_example_completed",
                WordExampleCompleted {
                    request_id: request_id.clone(),
                    translation: cached.translated_text.clone(),
                    part_of_speech: cached.part_of_speech.clone(),
                    cache_hit: true,
                },
            )
            .map_err(|error| format!("发送单词例句结果失败：{error}"))?;
            diagnostics::info(format!(
                "command.word_example.completed request_id={} cache_hit=true output_chars={}",
                request_id,
                cached.translated_text.chars().count()
            ));
            return Ok(WordExampleCommandResult::completed(
                cached.translated_text,
                cached.part_of_speech,
                true,
            ));
        }
    }

    let api_key = match secrets::load_api_key(&prepared.provider.id) {
        Ok(Some(value)) => value,
        Ok(None) => {
            unregister_request(&state, &request_id);
            return emit_word_failed(
                &app,
                &request_id,
                "尚未配置 API Key，请在设置中保存 Provider",
            );
        }
        Err(error) => {
            unregister_request(&state, &request_id);
            return emit_word_failed(&app, &request_id, &error);
        }
    };

    let system_prompt = build_word_example_prompt(
        &prepared.target_language,
        &prepared.glossary_terms,
        &prepared.example.source_text,
    );
    let user_text = format!(
        "规范词头：{}\n查询词形：{}\n英语例句：{}",
        prepared.canonical_word, prepared.word, prepared.example.source_text
    );
    let mut parser = WordExampleProtocolParser::default();
    let streamed = provider::stream_chat_completion(
        provider::ChatStreamRequest {
            request_id: &request_id,
            base_url: &prepared.provider.base_url,
            api_key: &api_key,
            model_id: &prepared.provider.model_id,
            system_prompt: &system_prompt,
            user_text: &user_text,
            cancel: &cancellation,
            operation: "word_example",
        },
        |content| {
            for delta in parser.push(&content)? {
                match delta {
                    WordExampleDelta::Translation(content) => app
                        .emit(
                            "word_example_translation_delta",
                            WordExampleTranslationDelta {
                                request_id: request_id.clone(),
                                content,
                            },
                        )
                        .map_err(|error| provider::ProviderError::Event(error.to_string()))?,
                    WordExampleDelta::Pos(content) => app
                        .emit(
                            "word_example_pos_delta",
                            WordExamplePosDelta {
                                request_id: request_id.clone(),
                                content,
                            },
                        )
                        .map_err(|error| provider::ProviderError::Event(error.to_string()))?,
                }
            }
            Ok(())
        },
    )
    .await;
    let streamed = match streamed {
        Ok(value) => value,
        Err(provider::ProviderError::Cancelled) => {
            unregister_request(&state, &request_id);
            app.emit(
                "word_example_cancelled",
                WordExampleCancelled {
                    request_id: request_id.clone(),
                },
            )
            .map_err(|error| format!("发送单词例句取消状态失败：{error}"))?;
            return Ok(WordExampleCommandResult::cancelled());
        }
        Err(error) => {
            unregister_request(&state, &request_id);
            return emit_word_failed(&app, &request_id, &error.to_string());
        }
    };
    let parsed = match parser.finish() {
        Ok(value) => value,
        Err(error) => {
            diagnostics::error(format!(
                "command.word_example.protocol_failed request_id={} raw_chars={} reason={error}",
                request_id,
                streamed.chars().count()
            ));
            unregister_request(&state, &request_id);
            return emit_word_failed(&app, &request_id, &error.to_string());
        }
    };

    let persistence = if prepared.cache_enabled {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let cache = db::WordAiCacheWrite {
            cache_key: &prepared.cache_key,
            example_id: prepared.example.example_id,
            normalized_word: &prepared.normalized_word,
            word: &prepared.word,
            canonical_word: &prepared.canonical_word,
            source_language: "en",
            target_language: &prepared.target_language,
            provider: &prepared.provider,
            prompt_id: &prepared.prompt.id,
            glossary_version: prepared.glossary_version,
            protocol_version: WORD_EXAMPLE_PROTOCOL_VERSION,
            translated_text: &parsed.translation,
            part_of_speech: &parsed.part_of_speech,
        };
        db::save_word_ai_cache(&connection, &cache)
            .and_then(|_| db::prune_cache(&connection, prepared.cache_max_bytes))
    } else {
        Ok(())
    };
    if let Err(error) = persistence {
        unregister_request(&state, &request_id);
        return emit_word_failed(&app, &request_id, &error);
    }

    unregister_request(&state, &request_id);
    app.emit(
        "word_example_completed",
        WordExampleCompleted {
            request_id: request_id.clone(),
            translation: parsed.translation.clone(),
            part_of_speech: parsed.part_of_speech.clone(),
            cache_hit: false,
        },
    )
    .map_err(|error| format!("发送单词例句结果失败：{error}"))?;
    diagnostics::info(format!(
        "command.word_example.completed request_id={} cache_hit=false output_chars={}",
        request_id,
        parsed.translation.chars().count()
    ));
    Ok(WordExampleCommandResult::completed(
        parsed.translation,
        parsed.part_of_speech,
        false,
    ))
}

struct PreparedWordExample {
    provider: contracts::ProviderRecord,
    prompt: Prompt,
    example: db::ParagraphExampleRecord,
    word: String,
    normalized_word: String,
    canonical_word: String,
    target_language: String,
    cache_key: String,
    cache_enabled: bool,
    cache_max_bytes: i64,
    glossary_version: i64,
    glossary_terms: Vec<GlossaryTerm>,
}

fn prepare_word_example(
    state: &AppState,
    example_id: i64,
    word: &str,
    canonical_word: &str,
    target_language: &str,
) -> Result<PreparedWordExample, String> {
    let normalized_word = word.to_lowercase();
    let (settings, provider, prompt, glossary_terms, glossary_version, example) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let settings = db::get_settings(&connection)?;
        let provider = db::get_provider(&connection)?;
        let prompt = db::get_prompt(&connection, &provider.prompt_id)?;
        let glossary_terms = db::list_glossary_terms(&connection)?;
        let glossary_version = db::glossary_version(&connection)?;
        let example = db::find_example_by_id_for_word(&connection, example_id, &normalized_word)?
            .ok_or_else(|| "例句索引已经失效，请重新查询词典".to_string())?;
        (
            settings,
            provider,
            prompt,
            glossary_terms,
            glossary_version,
            example,
        )
    };
    {
        let mut store = state
            .dictionary_store
            .lock()
            .map_err(|_| "词典存储锁已损坏".to_string())?;
        if !store.is_runtime_ready() {
            store
                .open_runtime("word_example")
                .map_err(|error| error.to_string())?;
        }
        let resolution = store
            .resolve(word, Some(canonical_word))
            .map_err(|error| error.to_string())?;
        let resolved_canonical = match resolution {
            dictionary::DictionaryLookupResolution::Found(measurement) => {
                measurement.result.canonical_word.to_lowercase()
            }
            dictionary::DictionaryLookupResolution::Ambiguous(_) => {
                return Err("规范词头选择不明确，请重新查询词典".to_string());
            }
            dictionary::DictionaryLookupResolution::NotFound => {
                return Err("查询词形已经不再存在，请重新查询词典".to_string());
            }
        };
        if resolved_canonical != canonical_word.to_lowercase() {
            return Err("规范词头与查询词形不匹配".to_string());
        }
    }
    let cache_key = make_word_ai_cache_key(&WordAiCacheKeyInput {
        base_url: &provider.base_url,
        provider_id: &provider.id,
        model_id: &provider.model_id,
        prompt_id: &prompt.id,
        prompt_version: prompt.version,
        glossary_version,
        example_id,
        source_text: &example.source_text,
        normalized_word: &normalized_word,
        canonical_word,
        target_language,
        protocol_version: WORD_EXAMPLE_PROTOCOL_VERSION,
    });
    Ok(PreparedWordExample {
        provider,
        prompt,
        example,
        word: word.to_string(),
        normalized_word,
        canonical_word: canonical_word.to_string(),
        target_language: target_language.to_string(),
        cache_key,
        cache_enabled: settings.word_ai_cache_enabled,
        cache_max_bytes: settings.cache_max_bytes,
        glossary_version,
        glossary_terms,
    })
}

fn build_word_example_prompt(
    target_language: &str,
    terms: &[GlossaryTerm],
    source_text: &str,
) -> String {
    let mut prompt = format!(
        "你是一名英语词典辅助工具。请处理用户提供的英语例句，只输出以下协议内容，不要输出解释、Markdown 或其他文字：<translation>例句译文</translation><pos>查询词在该句中的英文词性</pos>。目标语言代码是 {target_language}。词性使用简短英文标签，例如 noun、verb、adjective、adverb。"
    );
    let source_lower = source_text.to_lowercase();
    let glossary = terms
        .iter()
        .filter(|term| source_lower.contains(&term.source.to_lowercase()))
        .map(|term| format!("- {}：{}", term.source, term.target))
        .collect::<Vec<_>>();
    if !glossary.is_empty() {
        prompt.push_str(&format!(
            "\n翻译例句时遵循以下已命中的术语译法：\n{}",
            glossary.join("\n")
        ));
    }
    prompt
}

#[derive(Clone, Copy)]
struct WordAiCacheKeyInput<'a> {
    base_url: &'a str,
    provider_id: &'a str,
    model_id: &'a str,
    prompt_id: &'a str,
    prompt_version: i64,
    glossary_version: i64,
    example_id: i64,
    source_text: &'a str,
    normalized_word: &'a str,
    canonical_word: &'a str,
    target_language: &'a str,
    protocol_version: &'a str,
}

fn make_word_ai_cache_key(input: &WordAiCacheKeyInput<'_>) -> String {
    let canonical = format!(
        "base={}\nprovider={}\nmodel={}\nprompt={}@{}\nglossary={}\nexample={}\nsource={}\nword={}\ncanonical={}\ntarget_language={}\nprotocol={}",
        input.base_url,
        input.provider_id,
        input.model_id,
        input.prompt_id,
        input.prompt_version,
        input.glossary_version,
        input.example_id,
        input.source_text,
        input.normalized_word,
        input.canonical_word,
        input.target_language,
        input.protocol_version,
    );
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

#[derive(Default)]
struct WordExampleProtocolParser {
    buffer: String,
    section: WordExampleSection,
    translation: String,
    part_of_speech: String,
}

#[derive(Default, PartialEq, Eq)]
enum WordExampleSection {
    #[default]
    Waiting,
    Translation,
    Pos,
    Done,
}

enum WordExampleDelta {
    Translation(String),
    Pos(String),
}

impl WordExampleProtocolParser {
    fn push(&mut self, content: &str) -> Result<Vec<WordExampleDelta>, provider::ProviderError> {
        self.buffer.push_str(content);
        let mut deltas = Vec::new();
        loop {
            match self.section {
                WordExampleSection::Waiting => {
                    let translation_start = self.buffer.find("<translation>");
                    let pos_start = self.buffer.find("<pos>");
                    let next = match (translation_start, pos_start) {
                        (Some(left), Some(right)) if left <= right => Some((left, true)),
                        (Some(left), _) => Some((left, true)),
                        (_, Some(right)) => Some((right, false)),
                        (None, None) => None,
                    };
                    let Some((position, is_translation)) = next else {
                        let keep =
                            longest_tag_prefix_suffix(&self.buffer, &["<translation>", "<pos>"]);
                        let discard = self.buffer.len().saturating_sub(keep);
                        if !self.buffer[..discard].trim().is_empty() {
                            return Err(provider::ProviderError::Protocol(
                                "单词例句协议包含未标记内容".to_string(),
                            ));
                        }
                        discard_prefix(&mut self.buffer, discard);
                        break;
                    };
                    if !self.buffer[..position].trim().is_empty() {
                        return Err(provider::ProviderError::Protocol(
                            "单词例句协议包含未标记内容".to_string(),
                        ));
                    }
                    discard_prefix(&mut self.buffer, position);
                    if is_translation {
                        discard_prefix(&mut self.buffer, "<translation>".len());
                        self.section = WordExampleSection::Translation;
                    } else {
                        discard_prefix(&mut self.buffer, "<pos>".len());
                        self.section = WordExampleSection::Pos;
                    }
                }
                WordExampleSection::Translation => {
                    if let Some(position) = self.buffer.find("</translation>") {
                        let piece = self.buffer[..position].to_string();
                        discard_prefix(&mut self.buffer, position + "</translation>".len());
                        append_word_example_piece(&mut self.translation, piece, &mut deltas, true);
                        self.section = WordExampleSection::Waiting;
                        continue;
                    }
                    let flush_length = flushable_protocol_length(&self.buffer, "</translation>");
                    if flush_length == 0 {
                        break;
                    }
                    let piece = self.buffer[..flush_length].to_string();
                    discard_prefix(&mut self.buffer, flush_length);
                    append_word_example_piece(&mut self.translation, piece, &mut deltas, true);
                }
                WordExampleSection::Pos => {
                    if let Some(position) = self.buffer.find("</pos>") {
                        let piece = self.buffer[..position].to_string();
                        discard_prefix(&mut self.buffer, position + "</pos>".len());
                        append_word_example_piece(
                            &mut self.part_of_speech,
                            piece,
                            &mut deltas,
                            false,
                        );
                        self.section = WordExampleSection::Done;
                        continue;
                    }
                    let flush_length = flushable_protocol_length(&self.buffer, "</pos>");
                    if flush_length == 0 {
                        break;
                    }
                    let piece = self.buffer[..flush_length].to_string();
                    discard_prefix(&mut self.buffer, flush_length);
                    append_word_example_piece(&mut self.part_of_speech, piece, &mut deltas, false);
                }
                WordExampleSection::Done => {
                    if !self.buffer.trim().is_empty() {
                        return Err(provider::ProviderError::Protocol(
                            "单词例句协议包含结束标签后的多余内容".to_string(),
                        ));
                    }
                    self.buffer.clear();
                    break;
                }
            }
        }
        Ok(deltas)
    }

    fn finish(mut self) -> Result<ParsedWordExample, provider::ProviderError> {
        self.push("")?;
        if !matches!(self.section, WordExampleSection::Done) {
            return Err(provider::ProviderError::Protocol(
                "单词例句协议缺少完整标签".to_string(),
            ));
        }
        let translation = self.translation.trim().to_string();
        let part_of_speech = self.part_of_speech.trim().to_string();
        if translation.is_empty() || part_of_speech.is_empty() {
            return Err(provider::ProviderError::Protocol(
                "单词例句协议返回了空字段".to_string(),
            ));
        }
        Ok(ParsedWordExample {
            translation,
            part_of_speech,
        })
    }
}

struct ParsedWordExample {
    translation: String,
    part_of_speech: String,
}

fn append_word_example_piece(
    target: &mut String,
    piece: String,
    deltas: &mut Vec<WordExampleDelta>,
    translation: bool,
) {
    if piece.is_empty() {
        return;
    }
    target.push_str(&piece);
    if translation {
        deltas.push(WordExampleDelta::Translation(piece));
    } else {
        deltas.push(WordExampleDelta::Pos(piece));
    }
}

fn discard_prefix(value: &mut String, length: usize) {
    value.drain(..length.min(value.len()));
}

fn longest_tag_prefix_suffix(value: &str, tags: &[&str]) -> usize {
    tags.iter()
        .flat_map(|tag| (1..=tag.len().min(value.len())).map(move |length| (*tag, length)))
        .filter(|(tag, length)| value.ends_with(&tag[..*length]))
        .map(|(_, length)| length)
        .max()
        .unwrap_or(0)
}

fn flushable_protocol_length(value: &str, closing_tag: &str) -> usize {
    value
        .len()
        .saturating_sub(longest_tag_prefix_suffix(value, &[closing_tag]))
}

#[tauri::command]
fn get_dictionary_state(state: State<'_, AppState>) -> Result<DictionaryState, String> {
    let installation = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        db::get_dictionary_installation(&connection)?
    };
    let mut dictionary = if state.dictionary_initialising.load(Ordering::Acquire) {
        dictionary::deferred_state(&state.dictionary_dir(), installation.as_ref())
    } else {
        state
            .dictionary_store
            .lock()
            .map_err(|_| "词典存储锁已损坏".to_string())?
            .state(installation.as_ref())
    };
    if state
        .dictionary_update
        .lock()
        .map_err(|_| "词典更新状态锁已损坏".to_string())?
        .is_some()
    {
        dictionary.status = contracts::DictionaryStatus::Updating;
        dictionary.error = None;
    }
    Ok(dictionary)
}

#[tauri::command]
async fn update_dictionary(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DictionaryCommandResult, String> {
    let operation_id = Uuid::new_v4().to_string();
    {
        let mut current = state
            .dictionary_update
            .lock()
            .map_err(|_| "词典更新状态锁已损坏".to_string())?;
        if current.is_some() {
            return Err("词典更新已经在进行中，请稍候".to_string());
        }
        *current = Some(operation_id.clone());
    }

    let result =
        dictionary::update_dictionary(app, state.inner(), &state.dictionary_dir(), operation_id)
            .await;
    if let Ok(mut current) = state.dictionary_update.lock() {
        *current = None;
    }
    result
}

#[tauri::command]
fn clear_dictionary_history(state: State<'_, AppState>) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::clear_dictionary_history(&connection)
}

fn unregister_request(state: &AppState, request_id: &str) {
    if let Ok(mut cancellations) = state.cancellations.lock() {
        cancellations.remove(request_id);
    }
}

fn emit_failed(
    app: &AppHandle,
    request_id: &str,
    message: &str,
) -> Result<TranslationCommandResult, String> {
    diagnostics::error(format!(
        "command.translate.failed request_id={} reason={message}",
        request_id
    ));
    app.emit(
        "translation_failed",
        TranslationFailed {
            request_id: request_id.to_string(),
            message: message.to_string(),
        },
    )
    .map_err(|error| format!("发送翻译错误失败：{error}"))?;
    Ok(TranslationCommandResult::failed(message))
}

fn emit_word_failed(
    app: &AppHandle,
    request_id: &str,
    message: &str,
) -> Result<WordExampleCommandResult, String> {
    diagnostics::error(format!(
        "command.word_example.failed request_id={} reason={message}",
        request_id
    ));
    app.emit(
        "word_example_failed",
        WordExampleFailed {
            request_id: request_id.to_string(),
            message: message.to_string(),
        },
    )
    .map_err(|error| format!("发送单词例句错误失败：{error}"))?;
    Ok(WordExampleCommandResult::failed(message))
}

#[cfg(test)]
mod tests {
    use super::{
        build_system_prompt, cancel_request, make_cache_key, make_word_ai_cache_key,
        unregister_request, AppState, CacheKeyInput, StartupRuntime, WordAiCacheKeyInput,
        WordExampleDelta, WordExampleProtocolParser, WORD_EXAMPLE_PROTOCOL_VERSION,
    };
    use crate::contracts::GlossaryTerm;
    use rusqlite::Connection;
    use std::sync::{atomic::Ordering, Arc};
    use tokio_util::sync::CancellationToken;

    fn test_cache_key(source_text: &str) -> String {
        make_cache_key(&CacheKeyInput {
            base_url: "https://example.com/v1",
            provider_id: "default",
            model_id: "model-a",
            prompt_id: "prompt",
            prompt_version: 1,
            glossary_version: 1,
            source_language: "en",
            target_language: "zh-CN",
            source_text,
        })
    }

    #[test]
    fn cache_key_changes_when_translation_inputs_change() {
        let first = test_cache_key("hello");
        let second = test_cache_key("hello!");
        assert_ne!(first, second);
    }

    #[test]
    fn cache_key_does_not_include_api_key() {
        let first = test_cache_key("hello");
        assert!(!first.contains("api"));
    }

    #[test]
    fn word_example_cache_key_changes_with_example_content_and_target() {
        let first_input = WordAiCacheKeyInput {
            base_url: "https://example.com/v1",
            provider_id: "default",
            model_id: "model-a",
            prompt_id: "prompt",
            prompt_version: 1,
            glossary_version: 1,
            example_id: 7,
            source_text: "A target example.",
            normalized_word: "target",
            canonical_word: "target",
            target_language: "zh-CN",
            protocol_version: WORD_EXAMPLE_PROTOCOL_VERSION,
        };
        let changed_source_input = WordAiCacheKeyInput {
            source_text: "Another target example.",
            ..first_input
        };
        let changed_target_input = WordAiCacheKeyInput {
            target_language: "ja",
            ..first_input
        };
        let first = make_word_ai_cache_key(&first_input);
        let changed_source = make_word_ai_cache_key(&changed_source_input);
        let changed_target = make_word_ai_cache_key(&changed_target_input);
        assert_ne!(first, changed_source);
        assert_ne!(first, changed_target);
        assert!(!first.contains("secret"));
    }

    #[test]
    fn glossary_prompt_only_contains_matching_terms() {
        let terms = vec![
            GlossaryTerm {
                id: "1".into(),
                source: "embedding".into(),
                target: "嵌入".into(),
                note: None,
            },
            GlossaryTerm {
                id: "2".into(),
                source: "unmatched".into(),
                target: "不应出现".into(),
                note: None,
            },
        ];
        let prompt = build_system_prompt("base", &terms, "An embedding model");
        assert!(prompt.contains("embedding：嵌入"));
        assert!(!prompt.contains("不应出现"));
    }

    #[test]
    fn cancel_request_reports_active_token_and_cancels_it() {
        let state = AppState::new(
            Connection::open_in_memory().unwrap(),
            std::env::temp_dir(),
            Arc::new(StartupRuntime::new()),
        );
        let token = CancellationToken::new();
        state
            .cancellations
            .lock()
            .unwrap()
            .insert("active".to_string(), token.clone());

        assert!(cancel_request(&state, "active").unwrap());
        assert!(token.is_cancelled());

        unregister_request(&state, "active");
        assert!(!cancel_request(&state, "active").unwrap());
    }

    #[test]
    fn startup_stage_report_accepts_only_known_stages_and_opens_gate_at_first_paint() {
        let startup = StartupRuntime::new();
        assert!(startup.report_frontend_stage("dom_mounted", "main").is_ok());
        assert!(!startup.gate.first_paint.load(Ordering::Acquire));
        assert!(startup.report_frontend_stage("first_paint", "main").is_ok());
        assert!(startup.gate.first_paint.load(Ordering::Acquire));
        assert!(startup.report_frontend_stage("unexpected", "main").is_err());
    }

    #[test]
    fn word_example_protocol_parser_handles_split_tags_and_emits_sections() {
        let mut parser = WordExampleProtocolParser::default();
        let mut translation = String::new();
        let mut part_of_speech = String::new();
        for chunk in [
            "<trans",
            "lation>译",
            "文</trans",
            "lation><pos>ver",
            "b</pos>",
        ] {
            for delta in parser.push(chunk).unwrap() {
                match delta {
                    WordExampleDelta::Translation(value) => translation.push_str(&value),
                    WordExampleDelta::Pos(value) => part_of_speech.push_str(&value),
                }
            }
        }
        let parsed = parser.finish().unwrap();
        assert_eq!(translation, "译文");
        assert_eq!(part_of_speech, "verb");
        assert_eq!(parsed.translation, "译文");
        assert_eq!(parsed.part_of_speech, "verb");
    }

    #[test]
    fn word_example_protocol_parser_rejects_unstructured_output() {
        let mut parser = WordExampleProtocolParser::default();
        assert!(parser.push("普通文本").is_err());
    }
}
