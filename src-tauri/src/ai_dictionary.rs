use crate::{diagnostics, dictionary, normalize_ai_dictionary_entry};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::{fs, path::PathBuf, sync::Mutex};

const STORAGE_VERSION: i64 = 1;

pub struct AiDictionaryStore {
    path: PathBuf,
    connection: Option<Connection>,
    initialised: bool,
}

#[derive(Debug)]
pub struct AiDictionaryEntry {
    pub entry: Value,
}

struct StoredEntry {
    normalized_word: String,
    lookup_word: String,
    canonical_word: String,
    entry_json: String,
}

struct LegacyEntry {
    cache_key: String,
    record: StoredEntry,
    source_language: String,
    definition_language: String,
    created_at: String,
    last_used_at: String,
}

impl AiDictionaryStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            path: directory.join("ai-dictionary.sqlite"),
            connection: None,
            initialised: false,
        }
    }

    // Callers hold the AI store lock. Never wait for that lock while holding the
    // application database lock: migration briefly needs the latter twice.
    pub fn initialise(&mut self, application_database: &Mutex<Connection>) -> Result<(), String> {
        if self.initialised {
            return Ok(());
        }
        self.open()?;
        let legacy = {
            let source = application_database
                .lock()
                .map_err(|_| "应用数据库锁已损坏".to_string())?;
            read_legacy_entries(&source)?
        };
        if !legacy.is_empty() {
            self.import_legacy_entries(&legacy)?;
            // The destination COMMIT must complete before touching the source.
            // A cleanup failure leaves initialised=false so the next call retries.
            let source = application_database
                .lock()
                .map_err(|_| "应用数据库锁已损坏".to_string())?;
            cleanup_legacy_entries(&source, &legacy)?;
            diagnostics::info(format!(
                "dictionary.ai.migrated source_count={}",
                legacy.len()
            ));
        }
        self.initialised = true;
        Ok(())
    }

    pub fn find(&self, word: &str) -> Result<Option<AiDictionaryEntry>, String> {
        let normalized_word = dictionary::normalize_headword(word);
        let connection = self.connection()?;
        let record = connection
            .query_row(
                "SELECT normalized_word, lookup_word, canonical_word, entry_json
                 FROM ai_dictionary_entries WHERE normalized_word = ?1",
                params![normalized_word],
                |row| {
                    Ok(StoredEntry {
                        normalized_word: row.get(0)?,
                        lookup_word: row.get(1)?,
                        canonical_word: row.get(2)?,
                        entry_json: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("读取 AI 词典失败：{error}"))?;
        let Some(record) = record else {
            return Ok(None);
        };
        match decode_entry(&record) {
            Ok(entry) => Ok(Some(AiDictionaryEntry { entry })),
            Err(error) => {
                diagnostics::warn(format!(
                    "dictionary.ai.entry.invalid word={normalized_word} reason={error}"
                ));
                connection
                    .execute(
                        "DELETE FROM ai_dictionary_entries WHERE normalized_word = ?1",
                        params![normalized_word],
                    )
                    .map_err(|error| format!("删除无效 AI 词典条目失败：{error}"))?;
                Ok(None)
            }
        }
    }

    pub fn save(&self, lookup_word: &str, entry: Value) -> Result<(), String> {
        let normalized_word = dictionary::normalize_headword(lookup_word);
        if normalized_word.is_empty() {
            return Err("查询词形不能为空".to_string());
        }
        let entry = normalize_ai_dictionary_entry(entry, &normalized_word)?;
        let canonical_word = entry["headword"]
            .as_str()
            .ok_or_else(|| "AI 词典条目缺少规范词头".to_string())?;
        let entry_json = serde_json::to_string(&entry)
            .map_err(|error| format!("AI 词典条目无法序列化：{error}"))?;
        let now = Utc::now().to_rfc3339();
        self.connection()?
            .execute(
                "INSERT INTO ai_dictionary_entries
                    (normalized_word, lookup_word, canonical_word, entry_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(normalized_word) DO UPDATE SET
                    lookup_word = excluded.lookup_word,
                    canonical_word = excluded.canonical_word,
                    entry_json = excluded.entry_json,
                    updated_at = excluded.updated_at",
                params![normalized_word, lookup_word, canonical_word, entry_json, now],
            )
            .map_err(|error| format!("保存 AI 词典条目失败：{error}"))?;
        Ok(())
    }

    fn open(&mut self) -> Result<(), String> {
        if self.connection.is_some() {
            return Ok(());
        }
        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory)
                .map_err(|error| format!("创建 AI 词典目录失败：{error}"))?;
        }
        let connection =
            Connection::open(&self.path).map_err(|error| format!("打开 AI 词典失败：{error}"))?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| format!("读取 AI 词典存储版本失败：{error}"))?;
        if version > STORAGE_VERSION {
            return Err(format!("AI 词典存储版本不兼容：{version}"));
        }
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("开启 AI 词典初始化事务失败：{error}"))?;
        transaction
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS ai_dictionary_entries (
                    normalized_word TEXT PRIMARY KEY NOT NULL,
                    lookup_word TEXT NOT NULL,
                    canonical_word TEXT NOT NULL,
                    entry_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .map_err(|error| format!("初始化 AI 词典失败：{error}"))?;
        transaction
            .pragma_update(None, "user_version", STORAGE_VERSION)
            .map_err(|error| format!("更新 AI 词典存储版本失败：{error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("提交 AI 词典初始化失败：{error}"))?;
        self.connection = Some(connection);
        Ok(())
    }

    fn connection(&self) -> Result<&Connection, String> {
        self.connection
            .as_ref()
            .ok_or_else(|| "AI 词典尚未初始化".to_string())
    }

    fn import_legacy_entries(&self, legacy: &[LegacyEntry]) -> Result<(), String> {
        let transaction = self
            .connection()?
            .unchecked_transaction()
            .map_err(|error| format!("开启 AI 词典迁移事务失败：{error}"))?;
        // Input is sorted newest generation first. The conflict clause preserves
        // existing destination data and the first valid legacy entry per word.
        for legacy_entry in legacy {
            let decoded = if legacy_entry.source_language != "en"
                || legacy_entry.definition_language != "zh-Hans"
            {
                Err("AI 词典语言方向不符合固定契约".to_string())
            } else {
                decode_entry(&legacy_entry.record)
            };
            let entry = match decoded {
                Ok(entry) => entry,
                Err(error) => {
                    diagnostics::warn(format!(
                        "dictionary.ai.migration.invalid cache_key={} reason={error}",
                        legacy_entry.cache_key
                    ));
                    continue;
                }
            };
            let entry_json = serde_json::to_string(&entry)
                .map_err(|error| format!("序列化旧 AI 词典条目失败：{error}"))?;
            transaction
                .execute(
                    "INSERT INTO ai_dictionary_entries
                        (normalized_word, lookup_word, canonical_word, entry_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                     ON CONFLICT(normalized_word) DO NOTHING",
                    params![
                        legacy_entry.record.normalized_word,
                        legacy_entry.record.lookup_word,
                        legacy_entry.record.canonical_word,
                        entry_json,
                        legacy_entry.created_at,
                    ],
                )
                .map_err(|error| format!("迁移 AI 词典条目失败：{error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("提交 AI 词典迁移失败：{error}"))
    }
}

fn decode_entry(record: &StoredEntry) -> Result<Value, String> {
    let normalized_word = dictionary::normalize_headword(&record.lookup_word);
    if normalized_word.is_empty() || normalized_word != record.normalized_word {
        return Err("AI 词典查询词身份不匹配".to_string());
    }
    let raw = serde_json::from_str(&record.entry_json)
        .map_err(|error| format!("AI 词典条目 JSON 无法解析：{error}"))?;
    let entry = normalize_ai_dictionary_entry(raw, &normalized_word)?;
    if entry["headword"].as_str() != Some(record.canonical_word.as_str()) {
        return Err("AI 词典条目规范词头不一致".to_string());
    }
    Ok(entry)
}

fn read_legacy_entries(source: &Connection) -> Result<Vec<LegacyEntry>, String> {
    let exists: bool = source
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'ai_dictionary_cache')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("检查旧 AI 词典表失败：{error}"))?;
    if !exists {
        return Ok(Vec::new());
    }
    // Provider fields, including the optional legacy api_protocol column, have
    // no part in word identity or migration.
    let mut statement = source
        .prepare(
            "SELECT cache_key, normalized_word, lookup_word, canonical_word,
                    source_language, definition_language, entry_json, created_at, last_used_at
             FROM ai_dictionary_cache
             ORDER BY created_at DESC, last_used_at DESC, cache_key DESC",
        )
        .map_err(|error| format!("读取旧 AI 词典失败：{error}"))?;
    statement
        .query_map([], |row| {
            Ok(LegacyEntry {
                cache_key: row.get(0)?,
                record: StoredEntry {
                    normalized_word: row.get(1)?,
                    lookup_word: row.get(2)?,
                    canonical_word: row.get(3)?,
                    entry_json: row.get(6)?,
                },
                source_language: row.get(4)?,
                definition_language: row.get(5)?,
                created_at: row.get(7)?,
                last_used_at: row.get(8)?,
            })
        })
        .map_err(|error| format!("读取旧 AI 词典条目失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解码旧 AI 词典条目失败：{error}"))
}

fn cleanup_legacy_entries(source: &Connection, legacy: &[LegacyEntry]) -> Result<(), String> {
    let transaction = source
        .unchecked_transaction()
        .map_err(|error| format!("开启旧 AI 词典清理事务失败：{error}"))?;
    for entry in legacy {
        transaction
            .execute(
                "DELETE FROM ai_dictionary_cache
                 WHERE cache_key = ?1 AND entry_json = ?2 AND created_at = ?3 AND last_used_at = ?4",
                params![entry.cache_key, entry.record.entry_json, entry.created_at, entry.last_used_at],
            )
            .map_err(|error| format!("清理旧 AI 词典失败：{error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("提交旧 AI 词典清理失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use serde_json::json;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!("lilt-ai-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(headword: &str, summary: &str) -> Value {
        json!({
            "headword": headword,
            "headword_summary": summary,
            "pos_groups": [{"pos": "noun", "meanings": [{
                "learner_explanation": "意外发现美好事物的能力或现象"
            }]}]
        })
    }

    fn legacy_source() -> Mutex<Connection> {
        let source = Connection::open_in_memory().unwrap();
        // Deliberately omit api_protocol and all unused provider fields.
        source
            .execute_batch(
                "CREATE TABLE ai_dictionary_cache (
                cache_key TEXT PRIMARY KEY, normalized_word TEXT NOT NULL,
                lookup_word TEXT NOT NULL, canonical_word TEXT NOT NULL,
                source_language TEXT NOT NULL, definition_language TEXT NOT NULL,
                entry_json TEXT NOT NULL, created_at TEXT NOT NULL, last_used_at TEXT NOT NULL
            );",
            )
            .unwrap();
        Mutex::new(source)
    }

    fn insert_legacy(
        source: &Mutex<Connection>,
        key: &str,
        word: &str,
        entry: Value,
        created_at: &str,
        last_used_at: &str,
    ) {
        source.lock().unwrap().execute(
            "INSERT INTO ai_dictionary_cache VALUES (?1, ?2, ?3, ?4, 'en', 'zh-Hans', ?5, ?6, ?7)",
            params![key, dictionary::normalize_headword(word), word,
                entry["headword"].as_str().unwrap().trim(), entry.to_string(), created_at, last_used_at],
        ).unwrap();
    }

    fn legacy_count(source: &Mutex<Connection>) -> i64 {
        source
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM ai_dictionary_cache", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn ai_dictionary_normalizes_words_and_reopens_complete_entries() {
        let directory = TestDirectory::new();
        let source = Mutex::new(Connection::open_in_memory().unwrap());
        {
            let mut store = AiDictionaryStore::new(directory.0.clone());
            assert!(!store.path.exists(), "construction must remain lazy");
            store.initialise(&source).unwrap();
            store
                .save("  SeReNdIpItY  ", entry("Serendipity", "saved"))
                .unwrap();
        }
        let mut reopened = AiDictionaryStore::new(directory.0.clone());
        reopened.initialise(&source).unwrap();
        let saved = reopened.find(" SERENDIPITY ").unwrap().unwrap();
        assert_eq!(saved.entry["headword_summary"], "saved");
        assert_eq!(saved.entry["entry_id"], "ai-serendipity");
        assert_eq!(saved.entry["definition_language"]["code"], "zh-Hans");
        let (lookup_word, count): (String, i64) = reopened.connection().unwrap().query_row(
            "SELECT lookup_word, (SELECT COUNT(*) FROM ai_dictionary_entries) FROM ai_dictionary_entries",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(lookup_word, "  SeReNdIpItY  ");
        assert_eq!(count, 1);
        assert!(
            reopened.find("serendipities").unwrap().is_none(),
            "AI forms never create candidates"
        );
    }

    #[test]
    fn ai_dictionary_migration_uses_latest_valid_generation_without_api_protocol() {
        let directory = TestDirectory::new();
        let source = legacy_source();
        insert_legacy(
            &source,
            "old",
            "serendipity",
            entry("Serendipity", "old"),
            "2024",
            "2030",
        );
        insert_legacy(
            &source,
            "a",
            " Serendipity ",
            entry("Serendipity", "tie loser"),
            "2025",
            "2025",
        );
        insert_legacy(
            &source,
            "z",
            "SERENDIPITY",
            entry("Serendipity", "latest valid"),
            "2025",
            "2025",
        );
        insert_legacy(
            &source,
            "invalid",
            "serendipity",
            json!({"headword":"Serendipity", "pos_groups":[]}),
            "2026",
            "2026",
        );
        let mut store = AiDictionaryStore::new(directory.0.clone());
        store.initialise(&source).unwrap();
        assert_eq!(
            store.find("serendipity").unwrap().unwrap().entry["headword_summary"],
            "latest valid"
        );
        assert_eq!(legacy_count(&source), 0);
        let created: String = store
            .connection()
            .unwrap()
            .query_row("SELECT created_at FROM ai_dictionary_entries", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(created, "2025");
        drop(store);
        let mut reopened = AiDictionaryStore::new(directory.0.clone());
        reopened.initialise(&source).unwrap();
        assert_eq!(
            reopened.find("serendipity").unwrap().unwrap().entry["headword_summary"],
            "latest valid"
        );
    }

    #[test]
    fn ai_dictionary_migration_rolls_back_destination_failure_and_retries() {
        let directory = TestDirectory::new();
        let source = legacy_source();
        insert_legacy(
            &source,
            "first",
            "serendipity",
            entry("Serendipity", "first"),
            "2025",
            "2025",
        );
        insert_legacy(
            &source,
            "second",
            "zeta",
            entry("Zeta", "second"),
            "2024",
            "2024",
        );
        let mut store = AiDictionaryStore::new(directory.0.clone());
        store.open().unwrap();
        store.connection().unwrap().execute_batch(
            "CREATE TRIGGER reject_import BEFORE INSERT ON ai_dictionary_entries
             WHEN NEW.normalized_word = 'zeta' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;",
        ).unwrap();
        assert!(
            store
                .initialise(&source)
                .unwrap_err()
                .contains("fixture write failure")
        );
        assert_eq!(legacy_count(&source), 2);
        assert!(
            store.find("serendipity").unwrap().is_none(),
            "the earlier insert must roll back"
        );
        store
            .connection()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_import")
            .unwrap();
        store.initialise(&source).unwrap();
        assert_eq!(legacy_count(&source), 0);
        assert!(store.find("serendipity").unwrap().is_some());
        assert!(store.find("zeta").unwrap().is_some());
    }

    #[test]
    fn ai_dictionary_migration_commit_failure_keeps_source_for_reopen_retry() {
        let directory = TestDirectory::new();
        let source = legacy_source();
        insert_legacy(
            &source,
            "legacy",
            "serendipity",
            entry("Serendipity", "legacy"),
            "2025",
            "2025",
        );
        {
            let mut store = AiDictionaryStore::new(directory.0.clone());
            store.open().unwrap();
            store.connection().unwrap().execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE import_commit_guard (
                    word TEXT REFERENCES ai_dictionary_entries(normalized_word) DEFERRABLE INITIALLY DEFERRED
                 );
                 CREATE TRIGGER reject_commit AFTER INSERT ON ai_dictionary_entries
                 BEGIN INSERT INTO import_commit_guard VALUES ('missing-word'); END;",
            ).unwrap();
            let error = store.initialise(&source).unwrap_err();
            assert!(error.contains("提交 AI 词典迁移失败"), "{error}");
            assert_eq!(legacy_count(&source), 1);
            assert!(store.find("serendipity").unwrap().is_none());
            store
                .connection()
                .unwrap()
                .execute_batch("DROP TRIGGER reject_commit; DROP TABLE import_commit_guard;")
                .unwrap();
        }
        let mut reopened = AiDictionaryStore::new(directory.0.clone());
        reopened.initialise(&source).unwrap();
        assert_eq!(legacy_count(&source), 0);
        assert!(reopened.find("serendipity").unwrap().is_some());
    }

    #[test]
    fn ai_dictionary_migration_commits_before_cleanup_and_existing_destination_wins_retry() {
        let directory = TestDirectory::new();
        let source = legacy_source();
        insert_legacy(
            &source,
            "legacy",
            "serendipity",
            entry("Serendipity", "legacy"),
            "2025",
            "2025",
        );
        source
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_cleanup BEFORE DELETE ON ai_dictionary_cache
             BEGIN SELECT RAISE(ABORT, 'fixture cleanup failure'); END;",
            )
            .unwrap();
        {
            let mut store = AiDictionaryStore::new(directory.0.clone());
            assert!(
                store
                    .initialise(&source)
                    .unwrap_err()
                    .contains("fixture cleanup failure")
            );
            assert_eq!(legacy_count(&source), 1);
            let observer = Connection::open(&store.path).unwrap();
            let committed: i64 = observer
                .query_row("SELECT COUNT(*) FROM ai_dictionary_entries", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                committed, 1,
                "another connection must see the committed destination"
            );
            store
                .save("serendipity", entry("Serendipity", "destination newer"))
                .unwrap();
        }
        source
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_cleanup")
            .unwrap();
        let mut reopened = AiDictionaryStore::new(directory.0.clone());
        reopened.initialise(&source).unwrap();
        assert_eq!(legacy_count(&source), 0);
        assert_eq!(
            reopened.find("serendipity").unwrap().unwrap().entry["headword_summary"],
            "destination newer"
        );
    }

    #[test]
    fn ai_dictionary_rejects_incomplete_entries_and_removes_corrupt_stored_entries() {
        let directory = TestDirectory::new();
        let source = Mutex::new(Connection::open_in_memory().unwrap());
        let mut store = AiDictionaryStore::new(directory.0.clone());
        store.initialise(&source).unwrap();
        assert!(
            store
                .save("serendipity", json!({"headword":"serendipity"}))
                .is_err()
        );
        assert!(store.find("serendipity").unwrap().is_none());
        store
            .save("serendipity", entry("Serendipity", "valid"))
            .unwrap();
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE ai_dictionary_entries SET canonical_word = 'different'",
                [],
            )
            .unwrap();
        assert!(store.find("serendipity").unwrap().is_none());
        let count: i64 = store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM ai_dictionary_entries", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn ai_dictionary_survives_shared_cache_clear_and_prune() {
        let directory = TestDirectory::new();
        let source = Mutex::new(Connection::open_in_memory().unwrap());
        db::migrate(&source.lock().unwrap()).unwrap();
        let mut store = AiDictionaryStore::new(directory.0.clone());
        store.initialise(&source).unwrap();
        store
            .save("serendipity", entry("Serendipity", "persistent"))
            .unwrap();
        {
            let connection = source.lock().unwrap();
            let provider = db::get_provider(&connection).unwrap();
            db::save_cache(
                &connection,
                &db::CacheRecord {
                    cache_key: "paragraph",
                    source_text: "Some text",
                    translated_text: "一些文字",
                    source_language: "en",
                    target_language: "zh-Hans",
                    provider: &provider,
                    prompt_id: &provider.prompt_id,
                    glossary_version: 1,
                },
            )
            .unwrap();
            assert!(db::get_cache_stats(&connection, 100).unwrap().entry_count > 0);
            db::prune_cache(&connection, 0).unwrap();
            db::clear_cache(&connection).unwrap();
            let stats = db::get_cache_stats(&connection, 100).unwrap();
            assert_eq!(stats.entry_count, 0);
            assert_eq!(stats.usage_bytes, 0);
        }
        assert_eq!(
            store.find("serendipity").unwrap().unwrap().entry["headword_summary"],
            "persistent"
        );
    }
}
