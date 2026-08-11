mod contracts;
mod db;
mod diagnostics;
mod provider;
mod secrets;

use contracts::{
    AppSnapshot, GlossaryTerm, ModelInfo, Prompt, ProviderConfig, TranslationCancelled,
    TranslationCompleted, TranslationFailed, TranslationRequest, TranslationStarted,
    DEFAULT_PROVIDER_ID,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    database: Arc<Mutex<Connection>>,
    cancellations: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl AppState {
    fn new(connection: Connection) -> Self {
        Self {
            database: Arc::new(Mutex::new(connection)),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| std::io::Error::other(format!("无法定位应用数据目录：{error}")))?;
            initialise_database(&data_dir).map_err(std::io::Error::other)?;
            let connection = Connection::open(data_dir.join("app.sqlite"))
                .map_err(|error| std::io::Error::other(format!("打开应用数据库失败：{error}")))?;
            db::migrate(&connection).map_err(std::io::Error::other)?;
            app.manage(AppState::new(connection));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_snapshot,
            save_provider_config,
            fetch_models,
            save_app_settings,
            translate,
            cancel_translation,
            upsert_glossary_term,
            delete_glossary_term
        ])
        .run(tauri::generate_context!())
        .expect("error while running Lilt");
}

fn initialise_database(data_dir: &PathBuf) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|error| format!("创建应用数据目录失败：{error}"))
}

#[tauri::command]
fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    snapshot_from_connection(&connection)
}

fn snapshot_from_connection(connection: &Connection) -> Result<AppSnapshot, String> {
    let settings = db::get_settings(connection)?;
    let provider = db::get_provider(connection)?;
    let has_api_key = secrets::load_api_key(&provider.id).ok().flatten().is_some();
    let models = db::list_models(connection)?;
    let prompts = db::list_prompts(connection)?;
    let glossary_terms = db::list_glossary_terms(connection)?;
    let history = db::get_history(connection, settings.history_retention)?;
    let cache_stats = db::get_cache_stats(connection, settings.cache_max_bytes)?;
    Ok(AppSnapshot {
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
    )
}

#[tauri::command]
async fn translate(
    app: AppHandle,
    state: State<'_, AppState>,
    request: TranslationRequest,
) -> Result<(), String> {
    translate_impl(app, state.inner().clone(), request).await
}

async fn translate_impl(
    app: AppHandle,
    state: AppState,
    request: TranslationRequest,
) -> Result<(), String> {
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
    app.emit(
        "translation_started",
        TranslationStarted {
            request_id: request_id.clone(),
        },
    )
    .map_err(|error| format!("发送翻译状态失败：{error}"))?;

    let cancellation = CancellationToken::new();
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| "取消状态锁已损坏".to_string())?;
        cancellations.insert(request_id.clone(), cancellation.clone());
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
            db::find_cache(&connection, &prepared.cache_key)?
        };
        if let Some(cached) = cached {
            diagnostics::info(format!(
                "command.translate.cache_hit request_id={} cache_key={}",
                request_id, prepared.cache_key
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
            persist?;
            app.emit(
                "translation_completed",
                TranslationCompleted {
                    request_id: request_id.clone(),
                    content: cached.translated_text,
                    cache_hit: true,
                },
            )
            .map_err(|error| format!("发送翻译结果失败：{error}"))?;
            diagnostics::info(format!(
                "command.translate.completed request_id={} cache_hit=true",
                request_id
            ));
            return Ok(());
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
            return Ok(());
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
        if prepared.cache_enabled {
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
            db::save_cache(&connection, &cache)?;
            db::prune_cache(&connection, prepared.cache_max_bytes)?;
        }
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
        db::insert_history(&connection, &history)?;
        db::prune_history(&connection, prepared.history_retention)
    };
    if let Err(error) = persistence {
        diagnostics::error(format!(
            "command.translate.persistence_failed request_id={} reason={error}",
            request_id
        ));
        return emit_failed(&app, &request_id, &error);
    }
    let output_chars = translated.chars().count();
    app.emit(
        "translation_completed",
        TranslationCompleted {
            request_id: request_id.clone(),
            content: translated,
            cache_hit: false,
        },
    )
    .map_err(|error| format!("发送翻译结果失败：{error}"))?;
    diagnostics::info(format!(
        "command.translate.completed request_id={} cache_hit=false output_chars={}",
        request_id, output_chars
    ));
    Ok(())
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
fn cancel_translation(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    let cancellation = state
        .cancellations
        .lock()
        .map_err(|_| "取消状态锁已损坏".to_string())?
        .get(&request_id)
        .cloned();
    if let Some(token) = cancellation {
        token.cancel();
        diagnostics::info(format!(
            "command.translate.cancel_requested request_id={request_id}"
        ));
    } else {
        diagnostics::warn(format!(
            "command.translate.cancel_missing request_id={request_id}"
        ));
    }
    Ok(())
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

fn unregister_request(state: &AppState, request_id: &str) {
    if let Ok(mut cancellations) = state.cancellations.lock() {
        cancellations.remove(request_id);
    }
}

fn emit_failed(app: &AppHandle, request_id: &str, message: &str) -> Result<(), String> {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_system_prompt, make_cache_key, CacheKeyInput};
    use crate::contracts::GlossaryTerm;

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
}
