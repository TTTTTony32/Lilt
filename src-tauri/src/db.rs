use crate::contracts::{
    AppSettings, CacheStats, CachedTranslation, DictionaryHistoryEntry, GlossaryTerm, HistoryEntry,
    ModelInfo, Prompt, ProviderRecord, SelectionMode, DEFAULT_CACHE_MAX_BYTES, DEFAULT_GLOSSARY_ID,
    DEFAULT_HISTORY_RETENTION, DEFAULT_PARAGRAPH_EXAMPLE_LOOKUP_ENABLED, DEFAULT_PROMPT_ID,
    DEFAULT_PROVIDER_ID, DEFAULT_SELECTION_MODE, DEFAULT_SELECTION_SHORTCUT,
    DEFAULT_WORD_AI_CACHE_ENABLED, DICTIONARY_DISTRIBUTION_SCHEMA_VERSION,
    DICTIONARY_SQLITE_SCHEMA_VERSION,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

pub const DEFAULT_MODEL_ID: &str = "gpt-4o-mini";

pub struct HistoryRecord<'a> {
    pub source_text: &'a str,
    pub translated_text: &'a str,
    pub source_language: &'a str,
    pub target_language: &'a str,
    pub provider: &'a ProviderRecord,
    pub prompt_id: &'a str,
    pub glossary_version: i64,
    pub cache_hit: bool,
}

pub struct CacheRecord<'a> {
    pub cache_key: &'a str,
    pub source_text: &'a str,
    pub translated_text: &'a str,
    pub source_language: &'a str,
    pub target_language: &'a str,
    pub provider: &'a ProviderRecord,
    pub prompt_id: &'a str,
    pub glossary_version: i64,
}

#[derive(Debug, Clone)]
pub struct ParagraphExampleRecord {
    pub example_id: i64,
    pub source_text: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct WordAiCacheRecord {
    pub translated_text: String,
    pub part_of_speech: String,
}

#[derive(Debug, Clone)]
pub struct WordAiCacheWrite<'a> {
    pub cache_key: &'a str,
    pub example_id: i64,
    pub normalized_word: &'a str,
    pub word: &'a str,
    pub canonical_word: &'a str,
    pub source_language: &'a str,
    pub target_language: &'a str,
    pub provider: &'a ProviderRecord,
    pub prompt_id: &'a str,
    pub glossary_version: i64,
    pub protocol_version: &'a str,
    pub translated_text: &'a str,
    pub part_of_speech: &'a str,
}

pub struct DictionaryInstallationRecord<'a> {
    pub release_tag: &'a str,
    pub artifact_sha256: &'a str,
    pub installed_at: &'a str,
    pub entry_count: i64,
    pub distribution_schema_version: &'a str,
    pub sqlite_schema_version: &'a str,
    pub compressed_bytes: i64,
    pub database_bytes: i64,
}

#[derive(Debug, Clone)]
pub struct DictionaryInstallation {
    pub release_tag: String,
    pub artifact_sha256: String,
    pub installed_at: String,
    pub entry_count: i64,
    pub distribution_schema_version: String,
    pub sqlite_schema_version: String,
    pub compressed_bytes: i64,
    pub database_bytes: i64,
}

