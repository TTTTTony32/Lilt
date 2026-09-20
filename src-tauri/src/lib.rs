mod contracts;
mod db;
mod diagnostics;
mod dictionary;
mod examples;
mod glossary;
mod icons;
mod paragraph_learning;
mod pdf;
mod pdf_context;
mod pdf_engine;
mod pdf_jobs;
pub mod pdf_protocol;
pub mod pdf_worker;
mod provider;
mod secrets;
mod selection;
mod translation_core;
#[cfg(desktop)]
mod tray;

use contracts::{
    AI_DICTIONARY_PROTOCOL_VERSION, AppSnapshot, CloseBehavior, DEFAULT_PROVIDER_ID,
    DICTIONARY_HISTORY_LIMIT, DictionaryCommandResult, DictionaryInvalidInsight,
    DictionaryLookupCandidate, DictionaryLookupCommandResult, DictionaryMatchType,
    DictionaryProperNounInsight, DictionarySource, DictionaryState, GlossaryExportResult,
    GlossaryImportPreview, GlossaryImportPreviewTerm, GlossaryImportResult, GlossaryTerm,
    MAX_SELECTION_WINDOW_HEIGHT, MAX_SELECTION_WINDOW_WIDTH, MIN_SELECTION_WINDOW_HEIGHT,
    MIN_SELECTION_WINDOW_WIDTH, ModelInfo, ParagraphExample, PersonalDictionaryEntry,
    PersonalDictionaryExportResult, Prompt, ProviderConfig, SelectionMode, SelectionRequestPayload,
    SelectionRuntimeStatus, SelectionSettingsResult, SelectionTriggerNotice, ThinkingEffort,
    TranslationCancelled, TranslationCommandResult, TranslationCompleted, TranslationDelta,
    TranslationFailed, TranslationRequest, TranslationStarted, WORD_EXAMPLE_PROTOCOL_VERSION,
    WordExampleCancelled, WordExampleCommandResult, WordExampleCompleted, WordExampleFailed,
    WordExamplePosDelta, WordExampleRequest, WordExampleStarted, WordExampleTranslationDelta,
    clamp_selection_window_dimension,
};
use rusqlite::Connection;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewWindow, WindowEvent};
use tokio_util::sync::CancellationToken;
use translation_core::{StreamRequest as CoreStreamRequest, TranslationCore, TranslationMode};
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
    main_visible: AtomicBool,
}

impl StartupRuntime {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            gate: StartupGate::new(),
            main_visible: AtomicBool::new(false),
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
        Ok(())
    }

    fn ensure_main_visible(&self, window: &WebviewWindow) -> Result<(), String> {
        if !self.gate.first_paint.load(Ordering::Acquire) {
            return Err("主窗口尚未完成首屏绘制".to_string());
        }
        window
            .show()
            .map_err(|error| format!("显示主窗口失败：{error}"))?;
        window
            .set_focus()
            .map_err(|error| format!("聚焦主窗口失败：{error}"))
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) database: Arc<Mutex<Connection>>,
    pub(crate) cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) pdf_engine_preparing: Arc<AtomicBool>,
    pub(crate) pdf_engine_transition: Arc<Mutex<()>>,
    pub(crate) dictionary_update: Arc<Mutex<Option<String>>>,
    pub(crate) dictionary_store: Arc<Mutex<dictionary::DictionaryStore>>,
    pub(crate) dictionary_initialising: Arc<AtomicBool>,
    pub(crate) ai_dictionary_requests: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
    pub(crate) selection: selection::SelectionService,
    pub(crate) pdf_jobs: Arc<Mutex<HashMap<String, pdf_jobs::PdfJobHandle>>>,
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
            pdf_engine_preparing: Arc::new(AtomicBool::new(false)),
            pdf_engine_transition: Arc::new(Mutex::new(())),
            dictionary_update: Arc::new(Mutex::new(None)),
            dictionary_store: Arc::new(Mutex::new(dictionary_store)),
            dictionary_initialising: Arc::new(AtomicBool::new(false)),
            ai_dictionary_requests: Arc::new(Mutex::new(HashMap::new())),
            selection,
            pdf_jobs: Arc::new(Mutex::new(HashMap::new())),
            startup,
        }
    }

    fn dictionary_dir(&self) -> PathBuf {
        self.data_dir.join("dictionary")
    }
}

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build());
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    let app = builder
        .on_window_event(|window, event| {
            if window.label() == "selection" {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    match event {
                        WindowEvent::Focused(true) => state.selection.handle_focus_gained(),
                        WindowEvent::Focused(false) => state.selection.handle_focus_lost(),
                        WindowEvent::Resized(_) => state.selection.handle_window_resized(),
                        _ => {}
                    }
                }
            }
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let Some(state) = window.app_handle().try_state::<AppState>() else {
                        diagnostics::error("window.close.failed reason=app_state_unavailable");
                        return;
                    };
                    let behavior = match state.database.lock() {
                        Ok(connection) => match db::get_settings(&connection) {
                            Ok(settings) => settings.close_behavior,
                            Err(error) => {
                                diagnostics::error(format!(
                                    "window.close.failed reason=settings_read_failed error={error}"
                                ));
                                return;
                            }
                        },
                        Err(_) => {
                            diagnostics::error("window.close.failed reason=database_lock_failed");
                            return;
                        }
                    };
                    match behavior {
                        CloseBehavior::Exit => window.app_handle().exit(0),
                        CloseBehavior::Tray => {
                            if let Err(error) = window.hide() {
                                diagnostics::error(format!(
                                    "window.close.tray_failed error={error}"
                                ));
                            }
                        }
                        CloseBehavior::Ask => {
                            if let Err(error) = window.emit("window_close_requested", ()) {
                                diagnostics::error(format!(
                                    "window.close.dialog_event_failed error={error}"
                                ));
                            }
                        }
                    }
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
            let (
                selection_mode,
                selection_shortcut,
                selection_window_width,
                selection_window_height,
            ) = {
                let connection = state
                    .database
                    .lock()
                    .map_err(|_| std::io::Error::other("应用数据库锁已损坏"))?;
                let settings = db::get_settings(&connection).map_err(std::io::Error::other)?;
                (
                    settings.selection_mode,
                    settings.selection_shortcut,
                    settings.selection_window_width,
                    settings.selection_window_height,
                )
            };
            state.selection.attach_app(app.handle().clone());
            state
                .selection
                .set_window_size(selection_window_width, selection_window_height);
            app.manage(state.clone());
            #[cfg(windows)]
            {
                diagnostics::info("updater.plugin.ready");
            }
            #[cfg(windows)]
            if let Some(main_window) = app.get_webview_window("main") {
                if let Err(error) = set_main_taskbar_icon(&main_window) {
                    diagnostics::warn(format!("window.icon.taskbar_failed reason={error}"));
                } else {
                    diagnostics::info("window.icon.taskbar_ready source=256px_png");
                }
            }
            #[cfg(desktop)]
            {
                tray::init_tray(app.handle()).map_err(|error| {
                    std::io::Error::other(format!("初始化系统托盘失败：{error}"))
                })?;
                tray::register_menu_handler(app.handle());
            }
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
            clear_cache,
            set_pdf_preflight_enabled,
            set_paragraph_learning_mode,
            configure_selection,
            get_selection_status,
            set_selection_language,
            selection_window_ready,
            activate_selection,
            begin_selection_drag,
            end_selection_drag,
            save_selection_window_size,
            get_selection_request,
            open_selection_in_main,
            dismiss_selection,
            translate,
            cancel_translation,
            upsert_glossary_term,
            delete_glossary_term,
            preview_glossary,
            import_glossary,
            export_glossary,
            create_prompt,
            update_prompt,
            delete_prompt,
            set_default_prompt,
            save_personal_word,
            remove_personal_word,
            export_personal_dictionary,
            resolve_window_close,
            reset_close_behavior,
            query_dictionary,
            generate_word_example,
            cancel_word_example,
            get_dictionary_state,
            update_dictionary,
            clear_dictionary_history,
            pdf::read_pdf_bytes,
            pdf::reveal_pdf_file,
            pdf_engine::get_pdf_engine_status,
            pdf_engine::prepare_pdf_engine,
            pdf_jobs::start_pdf_translation,
            pdf_jobs::cancel_pdf_translation,
            prepare_for_updater_exit,
            recover_after_updater_failure,
            set_main_taskbar_progress,
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

#[tauri::command]
fn prepare_for_updater_exit(app: AppHandle) -> Result<(), String> {
    // Update.install invokes the official updater's on_before_exit hook immediately
    // before launching the installer. Release app resources before that hook exits us.
    #[cfg(windows)]
    cleanup_before_update(&app);
    #[cfg(not(windows))]
    let _ = app;
    Ok(())
}

#[tauri::command]
fn recover_after_updater_failure(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    if let Some(state) = app.try_state::<AppState>() {
        // The updater preparation is deliberately reversible when download or
        // signature verification fails before Windows launches the installer.
        state.selection.start_worker();
        diagnostics::warn("updater.failure.recovered selection_worker_restarted");
    }
    #[cfg(not(windows))]
    let _ = app;
    Ok(())
}

#[cfg(windows)]
fn cleanup_before_update(app: &AppHandle) {
    diagnostics::info("updater.before_exit.cleanup.begin");
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut cancellations) = state.cancellations.lock() {
            for cancellation in cancellations.values() {
                cancellation.cancel();
            }
            cancellations.clear();
        }
        pdf_jobs::shutdown_for_update(state.inner());
        state.selection.shutdown();
    }

    if let Some(main_window) = app.get_webview_window("main") {
        if let Err(error) = set_windows_taskbar_progress(&main_window, "none", 0.0) {
            diagnostics::warn(format!(
                "updater.before_exit.taskbar_cleanup_failed error={error}"
            ));
        }
    }
    if let Some(selection_window) = app.get_webview_window("selection") {
        let _ = selection_window.close();
    }
    diagnostics::info("updater.before_exit.cleanup.completed");
}

