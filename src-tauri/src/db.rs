use crate::contracts::{
    AppSettings, CacheStats, CachedTranslation, DictionaryHistoryEntry, GlossaryTerm, HistoryEntry,
    ModelInfo, Prompt, ProviderRecord, DEFAULT_CACHE_MAX_BYTES, DEFAULT_GLOSSARY_ID,
    DEFAULT_HISTORY_RETENTION, DEFAULT_PROMPT_ID, DEFAULT_PROVIDER_ID,
    DICTIONARY_DISTRIBUTION_SCHEMA_VERSION, DICTIONARY_SQLITE_SCHEMA_VERSION,
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
    let stats = get_cache_stats(connection, cache_max_bytes)?;
    Ok(AppSettings {
        history_retention,
        cache_enabled,
        cache_max_bytes,
        cache_usage_bytes: stats.usage_bytes,
    })
}

pub fn save_settings(
    connection: &Connection,
    history_retention: i64,
    cache_enabled: bool,
    cache_max_bytes: i64,
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
    prune_history(connection, history_retention.clamp(1, 1000))?;
    prune_cache(
        connection,
        cache_max_bytes.clamp(16 * 1024 * 1024, 2 * 1024 * 1024 * 1024),
    )
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
    Ok(())
}

pub fn get_cache_stats(connection: &Connection, max_bytes: i64) -> Result<CacheStats, String> {
    let (usage_bytes, entry_count) = connection
        .query_row(
            "SELECT COALESCE(SUM(byte_size), 0), COUNT(*) FROM translation_cache",
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
                "SELECT COALESCE(SUM(byte_size), 0) FROM translation_cache",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("读取缓存占用失败：{error}"))?;
        if usage <= max_bytes {
            return Ok(());
        }
        let deleted = connection
            .execute(
                "DELETE FROM translation_cache WHERE cache_key = (SELECT cache_key FROM translation_cache ORDER BY last_used_at ASC LIMIT 1)",
                [],
            )
            .map_err(|error| format!("清理翻译缓存失败：{error}"))?;
        if deleted == 0 {
            return Ok(());
        }
    }
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