pub fn migrate(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                base_url TEXT NOT NULL,
                model_id TEXT NOT NULL,
                prompt_id TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS models (
                id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                label TEXT NOT NULL,
                source TEXT NOT NULL,
                PRIMARY KEY (provider_id, id),
                FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS prompts (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                content TEXT NOT NULL,
                version INTEGER NOT NULL,
                is_builtin INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS glossaries (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS glossary_terms (
                id TEXT PRIMARY KEY NOT NULL,
                glossary_id TEXT NOT NULL,
                source TEXT NOT NULL,
                target TEXT NOT NULL,
                note TEXT,
                UNIQUE (glossary_id, source),
                FOREIGN KEY (glossary_id) REFERENCES glossaries(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS translation_history (
                id TEXT PRIMARY KEY NOT NULL,
                created_at TEXT NOT NULL,
                source_text TEXT NOT NULL,
                translated_text TEXT NOT NULL,
                source_language TEXT NOT NULL,
                target_language TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                provider_name TEXT NOT NULL,
                model_id TEXT NOT NULL,
                prompt_id TEXT NOT NULL,
                glossary_version INTEGER NOT NULL,
                cache_hit INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS translation_cache (
                cache_key TEXT PRIMARY KEY NOT NULL,
                created_at TEXT NOT NULL,
                last_used_at TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                source_text TEXT NOT NULL,
                translated_text TEXT NOT NULL,
                source_language TEXT NOT NULL,
                target_language TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                prompt_id TEXT NOT NULL,
                glossary_version INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS translation_cache_examples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                cache_key TEXT NOT NULL,
                sentence_index INTEGER NOT NULL,
                source_text TEXT NOT NULL,
                word_count INTEGER NOT NULL,
                source_created_at TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                UNIQUE (cache_key, sentence_index),
                FOREIGN KEY (cache_key) REFERENCES translation_cache(cache_key) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS translation_cache_example_terms (
                example_id INTEGER NOT NULL,
                normalized_word TEXT NOT NULL,
                PRIMARY KEY (example_id, normalized_word),
                FOREIGN KEY (example_id) REFERENCES translation_cache_examples(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_example_terms_word
                ON translation_cache_example_terms(normalized_word, example_id);
            CREATE INDEX IF NOT EXISTS idx_example_terms_example
                ON translation_cache_example_terms(example_id, normalized_word);
            CREATE INDEX IF NOT EXISTS idx_examples_cache
                ON translation_cache_examples(cache_key);

            CREATE TABLE IF NOT EXISTS translation_cache_example_index_state (
                cache_key TEXT PRIMARY KEY NOT NULL,
                status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                indexed_at TEXT,
                last_error TEXT,
                FOREIGN KEY (cache_key) REFERENCES translation_cache(cache_key) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS word_ai_cache (
                cache_key TEXT PRIMARY KEY NOT NULL,
                example_id INTEGER NOT NULL,
                normalized_word TEXT NOT NULL,
                word TEXT NOT NULL,
                canonical_word TEXT NOT NULL,
                source_language TEXT NOT NULL,
                target_language TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                prompt_id TEXT NOT NULL,
                glossary_version INTEGER NOT NULL,
                protocol_version TEXT NOT NULL,
                translated_text TEXT NOT NULL,
                part_of_speech TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_used_at TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                FOREIGN KEY (example_id) REFERENCES translation_cache_examples(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_word_ai_cache_last_used_at
                ON word_ai_cache(last_used_at ASC);

            CREATE INDEX IF NOT EXISTS idx_history_created_at ON translation_history(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_cache_last_used_at ON translation_cache(last_used_at ASC);

            CREATE TABLE IF NOT EXISTS dictionary_history (
                normalized_word TEXT PRIMARY KEY NOT NULL,
                display_word TEXT NOT NULL,
                last_queried_at TEXT NOT NULL,
                query_count INTEGER NOT NULL DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_dictionary_history_last_queried_at
                ON dictionary_history(last_queried_at DESC);

            CREATE TABLE IF NOT EXISTS dictionary_installation (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                release_tag TEXT NOT NULL,
                artifact_sha256 TEXT NOT NULL,
                installed_at TEXT NOT NULL,
                entry_count INTEGER NOT NULL,
                distribution_schema_version TEXT NOT NULL,
                sqlite_schema_version TEXT NOT NULL,
                compressed_bytes INTEGER NOT NULL,
                database_bytes INTEGER NOT NULL
            );
            ",
        )
        .map_err(|error| format!("数据库迁移失败：{error}"))?;

    connection
        .execute(
            "INSERT OR IGNORE INTO providers (id, name, base_url, model_id, prompt_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                DEFAULT_PROVIDER_ID,
                "OpenAI-compatible",
                "https://api.openai.com/v1",
                DEFAULT_MODEL_ID,
                DEFAULT_PROMPT_ID
            ],
        )
        .map_err(|error| format!("默认 Provider 初始化失败：{error}"))?;

    connection
        .execute(
            "INSERT OR IGNORE INTO prompts (id, name, content, version, is_builtin) VALUES (?1, ?2, ?3, 1, 1)",
            params![
                DEFAULT_PROMPT_ID,
                "通用段落翻译",
                "你是一名严谨的专业译者。将用户提供的段落翻译成目标语言，保留原文事实、语气、段落结构和 Markdown 格式。不要添加原文没有的信息，只输出译文。"
            ],
        )
        .map_err(|error| format!("默认 Prompt 初始化失败：{error}"))?;

    connection
        .execute(
            "INSERT OR IGNORE INTO glossaries (id, name, version) VALUES (?1, ?2, 1)",
            params![DEFAULT_GLOSSARY_ID, "全局术语表"],
        )
        .map_err(|error| format!("默认术语表初始化失败：{error}"))?;

    Ok(())
}

pub fn get_settings(connection: &Connection) -> Result<AppSettings, String> {
    let history_retention = get_setting(connection, "history_retention")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_HISTORY_RETENTION)
        .clamp(1, 1000);
    let cache_enabled = get_setting(connection, "cache_enabled")?
        .map(|value| value != "0")
        .unwrap_or(true);
    let cache_max_bytes = get_setting(connection, "cache_max_bytes")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_CACHE_MAX_BYTES)
        .clamp(16 * 1024 * 1024, 2 * 1024 * 1024 * 1024);
    let word_ai_cache_enabled = get_setting(connection, "word_ai_cache_enabled")?
        .map(|value| value != "0")
        .unwrap_or(DEFAULT_WORD_AI_CACHE_ENABLED);
    let paragraph_example_lookup_enabled =
        get_setting(connection, "paragraph_example_lookup_enabled")?
            .map(|value| value != "0")
            .unwrap_or(DEFAULT_PARAGRAPH_EXAMPLE_LOOKUP_ENABLED);
    let (selection_mode, selection_shortcut) = get_selection_settings(connection)?;
    let stats = get_cache_stats(connection, cache_max_bytes)?;
    Ok(AppSettings {
        history_retention,
        cache_enabled,
        cache_max_bytes,
        cache_usage_bytes: stats.usage_bytes,
        word_ai_cache_enabled,
        paragraph_example_lookup_enabled,
        selection_mode,
        selection_shortcut,
    })
}

pub fn get_selection_settings(connection: &Connection) -> Result<(SelectionMode, String), String> {
    let selection_mode = match get_setting(connection, "selection_mode")?.as_deref() {
        Some("automatic") => SelectionMode::Automatic,
        _ => DEFAULT_SELECTION_MODE,
    };
    let selection_shortcut = get_setting(connection, "selection_shortcut")?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SELECTION_SHORTCUT.to_string());
    Ok((selection_mode, selection_shortcut))
}

pub fn save_settings(
    connection: &Connection,
    history_retention: i64,
    cache_enabled: bool,
    cache_max_bytes: i64,
    word_ai_cache_enabled: bool,
    paragraph_example_lookup_enabled: bool,
) -> Result<(), String> {
    set_setting(
        connection,
        "history_retention",
        &history_retention.clamp(1, 1000).to_string(),
    )?;
    set_setting(
        connection,
        "cache_enabled",
        if cache_enabled { "1" } else { "0" },
    )?;
    set_setting(
        connection,
        "cache_max_bytes",
        &cache_max_bytes
            .clamp(16 * 1024 * 1024, 2 * 1024 * 1024 * 1024)
            .to_string(),
    )?;
    set_setting(
        connection,
        "word_ai_cache_enabled",
        if word_ai_cache_enabled { "1" } else { "0" },
    )?;
    set_setting(
        connection,
        "paragraph_example_lookup_enabled",
        if paragraph_example_lookup_enabled {
            "1"
        } else {
            "0"
        },
    )?;
    prune_history(connection, history_retention.clamp(1, 1000))?;
    prune_cache(
        connection,
        cache_max_bytes.clamp(16 * 1024 * 1024, 2 * 1024 * 1024 * 1024),
    )
}

pub fn save_selection_settings(
    connection: &Connection,
    mode: SelectionMode,
    shortcut: &str,
) -> Result<(), String> {
    set_setting(
        connection,
        "selection_mode",
        match mode {
            SelectionMode::Shortcut => "shortcut",
            SelectionMode::Automatic => "automatic",
        },
    )?;
    set_setting(connection, "selection_shortcut", shortcut)
}

pub fn get_provider(connection: &Connection) -> Result<ProviderRecord, String> {
    connection
        .query_row(
            "SELECT id, name, base_url, model_id, prompt_id FROM providers WHERE id = ?1",
            params![DEFAULT_PROVIDER_ID],
            |row| {
                Ok(ProviderRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    base_url: row.get(2)?,
                    model_id: row.get(3)?,
                    prompt_id: row.get(4)?,
                })
            },
        )
        .map_err(|error| format!("读取 Provider 失败：{error}"))
}

pub fn save_provider(
    connection: &Connection,
    base_url: &str,
    model_id: &str,
    prompt_id: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE providers SET base_url = ?1, model_id = ?2, prompt_id = ?3 WHERE id = ?4",
            params![base_url, model_id, prompt_id, DEFAULT_PROVIDER_ID],
        )
        .map_err(|error| format!("保存 Provider 失败：{error}"))?;
    Ok(())
}

pub fn list_models(connection: &Connection) -> Result<Vec<ModelInfo>, String> {
    let mut statement = connection
        .prepare("SELECT id, label, source FROM models WHERE provider_id = ?1 ORDER BY label COLLATE NOCASE")
        .map_err(|error| format!("读取模型列表失败：{error}"))?;
    let rows = statement
        .query_map(params![DEFAULT_PROVIDER_ID], |row| {
            Ok(ModelInfo {
                id: row.get(0)?,
                label: row.get(1)?,
                source: row.get(2)?,
            })
        })
        .map_err(|error| format!("读取模型列表失败：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取模型列表失败：{error}"))
}

pub fn replace_models(connection: &Connection, models: &[ModelInfo]) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM models WHERE provider_id = ?1",
            params![DEFAULT_PROVIDER_ID],
        )
        .map_err(|error| format!("清理模型列表失败：{error}"))?;
    for model in models {
        connection
            .execute(
                "INSERT INTO models (id, provider_id, label, source) VALUES (?1, ?2, ?3, ?4)",
                params![model.id, DEFAULT_PROVIDER_ID, model.label, model.source],
            )
            .map_err(|error| format!("保存模型列表失败：{error}"))?;
    }
    Ok(())
}

pub fn list_prompts(connection: &Connection) -> Result<Vec<Prompt>, String> {
    let mut statement = connection
        .prepare("SELECT id, name, content, version, is_builtin FROM prompts ORDER BY is_builtin DESC, name")
        .map_err(|error| format!("读取 Prompt 失败：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(Prompt {
                id: row.get(0)?,
                name: row.get(1)?,
                content: row.get(2)?,
                version: row.get(3)?,
                is_builtin: row.get::<_, i64>(4)? != 0,
            })
        })
        .map_err(|error| format!("读取 Prompt 失败：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取 Prompt 失败：{error}"))
}

pub fn get_prompt(connection: &Connection, prompt_id: &str) -> Result<Prompt, String> {
    connection
        .query_row(
            "SELECT id, name, content, version, is_builtin FROM prompts WHERE id = ?1",
            params![prompt_id],
            |row| {
                Ok(Prompt {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    content: row.get(2)?,
                    version: row.get(3)?,
                    is_builtin: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .map_err(|error| format!("读取 Prompt 失败：{error}"))
}

pub fn list_glossary_terms(connection: &Connection) -> Result<Vec<GlossaryTerm>, String> {
    let mut statement = connection
        .prepare("SELECT id, source, target, note FROM glossary_terms WHERE glossary_id = ?1 ORDER BY source COLLATE NOCASE")
        .map_err(|error| format!("读取术语表失败：{error}"))?;
    let rows = statement
        .query_map(params![DEFAULT_GLOSSARY_ID], |row| {
            Ok(GlossaryTerm {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                note: row.get(3)?,
            })
        })
        .map_err(|error| format!("读取术语表失败：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取术语表失败：{error}"))
}

pub fn upsert_glossary_term(
    connection: &Connection,
    id: Option<&str>,
    source: &str,
    target: &str,
    note: Option<&str>,
) -> Result<(), String> {
    let term_id = id.unwrap_or("");
    if term_id.is_empty() {
        connection
            .execute(
                "INSERT INTO glossary_terms (id, glossary_id, source, target, note) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(glossary_id, source) DO UPDATE SET target = excluded.target, note = excluded.note",
                params![uuid::Uuid::new_v4().to_string(), DEFAULT_GLOSSARY_ID, source, target, note],
            )
            .map_err(|error| format!("保存术语失败：{error}"))?;
    } else {
        connection
            .execute(
                "UPDATE glossary_terms SET source = ?1, target = ?2, note = ?3 WHERE id = ?4",
                params![source, target, note, term_id],
            )
            .map_err(|error| format!("更新术语失败：{error}"))?;
    }
    bump_glossary_version(connection)
}

pub fn delete_glossary_term(connection: &Connection, id: &str) -> Result<(), String> {
    connection
        .execute("DELETE FROM glossary_terms WHERE id = ?1", params![id])
        .map_err(|error| format!("删除术语失败：{error}"))?;
    bump_glossary_version(connection)
}

pub fn glossary_version(connection: &Connection) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT version FROM glossaries WHERE id = ?1",
            params![DEFAULT_GLOSSARY_ID],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取术语表版本失败：{error}"))
}

pub fn get_history(connection: &Connection, limit: i64) -> Result<Vec<HistoryEntry>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, created_at, source_text, translated_text, source_language, target_language,
                    provider_name, model_id, cache_hit
             FROM translation_history ORDER BY created_at DESC LIMIT ?1",
        )
        .map_err(|error| format!("读取翻译历史失败：{error}"))?;
    let rows = statement
        .query_map(params![limit.clamp(1, 1000)], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                created_at: row.get(1)?,
                source_text: row.get(2)?,
                translated_text: row.get(3)?,
                source_language: row.get(4)?,
                target_language: row.get(5)?,
                provider_name: row.get(6)?,
                model_id: row.get(7)?,
                cache_hit: row.get::<_, i64>(8)? != 0,
            })
        })
        .map_err(|error| format!("读取翻译历史失败：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取翻译历史失败：{error}"))
}

pub fn insert_history(connection: &Connection, record: &HistoryRecord<'_>) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO translation_history
             (id, created_at, source_text, translated_text, source_language, target_language,
              provider_id, provider_name, model_id, prompt_id, glossary_version, cache_hit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                uuid::Uuid::new_v4().to_string(),
                Utc::now().to_rfc3339(),
                record.source_text,
                record.translated_text,
                record.source_language,
                record.target_language,
                &record.provider.id,
                &record.provider.name,
                &record.provider.model_id,
                record.prompt_id,
                record.glossary_version,
                if record.cache_hit { 1 } else { 0 }
            ],
        )
        .map_err(|error| format!("写入翻译历史失败：{error}"))?;
    Ok(())
}