#[cfg(windows)]
fn set_main_taskbar_icon(window: &WebviewWindow) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{CreateIcon, ICON_BIG, SendMessageW, WM_SETICON},
    };

    let image = icons::high_resolution_icon().map_err(|error| error.to_string())?;
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(format!("默认窗口图标尺寸无效：{width}x{height}"));
    }

    let mut bgra = image.rgba().to_vec();
    if bgra.len() != (width as usize) * (height as usize) * 4 {
        return Err("默认窗口图标像素数据长度无效".to_string());
    }
    let mut and_mask = Vec::with_capacity((width as usize) * (height as usize));
    for pixel in bgra.chunks_exact_mut(4) {
        and_mask.push(pixel[3].wrapping_sub(u8::MAX));
        pixel.swap(0, 2);
    }

    let icon = unsafe {
        CreateIcon(
            None,
            width as i32,
            height as i32,
            1,
            32,
            and_mask.as_ptr(),
            bgra.as_ptr(),
        )
    }
    .map_err(|error| format!("创建任务栏图标失败：{error}"))?;
    let hwnd = window
        .hwnd()
        .map_err(|error| format!("获取主窗口句柄失败：{error}"))?;

    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
    }

    // WM_SETICON 不会复制 HICON。Windows 句柄需保持到进程结束，避免任务栏重绘时失效。
    let _ = icon;
    Ok(())
}

#[tauri::command]
fn set_main_taskbar_progress(
    window: WebviewWindow,
    state: String,
    value: f64,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("任务栏进度只允许主窗口设置".to_string());
    }

    #[cfg(windows)]
    {
        set_windows_taskbar_progress(&window, &state, value)
    }

    #[cfg(not(windows))]
    {
        let _ = (state, value);
        Ok(())
    }
}

#[cfg(windows)]
fn set_windows_taskbar_progress(
    window: &WebviewWindow,
    state: &str,
    value: f64,
) -> Result<(), String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        ITaskbarList3, TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL,
    };

    let hwnd = window
        .hwnd()
        .map_err(|error| format!("获取主窗口句柄失败：{error}"))?;
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    const RPC_E_CHANGED_MODE: i32 = -2_147_417_850;
    if init.is_err() && init.0 != RPC_E_CHANGED_MODE {
        return Err(format!("初始化 Windows COM 失败：0x{:08X}", init.0 as u32));
    }
    let should_uninitialize = init.is_ok();

    let result = (|| {
        let taskbar_clsid = windows::core::GUID::from_u128(0x56fdf344_fd6d_11d0_958a_006097c9a090);
        let taskbar: ITaskbarList3 =
            unsafe { CoCreateInstance(&taskbar_clsid, None, CLSCTX_INPROC_SERVER) }
                .map_err(|error| format!("创建 Windows 任务栏对象失败：{error}"))?;
        unsafe { taskbar.HrInit() }
            .map_err(|error| format!("初始化 Windows 任务栏对象失败：{error}"))?;

        let flag = match state {
            "none" => TBPF_NOPROGRESS,
            "indeterminate" => TBPF_INDETERMINATE,
            "normal" => TBPF_NORMAL,
            "error" => TBPF_ERROR,
            _ => return Err("无效的任务栏进度状态".to_string()),
        };
        unsafe { taskbar.SetProgressState(hwnd, flag) }
            .map_err(|error| format!("设置 Windows 任务栏进度状态失败：{error}"))?;
        if state == "normal" {
            let completed = (value.clamp(0.0, 100.0) * 100.0).round() as u64;
            unsafe { taskbar.SetProgressValue(hwnd, completed, 10_000) }
                .map_err(|error| format!("设置 Windows 任务栏进度值失败：{error}"))?;
        }
        Ok(())
    })();

    if should_uninitialize {
        unsafe { CoUninitialize() };
    }
    result
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
    state
        .startup
        .report_frontend_stage(&stage, window.label())?;
    if stage == "first_paint"
        && state
            .startup
            .main_visible
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        if let Err(error) = window.show() {
            state.startup.main_visible.store(false, Ordering::Release);
            diagnostics::error(format!("startup.main.visible_failed reason={error}"));
            return Err(format!("显示主窗口失败：{error}"));
        }
        state.startup.log("main.visible", window.label());
        state.startup.gate.open();
    }
    Ok(())
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
        personal_dictionary: snapshot_data.personal_dictionary,
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
    personal_dictionary: Vec<PersonalDictionaryEntry>,
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
    let personal_dictionary = db::list_personal_dictionary(connection)?;
    Ok(SnapshotData {
        settings,
        provider: ProviderConfig {
            id: provider.id,
            name: provider.name,
            base_url: provider.base_url,
            model_id: provider.model_id,
            prompt_id: provider.prompt_id,
            thinking_effort: provider.thinking_effort,
            has_api_key,
        },
        models,
        prompts,
        glossary_terms,
        history,
        cache_stats,
        dictionary_installation,
        dictionary_history,
        personal_dictionary,
    })
}

#[tauri::command]
fn save_provider_config(
    state: State<'_, AppState>,
    base_url: String,
    model_id: String,
    thinking_effort: ThinkingEffort,
    api_key: Option<String>,
) -> Result<(), String> {
    let normalized_url =
        provider::normalize_base_url(&base_url).map_err(|error| error.to_string())?;
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err("Model ID 不能为空".to_string());
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
    db::save_provider(&connection, &normalized_url, model_id, thinking_effort)
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
    pdf_preflight_page_limit: i64,
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
        pdf_preflight_page_limit,
    )
}

#[tauri::command]
fn clear_cache(state: State<'_, AppState>) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::clear_cache(&connection)
}

#[tauri::command]
fn set_pdf_preflight_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::save_pdf_preflight_enabled(&connection, enabled)
}

#[tauri::command]
fn set_paragraph_learning_mode(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::save_paragraph_learning_mode(&connection, enabled)
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
fn selection_window_ready(
    state: State<'_, AppState>,
) -> Result<Option<SelectionTriggerNotice>, String> {
    Ok(state.selection.window_ready())
}

#[tauri::command]
fn activate_selection(state: State<'_, AppState>, trigger_id: String) -> Result<(), String> {
    state.selection.activate_trigger(&trigger_id)
}

#[tauri::command]
fn begin_selection_drag(state: State<'_, AppState>) -> Result<(), String> {
    state.selection.begin_drag();
    Ok(())
}

#[tauri::command]
fn end_selection_drag(state: State<'_, AppState>) -> Result<(), String> {
    state.selection.end_drag();
    Ok(())
}

#[tauri::command]
fn save_selection_window_size(
    state: State<'_, AppState>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let width = clamp_selection_window_dimension(
        width,
        contracts::DEFAULT_SELECTION_WINDOW_WIDTH,
        MIN_SELECTION_WINDOW_WIDTH,
        MAX_SELECTION_WINDOW_WIDTH,
    );
    let height = clamp_selection_window_dimension(
        height,
        contracts::DEFAULT_SELECTION_WINDOW_HEIGHT,
        MIN_SELECTION_WINDOW_HEIGHT,
        MAX_SELECTION_WINDOW_HEIGHT,
    );
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::save_selection_window_size(&connection, width, height)?;
    state.selection.set_window_size(width, height);
    Ok(())
}

