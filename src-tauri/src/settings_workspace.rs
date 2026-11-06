use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};
use serde::Deserialize;
use tauri::AppHandle;

use crate::{
    AppState,
    contracts::{ApiProtocol, CloseBehavior, ModelInfo, SelectionMode, ThinkingEffort},
    db, provider, secrets,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInput {
    pub active_provider_id: String,
    pub providers: Vec<WorkspaceProvider>,
    pub deleted_provider_ids: Vec<String>,
    pub prompts: Vec<WorkspacePrompt>,
    pub deleted_prompt_ids: Vec<String>,
    pub settings: WorkspaceSettings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    pub prompt_id: String,
    pub thinking_effort: ThinkingEffort,
    pub api_protocol: ApiProtocol,
    pub api_key: Option<String>,
    pub models: Option<Vec<ModelInfo>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePrompt {
    pub id: String,
    pub name: String,
    pub content: String,
    pub source_language: String,
    pub target_language: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettings {
    pub history_retention: i64,
    pub cache_enabled: bool,
    pub cache_max_bytes: i64,
    pub word_ai_cache_enabled: bool,
    pub paragraph_example_lookup_enabled: bool,
    pub pdf_preflight_page_limit: i64,
    pub selection_mode: SelectionMode,
    pub selection_shortcut: String,
    pub close_behavior: CloseBehavior,
}

fn unique_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<HashSet<&'a str>, String> {
    let mut result = HashSet::new();
    for id in ids {
        if id.trim().is_empty() || !result.insert(id) {
            return Err(format!("{label}标识为空或重复"));
        }
    }
    Ok(result)
}

fn apply_database_changes(connection: &Connection, input: &WorkspaceInput) -> Result<(), String> {
    let existing_providers = db::list_providers(connection)?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<HashMap<_, _>>();
    let existing_prompts = db::list_prompts(connection)?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<HashMap<_, _>>();
    let provider_ids = unique_ids(
        input.providers.iter().map(|item| item.id.as_str()),
        "供应商",
    )?;
    let prompt_ids = unique_ids(input.prompts.iter().map(|item| item.id.as_str()), "提示词")?;
    let deleted_providers = unique_ids(
        input.deleted_provider_ids.iter().map(String::as_str),
        "删除供应商",
    )?;
    let deleted_prompts = unique_ids(
        input.deleted_prompt_ids.iter().map(String::as_str),
        "删除提示词",
    )?;
    if !provider_ids.contains(input.active_provider_id.as_str()) {
        return Err("请选择有效的供应商".to_string());
    }
    if input.settings.selection_shortcut.trim().is_empty() {
        return Err("快捷键不能为空".to_string());
    }
    for id in &deleted_providers {
        let item = existing_providers.get(*id).ok_or("待删除的供应商不存在")?;
        if item.is_builtin || provider_ids.contains(id) {
            return Err("内置供应商不能删除".to_string());
        }
    }
    for id in &deleted_prompts {
        let item = existing_prompts.get(*id).ok_or("待删除的提示词不存在")?;
        if item.is_builtin || prompt_ids.contains(id) {
            return Err("内置提示词不能删除".to_string());
        }
    }
    for prompt in &input.prompts {
        if prompt.name.trim().is_empty()
            || prompt.source_language.trim().is_empty()
            || prompt.target_language.trim().is_empty()
        {
            return Err("提示词名称、源语言和目标语言不能为空".to_string());
        }
        if !existing_prompts.contains_key(&prompt.id) && uuid::Uuid::parse_str(&prompt.id).is_err()
        {
            return Err("新提示词标识无效".to_string());
        }
    }
    for item in &input.providers {
        if item.name.trim().is_empty() || item.model_id.trim().is_empty() {
            return Err("供应商名称和 Model ID 不能为空".to_string());
        }
        if !prompt_ids.contains(item.prompt_id.as_str()) {
            return Err("供应商的默认提示词不存在".to_string());
        }
        provider::normalize_base_url(&item.base_url).map_err(|error| error.to_string())?;
        if !existing_providers.contains_key(&item.id) && uuid::Uuid::parse_str(&item.id).is_err() {
            return Err("新供应商标识无效".to_string());
        }
    }

    for prompt in &input.prompts {
        if let Some(current) = existing_prompts.get(&prompt.id) {
            if current.name != prompt.name.trim()
                || current.content != prompt.content.trim()
                || current.source_language != prompt.source_language.trim()
                || current.target_language != prompt.target_language.trim()
            {
                db::update_prompt(
                    connection,
                    &prompt.id,
                    &prompt.name,
                    &prompt.content,
                    &prompt.source_language,
                    &prompt.target_language,
                )?;
            }
        } else {
            connection.execute(
                "INSERT INTO prompts (id, name, content, source_language, target_language, version, is_builtin) VALUES (?1, ?2, ?3, ?4, ?5, 1, 0)",
                params![prompt.id, prompt.name.trim(), prompt.content.trim(), prompt.source_language.trim(), prompt.target_language.trim()],
            ).map_err(|error| format!("创建提示词失败：{error}"))?;
        }
    }
    for item in &input.providers {
        let base_url =
            provider::normalize_base_url(&item.base_url).map_err(|error| error.to_string())?;
        if !existing_providers.contains_key(&item.id) {
            connection.execute(
                "INSERT INTO providers (id, name, base_url, model_id, prompt_id, thinking_effort, api_protocol, is_builtin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
                params![item.id, item.name.trim(), base_url, item.model_id.trim(), item.prompt_id, item.thinking_effort.as_str(), item.api_protocol.as_str()],
            ).map_err(|error| format!("创建供应商失败：{error}"))?;
        } else {
            db::save_provider(
                connection,
                &item.id,
                &item.name,
                &base_url,
                item.model_id.trim(),
                item.api_protocol,
                item.thinking_effort,
            )?;
            db::set_default_prompt(connection, &item.id, &item.prompt_id)?;
        }
        if let Some(models) = &item.models {
            db::replace_models(connection, &item.id, models)?;
        }
    }
    for id in &input.deleted_provider_ids {
        connection
            .execute("DELETE FROM providers WHERE id = ?1", params![id])
            .map_err(|error| format!("删除供应商失败：{error}"))?;
    }
    db::set_active_provider(connection, &input.active_provider_id)?;
    let active_prompt = db::get_provider(connection)?.prompt_id;
    for id in &input.deleted_prompt_ids {
        db::delete_prompt(connection, id, &active_prompt)?;
    }
    db::save_settings(
        connection,
        input.settings.history_retention,
        input.settings.cache_enabled,
        input.settings.cache_max_bytes,
        input.settings.word_ai_cache_enabled,
        input.settings.paragraph_example_lookup_enabled,
        input.settings.pdf_preflight_page_limit,
    )?;
    db::save_selection_settings(
        connection,
        input.settings.selection_mode,
        input.settings.selection_shortcut.trim(),
    )?;
    db::save_close_behavior(connection, input.settings.close_behavior)?;
    Ok(())
}

fn restore_credentials(
    previous: &[(String, Option<String>)],
    write: &mut impl FnMut(&str, Option<&str>) -> Result<(), String>,
) -> bool {
    let mut restored = true;
    for (id, key) in previous.iter().rev() {
        if write(id, key.as_deref()).is_err() {
            restored = false;
        }
    }
    restored
}

fn apply_credentials(
    changes: &[(String, Option<String>)],
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
    write: &mut impl FnMut(&str, Option<&str>) -> Result<(), String>,
) -> Result<Vec<(String, Option<String>)>, String> {
    let mut previous = Vec::new();
    for (id, key) in changes {
        let outcome = read(id).and_then(|old| {
            previous.push((id.clone(), old));
            write(id, key.as_deref())
        });
        if let Err(reason) = outcome {
            return if restore_credentials(&previous, write) {
                Err(reason)
            } else {
                Err(format!("{reason}；恢复原有凭据失败，请检查设置"))
            };
        }
    }
    Ok(previous)
}

pub fn save(app: &AppHandle, state: &AppState, input: WorkspaceInput) -> Result<(), String> {
    let connection = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("开启设置保存事务失败：{error}"))?;
    apply_database_changes(&transaction, &input)?;

    let previous_selection = state.selection.status();
    let selection_changed = previous_selection.mode != input.settings.selection_mode
        || previous_selection.shortcut != input.settings.selection_shortcut.trim();
    let mut selection_applied = false;
    let mut changed_keys: Vec<(String, Option<String>)> = Vec::new();
    let changes: Vec<(String, Option<String>)> = input
        .providers
        .iter()
        .filter_map(|item| {
            item.api_key
                .as_deref()
                .filter(|key| !key.trim().is_empty())
                .map(|key| (item.id.clone(), Some(key.to_string())))
        })
        .chain(
            input
                .deleted_provider_ids
                .iter()
                .map(|id| (id.clone(), None)),
        )
        .collect();
    let mut write_key = |id: &str, key: Option<&str>| secrets::save_api_key(id, key);
    let outcome = (|| -> Result<(), String> {
        changed_keys = apply_credentials(&changes, secrets::load_api_key, &mut write_key)?;
        if selection_changed {
            state.selection.configure(
                app,
                input.settings.selection_mode,
                input.settings.selection_shortcut.trim(),
            )?;
            selection_applied = true;
        }
        transaction
            .commit()
            .map_err(|error| format!("提交设置失败：{error}"))?;
        Ok(())
    })();
    if let Err(reason) = outcome {
        let mut rollback_failed = false;
        if selection_applied
            && state
                .selection
                .configure(app, previous_selection.mode, &previous_selection.shortcut)
                .is_err()
        {
            rollback_failed = true;
        }
        if !restore_credentials(&changed_keys, &mut write_key) {
            rollback_failed = true;
        }
        return if rollback_failed {
            Err(format!("{reason}；恢复原有运行时或凭据失败，请检查设置"))
        } else {
            Err(reason)
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(connection: &Connection) -> WorkspaceInput {
        db::migrate(connection).unwrap();
        let settings = db::get_settings(connection).unwrap();
        let provider = db::get_provider(connection).unwrap();
        WorkspaceInput {
            active_provider_id: provider.id,
            providers: db::list_providers(connection)
                .unwrap()
                .into_iter()
                .map(|p| WorkspaceProvider {
                    id: p.id,
                    name: p.name,
                    base_url: p.base_url,
                    model_id: p.model_id,
                    prompt_id: p.prompt_id,
                    thinking_effort: p.thinking_effort,
                    api_protocol: p.api_protocol,
                    api_key: None,
                    models: None,
                })
                .collect(),
            deleted_provider_ids: vec![],
            prompts: db::list_prompts(connection)
                .unwrap()
                .into_iter()
                .map(|p| WorkspacePrompt {
                    id: p.id,
                    name: p.name,
                    content: p.content,
                    source_language: p.source_language,
                    target_language: p.target_language,
                })
                .collect(),
            deleted_prompt_ids: vec![],
            settings: WorkspaceSettings {
                history_retention: settings.history_retention,
                cache_enabled: settings.cache_enabled,
                cache_max_bytes: settings.cache_max_bytes,
                word_ai_cache_enabled: settings.word_ai_cache_enabled,
                paragraph_example_lookup_enabled: settings.paragraph_example_lookup_enabled,
                pdf_preflight_page_limit: settings.pdf_preflight_page_limit,
                selection_mode: settings.selection_mode,
                selection_shortcut: settings.selection_shortcut,
                close_behavior: settings.close_behavior,
            },
        }
    }

    #[test]
    fn workspace_saves_new_provider_prompt_and_settings_together() {
        let connection = Connection::open_in_memory().unwrap();
        let mut input = fixture(&connection);
        let prompt_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        input.prompts.push(WorkspacePrompt {
            id: prompt_id.clone(),
            name: "新提示词".into(),
            content: "正文".into(),
            source_language: "en".into(),
            target_language: "zh-CN".into(),
        });
        input.providers.push(WorkspaceProvider {
            id: provider_id.clone(),
            name: "新供应商".into(),
            base_url: "https://example.com/v1".into(),
            model_id: "model".into(),
            prompt_id: prompt_id.clone(),
            thinking_effort: ThinkingEffort::None,
            api_protocol: ApiProtocol::Responses,
            api_key: None,
            models: None,
        });
        input.active_provider_id = provider_id.clone();
        input.settings.history_retention = 23;
        let transaction = connection.unchecked_transaction().unwrap();
        apply_database_changes(&transaction, &input).unwrap();
        transaction.commit().unwrap();
        assert_eq!(db::get_provider(&connection).unwrap().id, provider_id);
        assert_eq!(db::get_provider(&connection).unwrap().prompt_id, prompt_id);
        assert_eq!(
            db::get_prompt(&connection, &prompt_id).unwrap().content,
            "正文"
        );
        assert_eq!(db::get_settings(&connection).unwrap().history_retention, 23);
    }

    #[test]
    fn workspace_failure_rolls_back_earlier_edits() {
        let connection = Connection::open_in_memory().unwrap();
        let mut input = fixture(&connection);
        let original_model = input.providers[0].model_id.clone();
        input.providers[0].model_id = "changed-model".into();
        let duplicate = ModelInfo {
            id: "same".into(),
            label: "same".into(),
            source: "remote".into(),
        };
        input.providers[1].models = Some(vec![duplicate.clone(), duplicate]);
        let transaction = connection.unchecked_transaction().unwrap();
        assert!(apply_database_changes(&transaction, &input).is_err());
        drop(transaction);
        assert_eq!(
            db::get_provider_by_id(&connection, &input.providers[0].id)
                .unwrap()
                .model_id,
            original_model
        );
    }

    #[test]
    fn deleting_provider_and_its_prompt_is_one_transaction() {
        let connection = Connection::open_in_memory().unwrap();
        let _ = fixture(&connection);
        let provider = db::create_provider(&connection).unwrap();
        let prompt = db::create_prompt(&connection).unwrap();
        db::set_default_prompt(&connection, &provider.id, &prompt.id).unwrap();
        let mut input = fixture(&connection);
        input.providers.retain(|item| item.id != provider.id);
        input.prompts.retain(|item| item.id != prompt.id);
        input.deleted_provider_ids.push(provider.id.clone());
        input.deleted_prompt_ids.push(prompt.id.clone());
        input.active_provider_id = "openai".to_string();
        let transaction = connection.unchecked_transaction().unwrap();
        apply_database_changes(&transaction, &input).unwrap();
        transaction.commit().unwrap();
        assert!(db::get_provider_by_id(&connection, &provider.id).is_err());
        assert!(db::get_prompt(&connection, &prompt.id).is_err());
        assert_eq!(db::get_provider(&connection).unwrap().id, "openai");
    }

    #[test]
    fn credential_failure_restores_prior_changes() {
        use std::cell::RefCell;
        let storage = RefCell::new(HashMap::from([(
            "first".to_string(),
            "old-key".to_string(),
        )]));
        let changes = vec![
            ("first".to_string(), Some("new-key".to_string())),
            ("second".to_string(), Some("fail".to_string())),
        ];
        let read = |id: &str| Ok(storage.borrow().get(id).cloned());
        let mut write = |id: &str, key: Option<&str>| {
            if key == Some("fail") {
                return Err("模拟凭据错误".to_string());
            }
            if let Some(key) = key {
                storage.borrow_mut().insert(id.to_string(), key.to_string());
            } else {
                storage.borrow_mut().remove(id);
            }
            Ok(())
        };
        assert!(apply_credentials(&changes, read, &mut write).is_err());
        assert_eq!(
            storage.borrow().get("first").map(String::as_str),
            Some("old-key")
        );
        assert!(!storage.borrow().contains_key("second"));
    }

    #[test]
    fn workspace_payload_accepts_frontend_camel_case_fields() {
        let input: WorkspaceInput = serde_json::from_value(serde_json::json!({
            "activeProviderId": "openai",
            "providers": [{
                "id": "openai", "name": "OpenAI", "baseUrl": "https://api.openai.com/v1",
                "modelId": "model", "promptId": "builtin-general", "thinkingEffort": "none",
                "apiProtocol": "responses", "apiKey": null,
                "models": [{ "id": "model", "label": "Model", "source": "remote" }]
            }],
            "deletedProviderIds": [],
            "prompts": [{ "id": "builtin-general", "name": "默认", "content": "正文", "sourceLanguage": "auto", "targetLanguage": "zh-CN" }],
            "deletedPromptIds": [],
            "settings": {
                "historyRetention": 50, "cacheEnabled": true, "cacheMaxBytes": 268435456,
                "wordAiCacheEnabled": true, "paragraphExampleLookupEnabled": true,
                "pdfPreflightPageLimit": 10, "selectionMode": "shortcut",
                "selectionShortcut": "Ctrl+Shift+L", "closeBehavior": "ask"
            }
        })).unwrap();
        assert_eq!(input.providers[0].models.as_ref().unwrap()[0].id, "model");
        assert_eq!(input.settings.selection_mode, SelectionMode::Shortcut);
    }
}