pub fn find_cache(
    connection: &Connection,
    cache_key: &str,
) -> Result<Option<CachedTranslation>, String> {
    let cached = connection
        .query_row(
            "SELECT translated_text FROM translation_cache WHERE cache_key = ?1",
            params![cache_key],
            |row| {
                Ok(CachedTranslation {
                    translated_text: row.get(0)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取翻译缓存失败：{error}"))?;
    if cached.is_some() {
        connection
            .execute(
                "UPDATE translation_cache SET last_used_at = ?1 WHERE cache_key = ?2",
                params![Utc::now().to_rfc3339(), cache_key],
            )
            .map_err(|error| format!("更新缓存访问时间失败：{error}"))?;
    }
    Ok(cached)
}

pub fn save_cache(connection: &Connection, record: &CacheRecord<'_>) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let byte_size = (record.source_text.len() + record.translated_text.len()) as i64;
    connection
        .execute(
            "INSERT INTO translation_cache
             (cache_key, created_at, last_used_at, byte_size, source_text, translated_text,
              source_language, target_language, provider_id, model_id, prompt_id, glossary_version)
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(cache_key) DO UPDATE SET
              last_used_at = excluded.last_used_at, byte_size = excluded.byte_size,
              translated_text = excluded.translated_text",
            params![
                record.cache_key,
                &now,
                byte_size,
                record.source_text,
                record.translated_text,
                record.source_language,
                record.target_language,
                &record.provider.id,
                &record.provider.model_id,
                record.prompt_id,
                record.glossary_version
            ],
        )
        .map_err(|error| format!("写入翻译缓存失败：{error}"))?;
    mark_example_index_pending(connection, record.cache_key)
}

pub fn get_cache_stats(connection: &Connection, max_bytes: i64) -> Result<CacheStats, String> {
    let (usage_bytes, entry_count) = connection
        .query_row(
            "SELECT
                COALESCE((SELECT SUM(byte_size) FROM translation_cache), 0)
                    + COALESCE((SELECT SUM(byte_size) FROM translation_cache_examples), 0)
                    + COALESCE((SELECT SUM(byte_size) FROM word_ai_cache), 0),
                (SELECT COUNT(*) FROM translation_cache) + (SELECT COUNT(*) FROM word_ai_cache)",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| format!("读取缓存占用失败：{error}"))?;
    Ok(CacheStats {
        usage_bytes,
        entry_count,
        max_bytes,
    })
}

pub fn prune_cache(connection: &Connection, max_bytes: i64) -> Result<(), String> {
    loop {
        let usage = connection
            .query_row(
                "SELECT
                    COALESCE((SELECT SUM(byte_size) FROM translation_cache), 0)
                        + COALESCE((SELECT SUM(byte_size) FROM translation_cache_examples), 0)
                        + COALESCE((SELECT SUM(byte_size) FROM word_ai_cache), 0)",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("读取缓存占用失败：{error}"))?;
        if usage <= max_bytes {
            return Ok(());
        }
        let oldest = connection
            .query_row(
                "SELECT cache_key, cache_kind FROM (
                    SELECT cache_key, last_used_at, 0 AS cache_kind FROM translation_cache
                    UNION ALL
                    SELECT cache_key, last_used_at, 1 AS cache_kind FROM word_ai_cache
                )
                ORDER BY last_used_at ASC, cache_kind ASC, cache_key ASC
                LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| format!("读取缓存清理候选失败：{error}"))?;
        let Some((cache_key, cache_kind)) = oldest else {
            return Ok(());
        };
        let deleted = if cache_kind == 0 {
            connection
                .execute(
                    "DELETE FROM translation_cache WHERE cache_key = ?1",
                    params![cache_key],
                )
                .map_err(|error| format!("清理翻译缓存失败：{error}"))?
        } else {
            connection
                .execute(
                    "DELETE FROM word_ai_cache WHERE cache_key = ?1",
                    params![cache_key],
                )
                .map_err(|error| format!("清理单词 AI 缓存失败：{error}"))?
        };
        if deleted == 0 {
            return Ok(());
        }
    }
}

pub fn mark_example_index_pending(connection: &Connection, cache_key: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO translation_cache_example_index_state
                (cache_key, status, attempts, indexed_at, last_error)
             VALUES (?1, 'pending', 0, NULL, NULL)
             ON CONFLICT(cache_key) DO UPDATE SET
                status = 'pending', indexed_at = NULL, last_error = NULL",
            params![cache_key],
        )
        .map_err(|error| format!("标记例句索引任务失败：{error}"))?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExampleIndexBackfillBatch {
    pub last_key: Option<String>,
    pub inserted: usize,
}

pub fn enqueue_missing_example_indexes(
    connection: &Connection,
    after_key: Option<&str>,
    limit: i64,
) -> Result<ExampleIndexBackfillBatch, String> {
    let limit = limit.clamp(1, 1000);
    let mut statement = connection
        .prepare(
            "SELECT cache.cache_key
             FROM translation_cache AS cache
             LEFT JOIN translation_cache_example_index_state AS state
               ON state.cache_key = cache.cache_key
             WHERE state.cache_key IS NULL
               AND (?1 IS NULL OR cache.cache_key > ?1)
             ORDER BY cache.cache_key
             LIMIT ?2",
        )
        .map_err(|error| format!("读取待登记例句索引失败：{error}"))?;
    let rows = statement
        .query_map(params![after_key, limit], |row| row.get::<_, String>(0))
        .map_err(|error| format!("读取待登记例句索引失败：{error}"))?;
    let keys = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取待登记例句索引失败：{error}"))?;
    drop(statement);
    let Some(last_key) = keys.last().cloned() else {
        return Ok(ExampleIndexBackfillBatch {
            last_key: None,
            inserted: 0,
        });
    };

    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("开启例句索引登记事务失败：{error}"))?;
    for cache_key in &keys {
        transaction
            .execute(
                "INSERT OR IGNORE INTO translation_cache_example_index_state
                    (cache_key, status, attempts, indexed_at, last_error)
                 VALUES (?1, 'pending', 0, NULL, NULL)",
                params![cache_key],
            )
            .map_err(|error| format!("登记例句索引任务失败：{error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("提交例句索引登记事务失败：{error}"))?;

    Ok(ExampleIndexBackfillBatch {
        last_key: Some(last_key),
        inserted: keys.len(),
    })
}

pub fn list_pending_example_indexes(
    connection: &Connection,
    limit: i64,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT cache_key FROM translation_cache_example_index_state
             WHERE status IN ('pending', 'running', 'failed')
             ORDER BY cache_key LIMIT ?1",
        )
        .map_err(|error| format!("读取例句索引任务失败：{error}"))?;
    let rows = statement
        .query_map(params![limit.clamp(1, 1000)], |row| row.get(0))
        .map_err(|error| format!("读取例句索引任务失败：{error}"))?;
    rows.collect::<Result<Vec<String>, _>>()
        .map_err(|error| format!("读取例句索引任务失败：{error}"))
}

pub fn index_translation_cache(connection: &Connection, cache_key: &str) -> Result<(), String> {
    let cache = connection
        .query_row(
            "SELECT source_text, source_language, created_at
             FROM translation_cache WHERE cache_key = ?1",
            params![cache_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取待索引翻译缓存失败：{error}"))?;
    let Some((source_text, source_language, source_created_at)) = cache else {
        return Ok(());
    };

    connection
        .execute(
            "INSERT INTO translation_cache_example_index_state
                (cache_key, status, attempts, indexed_at, last_error)
             VALUES (?1, 'running', 1, NULL, NULL)
             ON CONFLICT(cache_key) DO UPDATE SET
                status = 'running', attempts = attempts + 1, last_error = NULL",
            params![cache_key],
        )
        .map_err(|error| format!("更新例句索引状态失败：{error}"))?;

    let result = (|| -> Result<(), String> {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("开启例句索引事务失败：{error}"))?;
        transaction
            .execute(
                "DELETE FROM translation_cache_examples WHERE cache_key = ?1",
                params![cache_key],
            )
            .map_err(|error| format!("清理旧例句索引失败：{error}"))?;

        let sentences = if source_language.eq_ignore_ascii_case("en") {
            crate::examples::split_english_example_sentences(&source_text)
        } else {
            Vec::new()
        };
        let mut affected_words = std::collections::HashSet::new();
        for sentence in sentences {
            let byte_size = sentence.source_text.len() as i64;
            transaction
                .execute(
                    "INSERT INTO translation_cache_examples
                        (cache_key, sentence_index, source_text, word_count, source_created_at, byte_size)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        cache_key,
                        sentence.sentence_index,
                        sentence.source_text,
                        sentence.words.len() as i64,
                        source_created_at,
                        byte_size,
                    ],
                )
                .map_err(|error| format!("写入例句索引失败：{error}"))?;
            let example_id = transaction.last_insert_rowid();
            for normalized_word in sentence.words {
                affected_words.insert(normalized_word.clone());
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO translation_cache_example_terms
                            (example_id, normalized_word) VALUES (?1, ?2)",
                        params![example_id, normalized_word],
                    )
                    .map_err(|error| format!("写入例句词项索引失败：{error}"))?;
            }
        }

        for normalized_word in affected_words {
            transaction
                .execute(
                    "DELETE FROM translation_cache_example_terms
                     WHERE normalized_word = ?1
                       AND example_id NOT IN (
                           SELECT terms.example_id
                           FROM translation_cache_example_terms AS terms
                           JOIN translation_cache_examples AS examples
                             ON examples.id = terms.example_id
                           WHERE terms.normalized_word = ?1
                           ORDER BY examples.source_created_at DESC, examples.id DESC
                           LIMIT 5
                       )",
                    params![normalized_word],
                )
                .map_err(|error| format!("清理旧例句词项索引失败：{error}"))?;
        }
        transaction
            .execute(
                "DELETE FROM translation_cache_examples
                 WHERE cache_key = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM translation_cache_example_terms AS terms
                       WHERE terms.example_id = translation_cache_examples.id
                   )",
                params![cache_key],
            )
            .map_err(|error| format!("清理无效例句索引失败：{error}"))?;
        transaction
            .execute(
                "UPDATE translation_cache_example_index_state
                 SET status = 'completed', indexed_at = ?1, last_error = NULL
                 WHERE cache_key = ?2",
                params![Utc::now().to_rfc3339(), cache_key],
            )
            .map_err(|error| format!("完成例句索引失败：{error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("提交例句索引事务失败：{error}"))?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = connection.execute(
            "UPDATE translation_cache_example_index_state
             SET status = 'failed', last_error = ?1 WHERE cache_key = ?2",
            params![error, cache_key],
        );
        return Err(error);
    }
    Ok(())
}

pub fn find_latest_example(
    connection: &Connection,
    normalized_word: &str,
) -> Result<Option<ParagraphExampleRecord>, String> {
    connection
        .query_row(
            "SELECT examples.id, examples.source_text, examples.source_created_at
             FROM translation_cache_example_terms AS terms
             JOIN translation_cache_examples AS examples
               ON examples.id = terms.example_id
             WHERE terms.normalized_word = ?1
             ORDER BY examples.source_created_at DESC, examples.id DESC
             LIMIT 1",
            params![normalized_word],
            |row| {
                Ok(ParagraphExampleRecord {
                    example_id: row.get(0)?,
                    source_text: row.get(1)?,
                    created_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取段落例句失败：{error}"))
}

pub fn find_example_by_id_for_word(
    connection: &Connection,
    example_id: i64,
    normalized_word: &str,
) -> Result<Option<ParagraphExampleRecord>, String> {
    connection
        .query_row(
            "SELECT examples.id, examples.source_text, examples.source_created_at
             FROM translation_cache_examples AS examples
             JOIN translation_cache_example_terms AS terms
               ON terms.example_id = examples.id
             WHERE examples.id = ?1 AND terms.normalized_word = ?2",
            params![example_id, normalized_word],
            |row| {
                Ok(ParagraphExampleRecord {
                    example_id: row.get(0)?,
                    source_text: row.get(1)?,
                    created_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("校验例句词项关联失败：{error}"))
}

pub fn find_word_ai_cache(
    connection: &Connection,
    cache_key: &str,
) -> Result<Option<WordAiCacheRecord>, String> {
    let cached = connection
        .query_row(
            "SELECT translated_text, part_of_speech
             FROM word_ai_cache WHERE cache_key = ?1",
            params![cache_key],
            |row| {
                Ok(WordAiCacheRecord {
                    translated_text: row.get(0)?,
                    part_of_speech: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取单词 AI 缓存失败：{error}"))?;
    if cached.is_some() {
        connection
            .execute(
                "UPDATE word_ai_cache SET last_used_at = ?1 WHERE cache_key = ?2",
                params![Utc::now().to_rfc3339(), cache_key],
            )
            .map_err(|error| format!("更新单词 AI 缓存访问时间失败：{error}"))?;
    }
    Ok(cached)
}

pub fn save_word_ai_cache(
    connection: &Connection,
    record: &WordAiCacheWrite<'_>,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let byte_size = (record.translated_text.len() + record.part_of_speech.len()) as i64;
    connection
        .execute(
            "INSERT INTO word_ai_cache
                (cache_key, example_id, normalized_word, word, canonical_word,
                 source_language, target_language, provider_id, model_id, prompt_id,
                 glossary_version, protocol_version, translated_text, part_of_speech,
                 created_at, last_used_at, byte_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15, ?16)
             ON CONFLICT(cache_key) DO UPDATE SET
                example_id = excluded.example_id,
                translated_text = excluded.translated_text,
                part_of_speech = excluded.part_of_speech,
                last_used_at = excluded.last_used_at,
                byte_size = excluded.byte_size",
            params![
                record.cache_key,
                record.example_id,
                record.normalized_word,
                record.word,
                record.canonical_word,
                record.source_language,
                record.target_language,
                &record.provider.id,
                &record.provider.model_id,
                record.prompt_id,
                record.glossary_version,
                record.protocol_version,
                record.translated_text,
                record.part_of_speech,
                &now,
                byte_size,
            ],
        )
        .map_err(|error| format!("写入单词 AI 缓存失败：{error}"))?;
    Ok(())
}

pub fn prune_history(connection: &Connection, retention: i64) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM translation_history WHERE id NOT IN (SELECT id FROM translation_history ORDER BY created_at DESC LIMIT ?1)",
            params![retention.clamp(1, 1000)],
        )
        .map_err(|error| format!("清理翻译历史失败：{error}"))?;
    Ok(())
}

pub fn list_dictionary_history(
    connection: &Connection,
    limit: i64,
) -> Result<Vec<DictionaryHistoryEntry>, String> {
    let mut statement = connection
        .prepare(
            "SELECT normalized_word, display_word, last_queried_at, query_count
             FROM dictionary_history
             ORDER BY last_queried_at DESC
             LIMIT ?1",
        )
        .map_err(|error| format!("读取词典历史失败：{error}"))?;
    let rows = statement
        .query_map(params![limit.clamp(1, 1000)], |row| {
            Ok(DictionaryHistoryEntry {
                normalized_word: row.get(0)?,
                display_word: row.get(1)?,
                last_queried_at: row.get(2)?,
                query_count: row.get(3)?,
            })
        })
        .map_err(|error| format!("读取词典历史失败：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取词典历史失败：{error}"))
}

pub fn record_dictionary_query(
    connection: &Connection,
    normalized_word: &str,
    display_word: &str,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    connection
        .execute(
            "INSERT INTO dictionary_history
                (normalized_word, display_word, last_queried_at, query_count)
             VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(normalized_word) DO UPDATE SET
                display_word = excluded.display_word,
                last_queried_at = excluded.last_queried_at,
                query_count = dictionary_history.query_count + 1",
            params![normalized_word, display_word, now],
        )
        .map_err(|error| format!("写入词典历史失败：{error}"))?;
    prune_dictionary_history(connection)
}

pub fn prune_dictionary_history(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM dictionary_history
             WHERE normalized_word NOT IN (
                 SELECT normalized_word FROM dictionary_history
                 ORDER BY last_queried_at DESC LIMIT ?1
             )",
            params![crate::contracts::DICTIONARY_HISTORY_LIMIT],
        )
        .map_err(|error| format!("清理词典历史失败：{error}"))?;
    Ok(())
}

pub fn clear_dictionary_history(connection: &Connection) -> Result<(), String> {
    connection
        .execute("DELETE FROM dictionary_history", [])
        .map_err(|error| format!("清空词典历史失败：{error}"))?;
    Ok(())
}

pub fn save_dictionary_installation(
    connection: &Connection,
    record: &DictionaryInstallationRecord<'_>,
) -> Result<(), String> {
    if record.distribution_schema_version != DICTIONARY_DISTRIBUTION_SCHEMA_VERSION
        || record.sqlite_schema_version != DICTIONARY_SQLITE_SCHEMA_VERSION
    {
        return Err("词典安装元数据的契约版本不匹配".to_string());
    }
    connection
        .execute(
            "INSERT INTO dictionary_installation
                (id, release_tag, artifact_sha256, installed_at, entry_count,
                 distribution_schema_version, sqlite_schema_version, compressed_bytes, database_bytes)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                release_tag = excluded.release_tag,
                artifact_sha256 = excluded.artifact_sha256,
                installed_at = excluded.installed_at,
                entry_count = excluded.entry_count,
                distribution_schema_version = excluded.distribution_schema_version,
                sqlite_schema_version = excluded.sqlite_schema_version,
                compressed_bytes = excluded.compressed_bytes,
                database_bytes = excluded.database_bytes",
            params![
                record.release_tag,
                record.artifact_sha256,
                record.installed_at,
                record.entry_count,
                record.distribution_schema_version,
                record.sqlite_schema_version,
                record.compressed_bytes,
                record.database_bytes,
            ],
        )
        .map_err(|error| format!("写入词典安装信息失败：{error}"))?;
    Ok(())
}

pub fn get_dictionary_installation(
    connection: &Connection,
) -> Result<Option<DictionaryInstallation>, String> {
    connection
        .query_row(
            "SELECT release_tag, artifact_sha256, installed_at, entry_count,
                    distribution_schema_version, sqlite_schema_version,
                    compressed_bytes, database_bytes
             FROM dictionary_installation WHERE id = 1",
            [],
            |row| {
                Ok(DictionaryInstallation {
                    release_tag: row.get(0)?,
                    artifact_sha256: row.get(1)?,
                    installed_at: row.get(2)?,
                    entry_count: row.get(3)?,
                    distribution_schema_version: row.get(4)?,
                    sqlite_schema_version: row.get(5)?,
                    compressed_bytes: row.get(6)?,
                    database_bytes: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取词典安装信息失败：{error}"))
}

fn get_setting(connection: &Connection, key: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取本地设置失败：{error}"))
}

fn set_setting(connection: &Connection, key: &str, value: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|error| format!("保存本地设置失败：{error}"))?;
    Ok(())
}

fn bump_glossary_version(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "UPDATE glossaries SET version = version + 1 WHERE id = ?1",
            params![DEFAULT_GLOSSARY_ID],
        )
        .map_err(|error| format!("更新术语表版本失败：{error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory database should open");
        migrate(&connection).expect("database migration should succeed");
        connection
    }

    fn test_provider() -> ProviderRecord {
        ProviderRecord {
            id: DEFAULT_PROVIDER_ID.to_string(),
            name: "OpenAI-compatible".to_string(),
            base_url: "https://example.com/v1".to_string(),
            model_id: DEFAULT_MODEL_ID.to_string(),
            prompt_id: DEFAULT_PROMPT_ID.to_string(),
        }
    }

    #[test]
    fn cache_round_trip_and_capacity_pruning_work() {
        let connection = test_connection();
        let provider = test_provider();
        let record = CacheRecord {
            cache_key: "cache-key",
            source_text: "source",
            translated_text: "translated",
            source_language: "en",
            target_language: "zh-CN",
            provider: &provider,
            prompt_id: DEFAULT_PROMPT_ID,
            glossary_version: 1,
        };

        save_cache(&connection, &record).expect("cache write should succeed");
        let cached = find_cache(&connection, "cache-key")
            .expect("cache lookup should succeed")
            .expect("cache entry should exist");
        assert_eq!(cached.translated_text, "translated");
        assert_eq!(get_cache_stats(&connection, 1).unwrap().entry_count, 1);

        prune_cache(&connection, 1).expect("cache pruning should succeed");
        assert_eq!(get_cache_stats(&connection, 1).unwrap().entry_count, 0);
    }

    #[test]
    fn example_index_backfill_is_batched_and_idempotent() {
        let connection = test_connection();
        let provider = test_provider();
        for index in 0..3 {
            let cache_key = format!("cache-{index}");
            let source_text = format!("The target word appears in example {index}.");
            let record = CacheRecord {
                cache_key: &cache_key,
                source_text: &source_text,
                translated_text: "译文",
                source_language: "en",
                target_language: "zh-CN",
                provider: &provider,
                prompt_id: DEFAULT_PROMPT_ID,
                glossary_version: 1,
            };
            save_cache(&connection, &record).expect("cache write should succeed");
        }
        connection
            .execute("DELETE FROM translation_cache_example_index_state", [])
            .expect("old cache index state should be removable");

        let first = enqueue_missing_example_indexes(&connection, None, 2)
            .expect("first backfill batch should succeed");
        assert_eq!(first.inserted, 2);
        let cursor = first.last_key.expect("first batch should have a cursor");
        let second = enqueue_missing_example_indexes(&connection, Some(&cursor), 2)
            .expect("second backfill batch should succeed");
        assert_eq!(second.inserted, 1);
        assert!(second.last_key.is_some());
        let done = enqueue_missing_example_indexes(&connection, second.last_key.as_deref(), 2)
            .expect("completed backfill should be queryable");
        assert_eq!(
            done,
            ExampleIndexBackfillBatch {
                last_key: None,
                inserted: 0,
            }
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM translation_cache_example_index_state",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            3
        );
    }

    #[test]
    fn example_index_keeps_the_latest_five_and_returns_the_newest() {
        let connection = test_connection();
        let provider = test_provider();
        for index in 0..6 {
            let cache_key = format!("cache-{index}");
            let source_text = format!("The target word appears in example {index}.");
            let record = CacheRecord {
                cache_key: &cache_key,
                source_text: &source_text,
                translated_text: "译文",
                source_language: "en",
                target_language: "zh-CN",
                provider: &provider,
                prompt_id: DEFAULT_PROMPT_ID,
                glossary_version: 1,
            };
            save_cache(&connection, &record).expect("cache write should succeed");
            connection
                .execute(
                    "UPDATE translation_cache SET created_at = ?1, last_used_at = ?1 WHERE cache_key = ?2",
                    params![format!("2024-01-{:02}T00:00:00Z", index + 1), cache_key],
                )
                .expect("fixture timestamp should update");
            index_translation_cache(&connection, &cache_key)
                .expect("example indexing should succeed");
        }

        let indexed_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM translation_cache_example_terms WHERE normalized_word = 'target'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexed_count, 5);

        let latest = find_latest_example(&connection, "target")
            .expect("latest example lookup should succeed")
            .expect("latest example should exist");
        assert!(latest.source_text.contains("example 5"));
    }

    #[test]
    fn word_ai_cache_is_counted_and_cascades_with_its_example() {
        let connection = test_connection();
        let provider = test_provider();
        let record = CacheRecord {
            cache_key: "paragraph-cache",
            source_text: "A target example.",
            translated_text: "一个例句。",
            source_language: "en",
            target_language: "zh-CN",
            provider: &provider,
            prompt_id: DEFAULT_PROMPT_ID,
            glossary_version: 1,
        };
        save_cache(&connection, &record).expect("cache write should succeed");
        index_translation_cache(&connection, record.cache_key)
            .expect("example indexing should succeed");
        let example_id = find_latest_example(&connection, "target")
            .unwrap()
            .unwrap()
            .example_id;
        assert!(
            find_example_by_id_for_word(&connection, example_id, "other")
                .unwrap()
                .is_none()
        );
        assert!(
            find_example_by_id_for_word(&connection, example_id, "target")
                .unwrap()
                .is_some()
        );
        let word_cache = WordAiCacheWrite {
            cache_key: "word-cache",
            example_id,
            normalized_word: "target",
            word: "target",
            canonical_word: "target",
            source_language: "en",
            target_language: "zh-CN",
            provider: &provider,
            prompt_id: DEFAULT_PROMPT_ID,
            glossary_version: 1,
            protocol_version: crate::contracts::WORD_EXAMPLE_PROTOCOL_VERSION,
            translated_text: "目标例句。",
            part_of_speech: "noun",
        };
        save_word_ai_cache(&connection, &word_cache).expect("word cache write should succeed");
        assert!(find_word_ai_cache(&connection, "word-cache")
            .unwrap()
            .is_some());
        assert_eq!(get_cache_stats(&connection, 1024).unwrap().entry_count, 2);
        connection
            .execute(
                "DELETE FROM translation_cache WHERE cache_key = 'paragraph-cache'",
                [],
            )
            .expect("paragraph cache should delete");
        assert!(find_word_ai_cache(&connection, "word-cache")
            .unwrap()
            .is_none());
        assert!(find_latest_example(&connection, "target")
            .unwrap()
            .is_none());
    }

    #[test]
    fn settings_round_trip_includes_word_example_switches() {
        let connection = test_connection();
        save_settings(&connection, 12, true, 32 * 1024 * 1024, false, false)
            .expect("settings write should succeed");
        let settings = get_settings(&connection).expect("settings read should succeed");
        assert_eq!(settings.history_retention, 12);
        assert!(!settings.word_ai_cache_enabled);
        assert!(!settings.paragraph_example_lookup_enabled);
        assert_eq!(settings.selection_mode, SelectionMode::Shortcut);
        assert_eq!(settings.selection_shortcut, DEFAULT_SELECTION_SHORTCUT);

        save_selection_settings(&connection, SelectionMode::Automatic, "Alt+L")
            .expect("selection settings write should succeed");
        let selection_settings =
            get_settings(&connection).expect("selection settings read should succeed");
        assert_eq!(selection_settings.selection_mode, SelectionMode::Automatic);
        assert_eq!(selection_settings.selection_shortcut, "Alt+L");
    }

    #[test]
    fn history_retention_removes_old_entries() {
        let connection = test_connection();
        let provider = test_provider();
        for index in 0..3 {
            let source_text = format!("source-{index}");
            let record = HistoryRecord {
                source_text: &source_text,
                translated_text: "translated",
                source_language: "en",
                target_language: "zh-CN",
                provider: &provider,
                prompt_id: DEFAULT_PROMPT_ID,
                glossary_version: 1,
                cache_hit: false,
            };
            insert_history(&connection, &record).expect("history write should succeed");
        }

        prune_history(&connection, 2).expect("history pruning should succeed");
        assert_eq!(get_history(&connection, 100).unwrap().len(), 2);
    }

    #[test]
    fn dictionary_history_deduplicates_and_keeps_recent_limit() {
        let connection = test_connection();
        record_dictionary_query(&connection, "word-0", "word-0")
            .expect("dictionary history write should succeed");
        record_dictionary_query(&connection, "word-0", "Word")
            .expect("dictionary history update should succeed");
        let repeated_before_pruning = list_dictionary_history(&connection, 100)
            .expect("dictionary history read should succeed")
            .into_iter()
            .find(|entry| entry.normalized_word == "word-0")
            .expect("repeated word should exist before pruning");
        assert_eq!(repeated_before_pruning.query_count, 2);

        for index in 1..=crate::contracts::DICTIONARY_HISTORY_LIMIT {
            let word = format!("word-{index}");
            record_dictionary_query(&connection, &word, &word)
                .expect("dictionary history write should succeed");
        }

        let history = list_dictionary_history(&connection, 100)
            .expect("dictionary history read should succeed");
        assert_eq!(
            history.len() as i64,
            crate::contracts::DICTIONARY_HISTORY_LIMIT
        );
    }
}