#[tauri::command]
fn get_selection_request(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<SelectionRequestPayload, String> {
    state.selection.get_request(&request_id)
}

#[tauri::command]
fn open_selection_in_main(
    app: AppHandle,
    state: State<'_, AppState>,
    request_id: String,
) -> Result<(), String> {
    state.selection.get_request(&request_id)?;
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口尚未创建".to_string())?;
    state.startup.ensure_main_visible(&main)?;
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
            let parsed_cache = if prepared.learning_mode {
                match paragraph_learning::parse_cached_result(&source_text, &cached.translated_text)
                {
                    Ok(learning) => Some((learning.translated_text(), Some(learning))),
                    Err(error) => {
                        diagnostics::warn(format!(
                            "command.translate.learning_cache_invalid request_id={} reason={error}",
                            request_id
                        ));
                        let deletion = state
                            .database
                            .lock()
                            .map_err(|_| "应用数据库锁已损坏".to_string())
                            .and_then(|connection| {
                                db::delete_cache(&connection, &prepared.cache_key)
                            });
                        if let Err(error) = deletion {
                            unregister_request(&state, &request_id);
                            return emit_failed(&app, &request_id, &error);
                        }
                        None
                    }
                }
            } else {
                Some((cached.translated_text, None))
            };

            if let Some((content, learning)) = parsed_cache {
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
                        translated_text: &content,
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
                schedule_example_index(&state, &prepared.cache_key);
                app.emit(
                    "translation_completed",
                    TranslationCompleted {
                        request_id: request_id.clone(),
                        content: content.clone(),
                        cache_hit: true,
                        learning: learning.clone(),
                    },
                )
                .map_err(|error| format!("发送翻译结果失败：{error}"))?;
                diagnostics::info(format!(
                    "command.translate.completed request_id={} cache_hit=true",
                    request_id
                ));
                return Ok(match learning {
                    Some(learning) => {
                        TranslationCommandResult::completed_with_learning(content, true, learning)
                    }
                    None => TranslationCommandResult::completed(content, true),
                });
            }
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
    let learning_mode = prepared.learning_mode;
    let translated = TranslationCore::stream(
        CoreStreamRequest {
            request_id: &request_id,
            base_url: &prepared.provider.base_url,
            api_key: &api_key,
            model_id: &prepared.provider.model_id,
            system_prompt: &prepared.system_prompt,
            user_text: &source_text,
            cancel: &cancellation,
            mode: TranslationMode::Paragraph,
            thinking_effort: &prepared.provider.thinking_effort,
        },
        |content| {
            if learning_mode {
                return Ok(());
            }
            app.emit(
                "translation_delta",
                TranslationDelta {
                    request_id: request_id.clone(),
                    content,
                },
            )
            .map_err(|error| provider::ProviderError::Event(error.to_string()))
        },
    )
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

    let (content, learning, cache_text) = if prepared.learning_mode {
        let learning = match paragraph_learning::parse_model_output(&source_text, &translated) {
            Ok(value) => value,
            Err(error) => {
                diagnostics::error(format!(
                    "command.translate.learning_protocol_failed request_id={} reason={error}",
                    request_id
                ));
                return emit_failed(&app, &request_id, &error);
            }
        };
        let content = learning.translated_text();
        let cache_text = match learning.to_cache_json() {
            Ok(value) => value,
            Err(error) => return emit_failed(&app, &request_id, &error),
        };
        (content, Some(learning), cache_text)
    } else {
        (translated.clone(), None, translated)
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
                translated_text: &cache_text,
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
                translated_text: &content,
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
    let output_chars = content.chars().count();
    app.emit(
        "translation_completed",
        TranslationCompleted {
            request_id: request_id.clone(),
            content: content.clone(),
            cache_hit: false,
            learning: learning.clone(),
        },
    )
    .map_err(|error| format!("发送翻译结果失败：{error}"))?;
    diagnostics::info(format!(
        "command.translate.completed request_id={} cache_hit=false output_chars={}",
        request_id, output_chars
    ));
    Ok(match learning {
        Some(learning) => {
            TranslationCommandResult::completed_with_learning(content, false, learning)
        }
        None => TranslationCommandResult::completed(content, false),
    })
}

pub(crate) struct PreparedTranslation {
    pub(crate) provider: contracts::ProviderRecord,
    pub(crate) prompt: Prompt,
    pub(crate) glossary_terms: Vec<GlossaryTerm>,
    pub(crate) system_prompt: String,
    pub(crate) cache_key: String,
    pub(crate) cache_enabled: bool,
    pub(crate) learning_mode: bool,
    pub(crate) cache_max_bytes: i64,
    pub(crate) history_retention: i64,
    pub(crate) glossary_version: i64,
}

pub(crate) fn prepare_translation(
    state: &AppState,
    request: &TranslationRequest,
    source_text: &str,
) -> Result<PreparedTranslation, String> {
    prepare_translation_internal(state, request, source_text, None)
}

pub(crate) fn prepare_pdf_translation(
    state: &AppState,
    request: &TranslationRequest,
    source_text: &str,
    pdf_context: &PdfPromptContext,
) -> Result<PreparedTranslation, String> {
    prepare_translation_internal(state, request, source_text, Some(pdf_context))
}

pub(crate) struct PdfPromptContext {
    mode: TranslationMode,
    document_context: serde_json::Value,
    context_before: serde_json::Value,
    context_after: serde_json::Value,
    task_terms: serde_json::Value,
    abbreviations: serde_json::Value,
    engine_constraints: serde_json::Value,
}

impl PdfPromptContext {
    pub(crate) fn new(
        mode: TranslationMode,
        document_context: &serde_json::Value,
        context_before: &serde_json::Value,
        context_after: &serde_json::Value,
        task_terms: &serde_json::Value,
        abbreviations: &serde_json::Value,
        engine_constraints: &serde_json::Value,
    ) -> Self {
        Self {
            mode,
            document_context: pdf_context::DocumentContext::from_value(document_context.clone())
                .unwrap_or_else(|_| pdf_context::DocumentContext::empty())
                .to_value(),
            context_before: pdf_context::bounded_value(context_before),
            context_after: pdf_context::bounded_value(context_after),
            task_terms: pdf_context::bounded_value(task_terms),
            abbreviations: pdf_context::bounded_value(abbreviations),
            engine_constraints: pdf_context::bounded_value(engine_constraints),
        }
    }
}

fn prepare_translation_internal(
    state: &AppState,
    request: &TranslationRequest,
    source_text: &str,
    pdf_context: Option<&PdfPromptContext>,
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
    let base_system_prompt = build_system_prompt(&prompt.content, &terms, source_text);
    let learning_mode = request.learning_mode && pdf_context.is_none();
    let cache_input = CacheKeyInput {
        base_url: &provider.base_url,
        provider_id: &provider.id,
        model_id: &provider.model_id,
        prompt_id: &prompt.id,
        prompt_version: prompt.version,
        glossary_version,
        source_language: &request.source_language,
        target_language: &request.target_language,
        source_text,
    };
    let (system_prompt, cache_key) = match pdf_context {
        Some(context) => (
            build_pdf_system_prompt(&base_system_prompt, context),
            make_pdf_cache_key(&cache_input, context),
        ),
        None if learning_mode => (
            paragraph_learning::build_system_prompt(&base_system_prompt),
            make_learning_cache_key(&cache_input),
        ),
        None => (base_system_prompt, make_cache_key(&cache_input)),
    };
    Ok(PreparedTranslation {
        provider,
        prompt,
        glossary_terms: terms,
        system_prompt,
        cache_key,
        cache_enabled: settings.cache_enabled,
        learning_mode,
        cache_max_bytes: settings.cache_max_bytes,
        history_retention: settings.history_retention,
        glossary_version,
    })
}

fn build_pdf_system_prompt(base_prompt: &str, context: &PdfPromptContext) -> String {
    let encode = |value: &serde_json::Value| {
        serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
    };
    format!(
        "{base_prompt}\n\nPDF 文档上下文：{}\n相邻段落前文：{}\n相邻段落后文：{}\n当前任务术语：{}\n当前任务缩写：{}\nPDF 引擎约束：{}\n全局术语表具有最高优先级。当前任务术语和缩写只在全局术语表没有命中时提供参考，发生冲突时必须遵循全局术语表。保持所有公式、富文本、占位符和结构标识。",
        encode(&context.document_context),
        encode(&context.context_before),
        encode(&context.context_after),
        encode(&context.task_terms),
        encode(&context.abbreviations),
        encode(&context.engine_constraints),
    )
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
    hash_cache_key(&cache_key_material(input))
}

fn make_learning_cache_key(input: &CacheKeyInput<'_>) -> String {
    let canonical = format!(
        "mode={}\nprotocol={}\n{}",
        paragraph_learning::PARAGRAPH_LEARNING_CACHE_MODE,
        paragraph_learning::PARAGRAPH_LEARNING_PROTOCOL_VERSION,
        cache_key_material(input),
    );
    hash_cache_key(&canonical)
}

fn cache_key_material(input: &CacheKeyInput<'_>) -> String {
    format!(
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
    )
}

fn hash_cache_key(canonical: &str) -> String {
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

fn make_pdf_cache_key(input: &CacheKeyInput<'_>, context: &PdfPromptContext) -> String {
    let canonical = format!(
        "base={}\nprovider={}\nmodel={}\nprompt={}@{}\nglossary={}\nmode={}\ncontext_schema={}\ncontext={}\nwindow_before={}\nwindow_after={}\ntask_terms={}\nabbreviations={}\nengine_constraints={}\nsource_language={}\ntarget_language={}\nsource={}",
        input.base_url,
        input.provider_id,
        input.model_id,
        input.prompt_id,
        input.prompt_version,
        input.glossary_version,
        context.mode.provider_operation(),
        context
            .document_context
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        pdf_context::hash_value(&context.document_context),
        pdf_context::hash_value(&context.context_before),
        pdf_context::hash_value(&context.context_after),
        pdf_context::hash_value(&context.task_terms),
        pdf_context::hash_value(&context.abbreviations),
        pdf_context::hash_value(&context.engine_constraints),
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

pub(crate) fn schedule_example_index(state: &AppState, cache_key: &str) {
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

fn read_glossary_file(file_path: &str) -> Result<glossary::ParsedGlossaryImport, String> {
    let path = file_path.trim();
    if path.is_empty() {
        return Err("术语表文件路径不能为空".to_string());
    }
    let bytes = fs::read(path).map_err(|error| format!("读取术语表文件失败：{error}"))?;
    let content =
        String::from_utf8(bytes).map_err(|_| "术语表文件必须使用 UTF-8 编码".to_string())?;
    Ok(glossary::parse_csv(&content))
}

#[tauri::command]
fn preview_glossary(file_path: String) -> Result<GlossaryImportPreview, String> {
    let parsed = read_glossary_file(&file_path)?;
    Ok(GlossaryImportPreview {
        terms: parsed
            .terms
            .into_iter()
            .map(|term| GlossaryImportPreviewTerm {
                source: term.source,
                target: term.target,
            })
            .collect(),
        skipped_rows: parsed.skipped_rows,
    })
}

#[tauri::command]
fn import_glossary(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<GlossaryImportResult, String> {
    let parsed = read_glossary_file(&file_path)?;
    let counts = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        db::import_glossary_terms(&connection, &parsed.terms)?
    };
    Ok(GlossaryImportResult {
        added_count: counts.added_count,
        updated_count: counts.updated_count,
        skipped_count: parsed.skipped_rows.len(),
        skipped_rows: parsed.skipped_rows,
    })
}

#[tauri::command]
fn export_glossary(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<GlossaryExportResult, String> {
    let path = file_path.trim();
    if path.is_empty() {
        return Err("术语表导出路径不能为空".to_string());
    }
    let (content, entry_count) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let terms = db::list_glossary_terms(&connection)?;
        if terms.is_empty() {
            return Err("术语表为空，没有可导出的内容".to_string());
        }
        let content = glossary::export_csv(&terms)?;
        (content, terms.len())
    };
    fs::write(path, content.as_bytes()).map_err(|error| format!("写入术语表文件失败：{error}"))?;
    let file_name = PathBuf::from(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("lilt-glossary.csv")
        .to_string();
    Ok(GlossaryExportResult {
        entry_count,
        file_name,
    })
}

#[tauri::command]
fn create_prompt(state: State<'_, AppState>) -> Result<Prompt, String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::create_prompt(&connection)
}

#[tauri::command]
fn update_prompt(
    state: State<'_, AppState>,
    id: String,
    name: String,
    content: String,
    source_language: String,
    target_language: String,
) -> Result<Prompt, String> {
    let id = id.trim();
    let name = name.trim();
    let content = content.trim();
    let source_language = source_language.trim();
    let target_language = target_language.trim();
    if id.is_empty() || name.is_empty() || source_language.is_empty() || target_language.is_empty()
    {
        return Err("Prompt ID、名称、源语言和目标语言不能为空".to_string());
    }
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::update_prompt(
        &connection,
        id,
        name,
        content,
        source_language,
        target_language,
    )
}

#[tauri::command]
fn delete_prompt(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    let provider = db::get_provider(&connection)?;
    db::delete_prompt(&connection, id.trim(), &provider.prompt_id)
}

#[tauri::command]
fn set_default_prompt(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("Prompt ID 不能为空".to_string());
    }
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::set_default_prompt(&connection, id.trim())
}

#[tauri::command]
fn save_personal_word(
    state: State<'_, AppState>,
    lookup_word: String,
    canonical_word: String,
) -> Result<PersonalDictionaryEntry, String> {
    let lookup_word = lookup_word.trim();
    let canonical_word = canonical_word.trim();
    let normalized_canonical_word = dictionary::normalize_headword(canonical_word);
    if lookup_word.is_empty() || canonical_word.is_empty() || normalized_canonical_word.is_empty() {
        return Err("个人词典词条不能为空".to_string());
    }
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::save_personal_word(
        &connection,
        &normalized_canonical_word,
        canonical_word,
        lookup_word,
    )
}

#[tauri::command]
fn remove_personal_word(state: State<'_, AppState>, canonical_word: String) -> Result<(), String> {
    let normalized_canonical_word = dictionary::normalize_headword(&canonical_word);
    if normalized_canonical_word.is_empty() {
        return Err("个人词典词条不能为空".to_string());
    }
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::remove_personal_word(&connection, &normalized_canonical_word)
}

#[tauri::command]
fn export_personal_dictionary(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<PersonalDictionaryExportResult, String> {
    let path = file_path.trim();
    if path.is_empty() {
        return Err("个人词典导出路径不能为空".to_string());
    }
    let (content, entry_count) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let Some((content, entry_count)) = db::personal_dictionary_export_text(&connection)? else {
            return Err("个人词典为空，没有可导出的内容".to_string());
        };
        (content, entry_count)
    };
    fs::write(path, content.as_bytes())
        .map_err(|error| format!("写入个人词典文件失败：{error}"))?;
    let file_name = PathBuf::from(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("lilt-personal-dictionary.txt")
        .to_string();
    Ok(PersonalDictionaryExportResult {
        entry_count,
        file_name,
    })
}

#[tauri::command]
fn resolve_window_close(
    app: AppHandle,
    state: State<'_, AppState>,
    action: String,
    remember: bool,
) -> Result<(), String> {
    let behavior = match action.trim() {
        "exit" => CloseBehavior::Exit,
        "tray" => CloseBehavior::Tray,
        _ => return Err("关闭行为无效".to_string()),
    };
    if remember {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        db::save_close_behavior(&connection, behavior)?;
    }
    match behavior {
        CloseBehavior::Exit => app.exit(0),
        CloseBehavior::Tray => {
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "主窗口不存在".to_string())?;
            window
                .hide()
                .map_err(|error| format!("隐藏主窗口失败：{error}"))?;
        }
        CloseBehavior::Ask => unreachable!("ask is not a valid close resolution"),
    }
    Ok(())
}

#[tauri::command]
fn reset_close_behavior(state: State<'_, AppState>) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    db::save_close_behavior(&connection, CloseBehavior::Ask)
}

#[tauri::command]
async fn query_dictionary(
    state: State<'_, AppState>,
    word: String,
    canonical_word: Option<String>,
) -> Result<DictionaryLookupCommandResult, String> {
    query_dictionary_impl(state.inner().clone(), word, canonical_word).await
}

async fn query_dictionary_impl(
    state: AppState,
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

    {
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

    let (lookup, candidates, sql_elapsed, json_decode_elapsed, cache_hit) = match resolution {
        dictionary::DictionaryLookupResolution::Found(measurement) => (
            measurement.result,
            Vec::new(),
            measurement.sql_elapsed,
            measurement.json_decode_elapsed,
            false,
        ),
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
                invalid_word: false,
                invalid_insight: None,
            });
        }
        dictionary::DictionaryLookupResolution::NotFound => {
            match query_ai_dictionary(&state, &request_id, &display_word).await? {
                AiDictionaryLookup::Invalid { insight } => {
                    let history = list_history()?;
                    diagnostics::info(format!(
                        "command.dictionary.query.invalid_word request_id={request_id} total_ms={}",
                        started.elapsed().as_millis()
                    ));
                    return Ok(DictionaryLookupCommandResult {
                        lookup: None,
                        candidates: Vec::new(),
                        example: None,
                        history,
                        invalid_word: true,
                        invalid_insight: Some(insight),
                    });
                }
                AiDictionaryLookup::Found { lookup, cache_hit } => (
                    lookup,
                    Vec::new(),
                    Duration::ZERO,
                    Duration::ZERO,
                    cache_hit,
                ),
            }
        }
    };

    let history_started = Instant::now();
    let (history, example) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        db::record_dictionary_query(&connection, &lookup.normalized_word, &lookup.word)?;
        let history = db::list_dictionary_history(&connection, DICTIONARY_HISTORY_LIMIT)?;
        let settings = db::get_settings(&connection)?;
        let example = if settings.paragraph_example_lookup_enabled {
            db::find_latest_example(&connection, &lookup.normalized_word)?.map(|record| {
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
        "command.dictionary.query.completed request_id={request_id} source={:?} cache_hit={cache_hit} sql_ms={} json_decode_ms={} history_ms={} snapshot_refresh=skipped total_ms={}",
        lookup.source,
        sql_elapsed.as_millis(),
        json_decode_elapsed.as_millis(),
        history_elapsed.as_millis(),
        started.elapsed().as_millis()
    ));
    Ok(DictionaryLookupCommandResult {
        lookup: Some(lookup),
        candidates,
        example,
        history,
        invalid_word: false,
        invalid_insight: None,
    })
}

const AI_DICTIONARY_SOURCE_LANGUAGE: &str = "en";
const AI_DICTIONARY_DEFINITION_LANGUAGE: &str = "zh-Hans";
const AI_DICTIONARY_SYSTEM_PROMPT: &str = r#"你是一名英语词典工具。判断用户输入是否为有效的英语单词、固定搭配或短语，并严格只输出一个 JSON 对象，不要输出 Markdown、代码围栏、解释或其他文字。

有效输入输出：{"valid":true,"entry":{...}}。
无效输入输出：{"valid":false,"insight":{"possible_spellings":[],"proper_noun":null,"note":""}}。

当输入不是有效英语词条时，仍然要给出有限的辅助判断：
- possible_spellings：最多 5 个最可能的英文拼写修正，按可能性排序；没有可靠候选时返回空数组。候选必须是可以继续查询的词形，不要返回解释或原输入。
- proper_noun：如果输入可能是非英文词汇、产品名、项目名、框架名、人名、地名或其他专有名词，返回 {"name":"...","description":"用简体中文简要说明它是什么"}；无法确认时返回 null。不要把普通乱码或任意未知字符串强行判断为专有名词。
- note：用简体中文给出一句简短的判断，无法补充时返回空字符串。

当 valid 为 true 时，entry 必须完整符合 distribution_entry_v5 词典条目结构。headword_language 固定为 {"code":"en","name":"English"}，definition_language 固定为 {"code":"zh-Hans","name":"Chinese (Simplified)"}。至少提供一个 pos_groups，每个 pos_group 至少提供一个 meanings；每个 meaning 的 learner_explanation 必须是非空中文释义。所有字段都必须存在，即使没有内容也使用空字符串、空数组或 null。不要添加结构之外的字段。"#;

enum AiDictionaryLookup {
    Invalid {
        insight: DictionaryInvalidInsight,
    },
    Found {
        lookup: contracts::DictionaryLookupResult,
        cache_hit: bool,
    },
}

#[derive(Clone, Copy)]
struct AiDictionaryCacheKeyInput<'a> {
    normalized_word: &'a str,
    source_language: &'a str,
    definition_language: &'a str,
    provider_id: &'a str,
    base_url: &'a str,
    model_id: &'a str,
    thinking_effort: &'a str,
    protocol_version: &'a str,
}

fn make_ai_dictionary_cache_key(input: &AiDictionaryCacheKeyInput<'_>) -> String {
    let canonical = format!(
        "word={}\nsource_language={}\ndefinition_language={}\nprovider={}\nbase_url={}\nmodel={}\nthinking={}\nprotocol={}",
        input.normalized_word,
        input.source_language,
        input.definition_language,
        input.provider_id,
        input.base_url,
        input.model_id,
        input.thinking_effort,
        input.protocol_version,
    );
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

async fn query_ai_dictionary(
    state: &AppState,
    request_id: &str,
    display_word: &str,
) -> Result<AiDictionaryLookup, String> {
    let normalized_word = dictionary::normalize_headword(display_word);
    if normalized_word.is_empty() {
        return Err("查询词形不能为空".to_string());
    }

    let (mut provider, cache_max_bytes) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let settings = db::get_settings(&connection)?;
        let provider = db::get_provider(&connection)?;
        (provider, settings.cache_max_bytes)
    };
    provider.base_url =
        provider::normalize_base_url(&provider.base_url).map_err(|error| error.to_string())?;
    let cache_key = make_ai_dictionary_cache_key(&AiDictionaryCacheKeyInput {
        normalized_word: &normalized_word,
        source_language: AI_DICTIONARY_SOURCE_LANGUAGE,
        definition_language: AI_DICTIONARY_DEFINITION_LANGUAGE,
        provider_id: &provider.id,
        base_url: &provider.base_url,
        model_id: &provider.model_id,
        thinking_effort: provider.thinking_effort.as_str(),
        protocol_version: AI_DICTIONARY_PROTOCOL_VERSION,
    });

    if let Some(lookup) =
        load_ai_dictionary_cache(state, &cache_key, display_word, &normalized_word, &provider)?
    {
        diagnostics::info(format!(
            "command.dictionary.ai_cache.hit request_id={request_id}"
        ));
        return Ok(AiDictionaryLookup::Found {
            lookup,
            cache_hit: true,
        });
    }

    let request_lock = {
        let mut requests = state
            .ai_dictionary_requests
            .lock()
            .map_err(|_| "AI 词典请求锁已损坏".to_string())?;
        requests.retain(|_, request_lock| request_lock.strong_count() > 0);
        if let Some(request_lock) = requests.get(&cache_key).and_then(Weak::upgrade) {
            request_lock
        } else {
            let request_lock = Arc::new(tokio::sync::Mutex::new(()));
            requests.insert(cache_key.clone(), Arc::downgrade(&request_lock));
            request_lock
        }
    };
    let _request_guard = request_lock.lock().await;

    if let Some(lookup) =
        load_ai_dictionary_cache(state, &cache_key, display_word, &normalized_word, &provider)?
    {
        diagnostics::info(format!(
            "command.dictionary.ai_cache.hit_after_wait request_id={request_id}"
        ));
        return Ok(AiDictionaryLookup::Found {
            lookup,
            cache_hit: true,
        });
    }

    let api_key = match secrets::load_api_key(&provider.id) {
        Ok(Some(value)) => value,
        Ok(None) => return Err("尚未配置 API Key，请在设置中保存 Provider".to_string()),
        Err(error) => return Err(error),
    };
    let cancellation = CancellationToken::new();
    let streamed = TranslationCore::stream(
        CoreStreamRequest {
            request_id,
            base_url: &provider.base_url,
            api_key: &api_key,
            model_id: &provider.model_id,
            system_prompt: AI_DICTIONARY_SYSTEM_PROMPT,
            user_text: display_word,
            cancel: &cancellation,
            mode: TranslationMode::Dictionary,
            thinking_effort: &provider.thinking_effort,
        },
        |_: String| Ok(()),
    )
    .await
    .map_err(|error| error.to_string())?;
    let parsed = parse_ai_dictionary_protocol(&streamed, &cache_key)?;
    let entry = match parsed {
        ParsedAiDictionary::Invalid { insight } => {
            return Ok(AiDictionaryLookup::Invalid { insight });
        }
        ParsedAiDictionary::Valid(entry) => entry,
    };
    let canonical_word = entry
        .get("headword")
        .and_then(Value::as_str)
        .ok_or_else(|| "AI 词典条目缺少规范词头".to_string())?
        .to_string();
    let entry_json =
        serde_json::to_string(&entry).map_err(|error| format!("AI 词典条目无法序列化：{error}"))?;

    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    let cache = db::AiDictionaryCacheWrite {
        cache_key: &cache_key,
        normalized_word: &normalized_word,
        lookup_word: display_word,
        canonical_word: &canonical_word,
        source_language: AI_DICTIONARY_SOURCE_LANGUAGE,
        definition_language: AI_DICTIONARY_DEFINITION_LANGUAGE,
        entry_json: &entry_json,
        provider: &provider,
        protocol_version: AI_DICTIONARY_PROTOCOL_VERSION,
    };
    db::save_ai_dictionary_cache(&connection, &cache)
        .and_then(|_| db::prune_cache(&connection, cache_max_bytes))?;

    diagnostics::info(format!(
        "command.dictionary.ai.generated request_id={request_id}"
    ));
    Ok(AiDictionaryLookup::Found {
        lookup: build_ai_dictionary_lookup(display_word, &normalized_word, entry),
        cache_hit: false,
    })
}

fn load_ai_dictionary_cache(
    state: &AppState,
    cache_key: &str,
    display_word: &str,
    normalized_word: &str,
    provider: &contracts::ProviderRecord,
) -> Result<Option<contracts::DictionaryLookupResult>, String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    let Some(record) = db::find_ai_dictionary_cache(&connection, cache_key)? else {
        return Ok(None);
    };
    match decode_ai_dictionary_cache_record(
        cache_key,
        display_word,
        normalized_word,
        provider,
        &record,
    ) {
        Ok(lookup) => Ok(Some(lookup)),
        Err(error) => {
            diagnostics::warn(format!(
                "command.dictionary.ai_cache.invalid cache_key={cache_key} reason={error}"
            ));
            db::delete_ai_dictionary_cache(&connection, cache_key)?;
            Ok(None)
        }
    }
}

fn decode_ai_dictionary_cache_record(
    cache_key: &str,
    display_word: &str,
    normalized_word: &str,
    provider: &contracts::ProviderRecord,
    record: &db::AiDictionaryCacheRecord,
) -> Result<contracts::DictionaryLookupResult, String> {
    if record.lookup_word.trim().is_empty()
        || dictionary::normalize_headword(&record.lookup_word) != normalized_word
        || record.normalized_word != normalized_word
        || record.source_language != AI_DICTIONARY_SOURCE_LANGUAGE
        || record.definition_language != AI_DICTIONARY_DEFINITION_LANGUAGE
        || record.provider_id != provider.id
        || record.base_url != provider.base_url
        || record.model_id != provider.model_id
        || record.thinking_effort != provider.thinking_effort.as_str()
        || record.protocol_version != AI_DICTIONARY_PROTOCOL_VERSION
    {
        return Err("AI 词典缓存身份不匹配".to_string());
    }
    let raw: Value = serde_json::from_str(&record.entry_json)
        .map_err(|error| format!("AI 词典缓存 JSON 无法解析：{error}"))?;
    let entry = normalize_ai_dictionary_entry(raw, cache_key)?;
    let canonical_word = entry
        .get("headword")
        .and_then(Value::as_str)
        .ok_or_else(|| "AI 词典缓存缺少规范词头".to_string())?;
    if record.canonical_word != canonical_word {
        return Err("AI 词典缓存规范词头不一致".to_string());
    }
    Ok(build_ai_dictionary_lookup(
        display_word,
        normalized_word,
        entry,
    ))
}

fn build_ai_dictionary_lookup(
    display_word: &str,
    normalized_word: &str,
    entry: Value,
) -> contracts::DictionaryLookupResult {
    let canonical_word = entry
        .get("headword")
        .and_then(Value::as_str)
        .unwrap_or(display_word)
        .to_string();
    contracts::DictionaryLookupResult {
        word: display_word.to_string(),
        normalized_word: normalized_word.to_string(),
        canonical_word,
        match_type: DictionaryMatchType::Exact,
        source: DictionarySource::Ai,
        entry,
    }
}

enum ParsedAiDictionary {
    Invalid { insight: DictionaryInvalidInsight },
    Valid(Value),
}

fn parse_ai_dictionary_protocol(raw: &str, cache_key: &str) -> Result<ParsedAiDictionary, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|error| format!("AI 词典协议 JSON 无法解析：{error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "AI 词典协议顶层必须是对象".to_string())?;
    let valid = object
        .get("valid")
        .and_then(Value::as_bool)
        .ok_or_else(|| "AI 词典协议缺少 valid 布尔字段".to_string())?;
    if !valid {
        let insight = if object.len() == 1 && object.contains_key("valid") {
            DictionaryInvalidInsight {
                possible_spellings: Vec::new(),
                proper_noun: None,
                note: String::new(),
            }
        } else {
            ensure_exact_fields(object, &["valid", "insight"], "AI 词典无效协议")?;
            parse_dictionary_invalid_insight(
                object
                    .get("insight")
                    .ok_or_else(|| "AI 词典无效协议缺少 insight".to_string())?,
            )?
        };
        return Ok(ParsedAiDictionary::Invalid { insight });
    }
    ensure_exact_fields(object, &["valid", "entry"], "AI 词典有效协议")?;
    let entry = object
        .get("entry")
        .ok_or_else(|| "AI 词典有效协议缺少 entry".to_string())?
        .clone();
    Ok(ParsedAiDictionary::Valid(normalize_ai_dictionary_entry(
        entry, cache_key,
    )?))
}

fn parse_dictionary_invalid_insight(value: &Value) -> Result<DictionaryInvalidInsight, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "AI 词典无效协议的 insight 必须是对象".to_string())?;
    ensure_exact_fields(
        object,
        &["possible_spellings", "proper_noun", "note"],
        "AI 词典无效协议的 insight",
    )?;
    let possible_spellings =
        string_array(object, "possible_spellings", "AI 词典无效协议的 insight")?;
    if possible_spellings.len() > 5 {
        return Err("AI 词典无效协议的 insight.possible_spellings 不能超过 5 项".to_string());
    }
    let mut suggestions = Vec::with_capacity(possible_spellings.len());
    for value in possible_spellings {
        let canonical_word = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "AI 词典无效协议的拼写候选不能为空".to_string())?;
        let normalized_canonical_word = dictionary::normalize_headword(canonical_word);
        if normalized_canonical_word.is_empty() {
            return Err("AI 词典无效协议的拼写候选无法规范化".to_string());
        }
        if suggestions
            .iter()
            .any(|candidate: &DictionaryLookupCandidate| {
                candidate.normalized_canonical_word == normalized_canonical_word
            })
        {
            continue;
        }
        suggestions.push(DictionaryLookupCandidate {
            canonical_word: canonical_word.to_string(),
            normalized_canonical_word,
        });
    }
    let proper_noun = match object.get("proper_noun") {
        Some(Value::Null) => None,
        Some(Value::Object(proper_noun)) => {
            ensure_exact_fields(
                proper_noun,
                &["name", "description"],
                "AI 词典无效协议的 proper_noun",
            )?;
            Some(DictionaryProperNounInsight {
                name: required_string(proper_noun, "name", "AI 词典无效协议的 proper_noun", false)?
                    .trim()
                    .to_string(),
                description: required_string(
                    proper_noun,
                    "description",
                    "AI 词典无效协议的 proper_noun",
                    false,
                )
                .map(str::trim)
                .map(str::to_string)?,
            })
        }
        _ => return Err("AI 词典无效协议的 proper_noun 必须是对象或 null".to_string()),
    };
    let note = required_string(object, "note", "AI 词典无效协议的 insight", true)?
        .trim()
        .to_string();
    Ok(DictionaryInvalidInsight {
        possible_spellings: suggestions,
        proper_noun,
        note,
    })
}

fn normalize_ai_dictionary_entry(value: Value, cache_key: &str) -> Result<Value, String> {
    validate_ai_dictionary_entry(&value)?;
    let mut entry = value;
    let object = entry
        .as_object_mut()
        .ok_or_else(|| "AI 词典 entry 必须是对象".to_string())?;
    let headword_language_code = object
        .get("headword_language")
        .and_then(Value::as_object)
        .and_then(|language| language.get("code"))
        .and_then(Value::as_str);
    let definition_language_code = object
        .get("definition_language")
        .and_then(Value::as_object)
        .and_then(|language| language.get("code"))
        .and_then(Value::as_str);
    if headword_language_code != Some(AI_DICTIONARY_SOURCE_LANGUAGE)
        || definition_language_code != Some(AI_DICTIONARY_DEFINITION_LANGUAGE)
    {
        return Err("AI 词典 entry 语言方向不符合固定契约".to_string());
    }
    let headword = object
        .get("headword")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "AI 词典 entry.headword 不能为空".to_string())?
        .to_string();
    let normalized_headword = dictionary::normalize_headword(&headword);
    if normalized_headword.is_empty() {
        return Err("AI 词典 entry.headword 无法规范化".to_string());
    }
    object.insert(
        "schema_version".to_string(),
        Value::String(contracts::DICTIONARY_DISTRIBUTION_SCHEMA_VERSION.to_string()),
    );
    object.insert(
        "entry_id".to_string(),
        Value::String(format!("ai-{cache_key}")),
    );
    object.insert("headword".to_string(), Value::String(headword));
    object.insert(
        "normalized_headword".to_string(),
        Value::String(normalized_headword),
    );
    object.insert(
        "headword_language".to_string(),
        serde_json::json!({"code": "en", "name": "English"}),
    );
    object.insert(
        "definition_language".to_string(),
        serde_json::json!({"code": "zh-Hans", "name": "Chinese (Simplified)"}),
    );
    Ok(entry)
}

fn ensure_exact_fields(
    object: &Map<String, Value>,
    fields: &[&str],
    context: &str,
) -> Result<(), String> {
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return Err(format!("{context}包含缺失或多余字段"));
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
    allow_empty: bool,
) -> Result<&'a str, String> {
    let value = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context}.{field} 必须是字符串"))?;
    if !allow_empty && value.trim().is_empty() {
        return Err(format!("{context}.{field} 不能为空"));
    }
    Ok(value)
}

fn optional_string(object: &Map<String, Value>, field: &str, context: &str) -> Result<(), String> {
    match object.get(field) {
        Some(Value::Null) | Some(Value::String(_)) => Ok(()),
        _ => Err(format!("{context}.{field} 必须是字符串或 null")),
    }
}

fn string_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Vec<Value>, String> {
    let value = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}.{field} 必须是数组"))?;
    if value.iter().any(|item| !item.is_string()) {
        return Err(format!("{context}.{field} 必须只包含字符串"));
    }
    Ok(value)
}

fn object_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Vec<Value>, String> {
    let value = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}.{field} 必须是数组"))?;
    if value.iter().any(|item| !item.is_object()) {
        return Err(format!("{context}.{field} 必须只包含对象"));
    }
    Ok(value)
}

fn validate_ai_dictionary_entry(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "AI 词典 entry 必须是对象".to_string())?;
    ensure_exact_fields(
        object,
        &[
            "schema_version",
            "entry_id",
            "headword",
            "normalized_headword",
            "headword_language",
            "definition_language",
            "entry_type",
            "headword_summary",
            "memory_hook",
            "study_notes",
            "etymology_note",
            "etymologies",
            "pos_groups",
        ],
        "AI 词典 entry",
    )?;
    required_string(object, "schema_version", "entry", false)?;
    required_string(object, "entry_id", "entry", false)?;
    required_string(object, "headword", "entry", false)?;
    required_string(object, "normalized_headword", "entry", false)?;
    validate_language(object.get("headword_language"), "entry.headword_language")?;
    validate_language(
        object.get("definition_language"),
        "entry.definition_language",
    )?;
    required_string(object, "entry_type", "entry", false)?;
    required_string(object, "headword_summary", "entry", true)?;
    required_string(object, "memory_hook", "entry", true)?;
    string_array(object, "study_notes", "entry")?;
    optional_string(object, "etymology_note", "entry")?;
    let etymologies = object_array(object, "etymologies", "entry")?;
    for (index, item) in etymologies.iter().enumerate() {
        validate_etymology(item, &format!("entry.etymologies[{index}]"))?;
    }
    let groups = object_array(object, "pos_groups", "entry")?;
    if groups.is_empty() {
        return Err("AI 词典 entry.pos_groups 不能为空".to_string());
    }
    for (index, item) in groups.iter().enumerate() {
        validate_pos_group(item, &format!("entry.pos_groups[{index}]"))?;
    }
    Ok(())
}

fn validate_language(value: Option<&Value>, context: &str) -> Result<(), String> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{context} 必须是对象"))?;
    ensure_exact_fields(object, &["code", "name"], context)?;
    required_string(object, "code", context, false)?;
    required_string(object, "name", context, false)?;
    Ok(())
}

fn validate_etymology(value: &Value, context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} 必须是对象"))?;
    ensure_exact_fields(object, &["etymology_id", "text", "pos_members"], context)?;
    required_string(object, "etymology_id", context, false)?;
    optional_string(object, "text", context)?;
    string_array(object, "pos_members", context)?;
    Ok(())
}

fn validate_pos_group(value: &Value, context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} 必须是对象"))?;
    ensure_exact_fields(
        object,
        &[
            "pos",
            "etymology_id",
            "proper_name",
            "summary",
            "usage_note",
            "forms",
            "pronunciations",
            "relations",
            "meanings",
        ],
        context,
    )?;
    required_string(object, "pos", context, false)?;
    optional_string(object, "etymology_id", context)?;
    if object.get("proper_name").and_then(Value::as_bool).is_none() {
        return Err(format!("{context}.proper_name 必须是布尔值"));
    }
    required_string(object, "summary", context, true)?;
    optional_string(object, "usage_note", context)?;
    let forms = object_array(object, "forms", context)?;
    for (index, item) in forms.iter().enumerate() {
        validate_form(item, &format!("{context}.forms[{index}]"))?;
    }
    let pronunciations = object_array(object, "pronunciations", context)?;
    for (index, item) in pronunciations.iter().enumerate() {
        validate_pronunciation(item, &format!("{context}.pronunciations[{index}]"))?;
    }
    let relations = object_array(object, "relations", context)?;
    for (index, item) in relations.iter().enumerate() {
        validate_relation(item, &format!("{context}.relations[{index}]"))?;
    }
    let meanings = object_array(object, "meanings", context)?;
    if meanings.is_empty() {
        return Err(format!("{context}.meanings 不能为空"));
    }
    for (index, item) in meanings.iter().enumerate() {
        validate_meaning(item, &format!("{context}.meanings[{index}]"))?;
    }
    Ok(())
}

fn validate_form(value: &Value, context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} 必须是对象"))?;
    ensure_exact_fields(object, &["text", "tags", "roman"], context)?;
    required_string(object, "text", context, false)?;
    string_array(object, "tags", context)?;
    optional_string(object, "roman", context)
}

fn validate_pronunciation(value: &Value, context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} 必须是对象"))?;
    ensure_exact_fields(object, &["ipa", "text", "tags"], context)?;
    optional_string(object, "ipa", context)?;
    optional_string(object, "text", context)?;
    string_array(object, "tags", context)?;
    Ok(())
}

fn validate_relation(value: &Value, context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} 必须是对象"))?;
    ensure_exact_fields(object, &["type", "word", "lang_code"], context)?;
    required_string(object, "type", context, false)?;
    required_string(object, "word", context, false)?;
    optional_string(object, "lang_code", context)
}

fn validate_meaning(value: &Value, context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} 必须是对象"))?;
    ensure_exact_fields(
        object,
        &[
            "sense_id",
            "priority",
            "short_gloss",
            "learner_explanation",
            "usage_note",
            "labels",
            "topics",
            "examples",
        ],
        context,
    )?;
    required_string(object, "sense_id", context, false)?;
    let priority = required_string(object, "priority", context, false)?;
    if !matches!(priority, "core" | "common" | "rare") {
        return Err(format!("{context}.priority 不是受支持的枚举值"));
    }
    optional_string(object, "short_gloss", context)?;
    required_string(object, "learner_explanation", context, false)?;
    optional_string(object, "usage_note", context)?;
    string_array(object, "labels", context)?;
    string_array(object, "topics", context)?;
    let examples = object_array(object, "examples", context)?;
    for (index, item) in examples.iter().enumerate() {
        let example_context = format!("{context}.examples[{index}]");
        let example = item
            .as_object()
            .ok_or_else(|| format!("{example_context} 必须是对象"))?;
        ensure_exact_fields(example, &["text", "translation"], &example_context)?;
        required_string(example, "text", &example_context, false)?;
        required_string(example, "translation", &example_context, false)?;
    }
    Ok(())
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
        request.source,
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
    let streamed = TranslationCore::stream(
        CoreStreamRequest {
            request_id: &request_id,
            base_url: &prepared.provider.base_url,
            api_key: &api_key,
            model_id: &prepared.provider.model_id,
            system_prompt: &system_prompt,
            user_text: &user_text,
            cancel: &cancellation,
            mode: TranslationMode::WordExample,
            thinking_effort: &prepared.provider.thinking_effort,
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
    source: DictionarySource,
) -> Result<PreparedWordExample, String> {
    let normalized_word = dictionary::normalize_headword(word);
    let (settings, provider, prompt, glossary_terms, glossary_version, example) = {
        let connection = state
            .database
            .lock()
            .map_err(|_| "应用数据库锁已损坏".to_string())?;
        let settings = db::get_settings(&connection)?;
        let mut provider = db::get_provider(&connection)?;
        provider.base_url =
            provider::normalize_base_url(&provider.base_url).map_err(|error| error.to_string())?;
        let prompt = db::get_prompt(&connection, &provider.prompt_id)?;
        let glossary_terms = db::list_glossary_terms(&connection)?;
        let glossary_version = db::glossary_version(&connection)?;
        if source == DictionarySource::Ai {
            let cache_key = make_ai_dictionary_cache_key(&AiDictionaryCacheKeyInput {
                normalized_word: &normalized_word,
                source_language: AI_DICTIONARY_SOURCE_LANGUAGE,
                definition_language: AI_DICTIONARY_DEFINITION_LANGUAGE,
                provider_id: &provider.id,
                base_url: &provider.base_url,
                model_id: &provider.model_id,
                thinking_effort: provider.thinking_effort.as_str(),
                protocol_version: AI_DICTIONARY_PROTOCOL_VERSION,
            });
            let record = db::find_ai_dictionary_cache(&connection, &cache_key)?
                .ok_or_else(|| "AI 词典缓存已经失效，请重新查询词典".to_string())?;
            let cached_lookup = decode_ai_dictionary_cache_record(
                &cache_key,
                word,
                &normalized_word,
                &provider,
                &record,
            )?;
            if dictionary::normalize_headword(&cached_lookup.canonical_word)
                != dictionary::normalize_headword(canonical_word)
            {
                return Err("AI 词典缓存规范词头与例句请求不匹配".to_string());
            }
        }
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
    if source == DictionarySource::Local {
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
                dictionary::normalize_headword(&measurement.result.canonical_word)
            }
            dictionary::DictionaryLookupResolution::Ambiguous(_) => {
                return Err("规范词头选择不明确，请重新查询词典".to_string());
            }
            dictionary::DictionaryLookupResolution::NotFound => {
                return Err("查询词形已经不再存在，请重新查询词典".to_string());
            }
        };
        if resolved_canonical != dictionary::normalize_headword(canonical_word) {
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
        AI_DICTIONARY_DEFINITION_LANGUAGE, AI_DICTIONARY_PROTOCOL_VERSION,
        AI_DICTIONARY_SOURCE_LANGUAGE, AiDictionaryCacheKeyInput, AppState, CacheKeyInput,
        ParsedAiDictionary, PdfPromptContext, StartupRuntime, WORD_EXAMPLE_PROTOCOL_VERSION,
        WordAiCacheKeyInput, WordExampleDelta, WordExampleProtocolParser, build_system_prompt,
        cancel_request, make_ai_dictionary_cache_key, make_cache_key, make_learning_cache_key,
        make_pdf_cache_key, make_word_ai_cache_key, parse_ai_dictionary_protocol,
        unregister_request,
    };
    use crate::contracts::GlossaryTerm;
    use crate::translation_core::TranslationMode;
    use rusqlite::Connection;
    use serde_json::json;
    use std::sync::{Arc, atomic::Ordering};
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
    fn learning_cache_key_is_isolated_from_plain_translation_cache() {
        let input = CacheKeyInput {
            base_url: "https://example.com/v1",
            provider_id: "default",
            model_id: "model-a",
            prompt_id: "prompt",
            prompt_version: 1,
            glossary_version: 1,
            source_language: "en",
            target_language: "zh-CN",
            source_text: "hello",
        };
        assert_ne!(make_cache_key(&input), make_learning_cache_key(&input));
    }

    #[test]
    fn pdf_cache_context_isolated_from_paragraph_cache() {
        let input = CacheKeyInput {
            base_url: "https://example.com/v1",
            provider_id: "default",
            model_id: "model-a",
            prompt_id: "prompt",
            prompt_version: 1,
            glossary_version: 1,
            source_language: "en",
            target_language: "zh-CN",
            source_text: "hello",
        };
        let empty_context = json!({"schema_version": 1});
        let changed_context = json!({"schema_version": 1, "title": "A different document"});
        let empty_window = json!([]);
        fn context(
            document_context: &serde_json::Value,
            empty_window: &serde_json::Value,
        ) -> PdfPromptContext {
            PdfPromptContext::new(
                TranslationMode::PdfSegment,
                document_context,
                empty_window,
                empty_window,
                empty_window,
                empty_window,
                empty_window,
            )
        }
        let paragraph_key = make_cache_key(&input);
        let pdf_key = make_pdf_cache_key(&input, &context(&empty_context, &empty_window));
        let changed_pdf_key = make_pdf_cache_key(&input, &context(&changed_context, &empty_window));
        assert_ne!(paragraph_key, pdf_key);
        assert_ne!(pdf_key, changed_pdf_key);
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

    fn ai_dictionary_entry_fixture() -> serde_json::Value {
        json!({
            "schema_version": "distribution_entry_v5",
            "entry_id": "model-entry-id",
            "headword": " Serendipity ",
            "normalized_headword": "model-normalized",
            "headword_language": { "code": "en", "name": "English" },
            "definition_language": { "code": "zh-Hans", "name": "Chinese (Simplified)" },
            "entry_type": "word",
            "headword_summary": "",
            "memory_hook": "",
            "study_notes": [],
            "etymology_note": null,
            "etymologies": [],
            "pos_groups": [{
                "pos": "noun",
                "etymology_id": null,
                "proper_name": false,
                "summary": "",
                "usage_note": null,
                "forms": [],
                "pronunciations": [],
                "relations": [],
                "meanings": [{
                    "sense_id": "1",
                    "priority": "common",
                    "short_gloss": "机缘巧合",
                    "learner_explanation": "意外发现美好事物的能力或现象",
                    "usage_note": null,
                    "labels": [],
                    "topics": [],
                    "examples": [{
                        "text": "It was a happy accident.",
                        "translation": "这是一次幸运的偶然。"
                    }]
                }]
            }]
        })
    }

    #[test]
    fn ai_dictionary_cache_key_isolated_by_provider_context_without_api_key() {
        let input = AiDictionaryCacheKeyInput {
            normalized_word: "serendipity",
            source_language: AI_DICTIONARY_SOURCE_LANGUAGE,
            definition_language: AI_DICTIONARY_DEFINITION_LANGUAGE,
            provider_id: "provider-a",
            base_url: "https://example.com/v1",
            model_id: "model-a",
            thinking_effort: "none",
            protocol_version: AI_DICTIONARY_PROTOCOL_VERSION,
        };
        let first = make_ai_dictionary_cache_key(&input);
        let changed_model = make_ai_dictionary_cache_key(&AiDictionaryCacheKeyInput {
            model_id: "model-b",
            ..input
        });
        let changed_thinking = make_ai_dictionary_cache_key(&AiDictionaryCacheKeyInput {
            thinking_effort: "high",
            ..input
        });
        assert_ne!(first, changed_model);
        assert_ne!(first, changed_thinking);
        assert!(!first.contains("api-key"));
    }

    #[test]
    fn ai_dictionary_protocol_normalizes_identity_fields() {
        let parsed = parse_ai_dictionary_protocol(
            &json!({ "valid": true, "entry": ai_dictionary_entry_fixture() }).to_string(),
            "cache-key",
        )
        .expect("valid AI dictionary protocol should parse");
        let ParsedAiDictionary::Valid(entry) = parsed else {
            panic!("valid protocol should return an entry");
        };
        assert_eq!(entry["schema_version"], "distribution_entry_v5");
        assert_eq!(entry["entry_id"], "ai-cache-key");
        assert_eq!(entry["headword"], "Serendipity");
        assert_eq!(entry["normalized_headword"], "serendipity");
        assert_eq!(entry["headword_language"]["code"], "en");
        assert_eq!(entry["definition_language"]["code"], "zh-Hans");
    }

    #[test]
    fn ai_dictionary_protocol_rejects_invalid_shapes_and_accepts_explicit_invalid_word() {
        assert!(matches!(
            parse_ai_dictionary_protocol(r#"{"valid":false}"#, "cache-key"),
            Ok(ParsedAiDictionary::Invalid { .. })
        ));
        let parsed = parse_ai_dictionary_protocol(
            r#"{"valid":false,"insight":{"possible_spellings":["tauri"],"proper_noun":{"name":"Tauri","description":"跨平台应用框架"},"note":"可能是项目名称"}}"#,
            "cache-key",
        )
        .expect("invalid word insight should parse");
        let ParsedAiDictionary::Invalid { insight } = parsed else {
            panic!("invalid protocol should return an insight");
        };
        assert_eq!(insight.possible_spellings[0].canonical_word, "tauri");
        assert_eq!(insight.proper_noun.as_ref().unwrap().name, "Tauri");
        assert_eq!(insight.note, "可能是项目名称");
        assert!(
            parse_ai_dictionary_protocol(r#"{"valid":false,"entry":null}"#, "cache-key").is_err()
        );
        assert!(
            parse_ai_dictionary_protocol(
                r#"{"valid":false,"insight":{"possible_spellings":[],"proper_noun":null}}"#,
                "cache-key"
            )
            .is_err()
        );
        assert!(parse_ai_dictionary_protocol("普通文本", "cache-key").is_err());

        let mut missing_meanings = ai_dictionary_entry_fixture();
        missing_meanings["pos_groups"][0]["meanings"] = json!([]);
        assert!(
            parse_ai_dictionary_protocol(
                &json!({ "valid": true, "entry": missing_meanings }).to_string(),
                "cache-key"
            )
            .is_err()
        );

        let mut extra_field = ai_dictionary_entry_fixture();
        extra_field["extra"] = json!(true);
        assert!(
            parse_ai_dictionary_protocol(
                &json!({ "valid": true, "entry": extra_field }).to_string(),
                "cache-key"
            )
            .is_err()
        );

        let mut wrong_language = ai_dictionary_entry_fixture();
        wrong_language["definition_language"]["code"] = json!("ja");
        assert!(
            parse_ai_dictionary_protocol(
                &json!({ "valid": true, "entry": wrong_language }).to_string(),
                "cache-key"
            )
            .is_err()
        );
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
    fn startup_stage_report_accepts_only_known_stages_without_showing_the_window() {
        let startup = StartupRuntime::new();
        assert!(startup.report_frontend_stage("dom_mounted", "main").is_ok());
        assert!(!startup.gate.first_paint.load(Ordering::Acquire));
        assert!(startup.report_frontend_stage("first_paint", "main").is_ok());
        assert!(!startup.gate.first_paint.load(Ordering::Acquire));
        assert!(!startup.main_visible.load(Ordering::Acquire));
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
