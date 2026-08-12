use crate::contracts::{
    DictionaryCommandResult, DictionaryDownloadProgress, DictionaryExtractProgress,
    DictionaryLookupResult, DictionaryState, DictionaryStatus, DictionaryUpdateCompleted,
    DictionaryUpdateFailed, DictionaryUpdateStarted, DictionaryVerifyProgress,
    DICTIONARY_DISTRIBUTION_SCHEMA_VERSION, DICTIONARY_SQLITE_SCHEMA_VERSION,
};
use crate::db::{self, DictionaryInstallationRecord};
use crate::AppState;
use chrono::Utc;
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use reqwest::Client;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::time::sleep;
use url::Url;

pub const DISTRIBUTION_DB_FILE: &str = "distribution.sqlite";
pub const DISTRIBUTION_ARCHIVE_PART_FILE: &str = "distribution.sqlite.gz.part";
pub const DISTRIBUTION_DB_PART_FILE: &str = "distribution.sqlite.part";
pub const DISTRIBUTION_DB_BACKUP_FILE: &str = "distribution.sqlite.backup";
pub const DICTIONARY_ARCHIVE_ASSET: &str = "distribution.sqlite.gz";
pub const DICTIONARY_CHECKSUM_ASSET: &str = "SHA256SUMS.txt";

const GITHUB_RELEASE_URL: &str =
    "https://api.github.com/repos/ahpxex/open-dictionary/releases/latest";
const MAX_DOWNLOAD_RETRIES: u32 = 2;
const PROGRESS_CHUNK: u64 = 4 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DictionaryError {
    #[error("词典未安装，请在设置中下载词典")]
    NotInstalled,
    #[error("查询词形不能为空")]
    EmptyInput,
    #[error("词典数据库不可读取：{0}")]
    DatabaseUnreadable(String),
    #[error("词典分发契约不匹配：{0}")]
    ContractMismatch(String),
    #[error("未找到词条：{0}")]
    NotFound(String),
}

#[derive(Debug, Clone)]
pub struct DistributionMetadata {
    pub distribution_schema_version: String,
    pub sqlite_schema_version: String,
    pub entry_count: i64,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
}

#[derive(Debug, Clone)]
struct ReleasePlan {
    tag: String,
    archive_url: String,
    archive_size: u64,
    checksums_url: String,
}

pub fn normalize_headword(word: &str) -> String {
    word.trim().to_lowercase()
}

fn open_read_only(path: &Path) -> Result<Connection, DictionaryError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))
}

fn database_path(dictionary_dir: &Path) -> PathBuf {
    dictionary_dir.join(DISTRIBUTION_DB_FILE)
}

fn metadata_value<'a>(metadata: &'a [(String, String)], key: &str) -> Option<&'a str> {
    metadata
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str())
}

fn read_metadata_rows(connection: &Connection) -> Result<Vec<(String, String)>, DictionaryError> {
    let mut statement = connection
        .prepare("SELECT key, value_json FROM metadata")
        .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))
}

fn metadata_string(rows: &[(String, String)], key: &str) -> Result<String, DictionaryError> {
    let raw = metadata_value(rows, key)
        .ok_or_else(|| DictionaryError::ContractMismatch(format!("metadata 缺少 {key}")))?;
    let value: Value = serde_json::from_str(raw).map_err(|error| {
        DictionaryError::ContractMismatch(format!("metadata.{key} 不是合法 JSON：{error}"))
    })?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| DictionaryError::ContractMismatch(format!("metadata.{key} 不是字符串")))
}

fn metadata_count(rows: &[(String, String)]) -> Result<i64, DictionaryError> {
    let raw = metadata_value(rows, "entry_count").ok_or_else(|| {
        DictionaryError::ContractMismatch("metadata 缺少 entry_count".to_string())
    })?;
    let value: Value = serde_json::from_str(raw).map_err(|error| {
        DictionaryError::ContractMismatch(format!("metadata.entry_count 不是合法 JSON：{error}"))
    })?;
    value.as_i64().ok_or_else(|| {
        DictionaryError::ContractMismatch("metadata.entry_count 不是整数".to_string())
    })
}

fn validate_contract(connection: &Connection) -> Result<DistributionMetadata, DictionaryError> {
    let required_tables = ["metadata", "entries"];
    for table in required_tables {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                )",
                [table],
                |row| row.get(0),
            )
            .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))?;
        if !exists {
            return Err(DictionaryError::ContractMismatch(format!(
                "SQLite 缺少 {table} 表"
            )));
        }
    }

    let rows = read_metadata_rows(connection)?;
    let distribution_schema_version = metadata_string(&rows, "distribution_schema_version")?;
    let sqlite_schema_version = metadata_string(&rows, "sqlite_schema_version")?;
    if distribution_schema_version != DICTIONARY_DISTRIBUTION_SCHEMA_VERSION {
        return Err(DictionaryError::ContractMismatch(format!(
            "distribution schema 应为 {DICTIONARY_DISTRIBUTION_SCHEMA_VERSION}，实际为 {distribution_schema_version}"
        )));
    }
    if sqlite_schema_version != DICTIONARY_SQLITE_SCHEMA_VERSION {
        return Err(DictionaryError::ContractMismatch(format!(
            "SQLite schema 应为 {DICTIONARY_SQLITE_SCHEMA_VERSION}，实际为 {sqlite_schema_version}"
        )));
    }

    let entry_count = metadata_count(&rows)?;
    let actual_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
        .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))?;
    if entry_count != actual_count {
        return Err(DictionaryError::ContractMismatch(format!(
            "metadata.entry_count 为 {entry_count}，实际词条数为 {actual_count}"
        )));
    }

    let invalid_schema_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM entries WHERE schema_version <> ?1",
            [DICTIONARY_DISTRIBUTION_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))?;
    if invalid_schema_rows > 0 {
        return Err(DictionaryError::ContractMismatch(format!(
            "entries 中有 {invalid_schema_rows} 条词条使用了错误的分发契约"
        )));
    }

    Ok(DistributionMetadata {
        distribution_schema_version,
        sqlite_schema_version,
        entry_count,
    })
}

fn validate_connection(connection: &Connection) -> Result<DistributionMetadata, DictionaryError> {
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))?;
    if integrity != "ok" {
        return Err(DictionaryError::DatabaseUnreadable(format!(
            "SQLite integrity_check 返回 {integrity}"
        )));
    }

    validate_contract(connection)
}

pub fn validate_distribution_database(
    path: &Path,
) -> Result<DistributionMetadata, DictionaryError> {
    if !path.is_file() {
        return Err(DictionaryError::NotInstalled);
    }
    let connection = open_read_only(path)?;
    validate_connection(&connection)
}

pub fn read_state(
    dictionary_dir: &Path,
    installation: Option<db::DictionaryInstallation>,
) -> DictionaryState {
    let path = database_path(dictionary_dir);
    let database_bytes = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut state = DictionaryState::not_installed();
    state.cache_size_bytes = database_bytes;

    let Some(installation) = installation else {
        if !path.is_file() {
            return state;
        }
        let state_result =
            open_read_only(&path).and_then(|connection| validate_contract(&connection));
        return match state_result {
            Ok(metadata) => DictionaryState {
                status: DictionaryStatus::Ready,
                installed_release: None,
                artifact_sha256: None,
                entry_count: Some(metadata.entry_count),
                distribution_schema_version: Some(metadata.distribution_schema_version),
                sqlite_schema_version: Some(metadata.sqlite_schema_version),
                installed_at: None,
                downloaded_bytes: 0,
                total_bytes: 0,
                database_bytes,
                cache_size_bytes: database_bytes,
                error: None,
            },
            Err(error) => DictionaryState {
                status: DictionaryStatus::Failed,
                error: Some(error.to_string()),
                ..state
            },
        };
    };

    state.installed_release = Some(installation.release_tag.clone());
    state.artifact_sha256 = Some(installation.artifact_sha256.clone());
    state.entry_count = Some(installation.entry_count);
    state.distribution_schema_version = Some(installation.distribution_schema_version.clone());
    state.sqlite_schema_version = Some(installation.sqlite_schema_version.clone());
    state.installed_at = Some(installation.installed_at.clone());
    state.downloaded_bytes = installation.compressed_bytes.max(0) as u64;
    state.total_bytes = state.downloaded_bytes;
    state.database_bytes = installation.database_bytes.max(0) as u64;
    state.cache_size_bytes = database_bytes;

    if !path.is_file() {
        state.error = Some("词典安装记录存在，但 distribution.sqlite 不存在".to_string());
        return state;
    }

    let state_result = open_read_only(&path).and_then(|connection| validate_contract(&connection));
    match state_result {
        Ok(metadata)
            if metadata.entry_count == installation.entry_count
                && metadata.distribution_schema_version
                    == installation.distribution_schema_version
                && metadata.sqlite_schema_version == installation.sqlite_schema_version =>
        {
            state.status = DictionaryStatus::Ready;
            state
        }
        Ok(_) => {
            state.status = DictionaryStatus::Failed;
            state.error = Some("词典安装记录与分发数据库不一致".to_string());
            state
        }
        Err(error) => {
            state.status = DictionaryStatus::Failed;
            state.error = Some(error.to_string());
            state
        }
    }
}

pub fn query(dictionary_dir: &Path, word: &str) -> Result<DictionaryLookupResult, DictionaryError> {
    let display_word = word.trim();
    if display_word.is_empty() {
        return Err(DictionaryError::EmptyInput);
    }
    let normalized_word = normalize_headword(display_word);
    let path = database_path(dictionary_dir);
    if !path.is_file() {
        return Err(DictionaryError::NotInstalled);
    }

    let connection = open_read_only(&path)?;
    validate_connection(&connection)?;
    let raw: Option<String> = connection
        .query_row(
            "SELECT document_json FROM entries
             WHERE headword_language_code = 'en' AND normalized_headword = ?1
             LIMIT 1",
            [&normalized_word],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| DictionaryError::DatabaseUnreadable(error.to_string()))?;
    let raw = raw.ok_or_else(|| DictionaryError::NotFound(display_word.to_string()))?;
    let entry: Value = serde_json::from_str(&raw).map_err(|error| {
        DictionaryError::DatabaseUnreadable(format!("词条 JSON 无法解析：{error}"))
    })?;
    if !entry.is_object() {
        return Err(DictionaryError::DatabaseUnreadable(
            "词条 JSON 不是对象".to_string(),
        ));
    }

    Ok(DictionaryLookupResult {
        word: display_word.to_string(),
        normalized_word,
        entry,
    })
}

pub fn parse_sha256_sums(manifest: &str, file_name: &str) -> Option<String> {
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let Some(hash) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(remainder) = parts.next().map(str::trim_start) else {
            continue;
        };
        let candidate = remainder.strip_prefix('*').unwrap_or(remainder).trim();
        if hash.len() == 64
            && hash.chars().all(|character| character.is_ascii_hexdigit())
            && candidate == file_name
        {
            return Some(hash.to_lowercase());
        }
    }
    None
}

fn validate_asset_url(raw: &str, asset_name: &str) -> Result<String, String> {
    let parsed = Url::parse(raw).map_err(|error| format!("{asset_name} 下载地址无效：{error}"))?;
    if parsed.scheme() != "https"
        || !matches!(
            parsed.host_str(),
            Some("github.com") | Some("release-assets.githubusercontent.com")
        )
    {
        return Err(format!("{asset_name} 下载地址不是受支持的 GitHub 地址"));
    }
    Ok(raw.to_string())
}

async fn fetch_release(client: &Client) -> Result<ReleasePlan, String> {
    let response = client
        .get(GITHUB_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("读取 GitHub Release 失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "读取 GitHub Release 失败：HTTP {}",
            response.status()
        ));
    }
    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|error| format!("GitHub Release 响应格式无法识别：{error}"))?;
    let archive = release
        .assets
        .iter()
        .find(|asset| asset.name == DICTIONARY_ARCHIVE_ASSET)
        .ok_or_else(|| format!("最新 Release 缺少 {DICTIONARY_ARCHIVE_ASSET}"))?;
    let checksums = release
        .assets
        .iter()
        .find(|asset| asset.name == DICTIONARY_CHECKSUM_ASSET)
        .ok_or_else(|| format!("最新 Release 缺少 {DICTIONARY_CHECKSUM_ASSET}"))?;
    Ok(ReleasePlan {
        tag: release.tag_name,
        archive_url: validate_asset_url(&archive.browser_download_url, DICTIONARY_ARCHIVE_ASSET)?,
        archive_size: archive.size.unwrap_or(0),
        checksums_url: validate_asset_url(
            &checksums.browser_download_url,
            DICTIONARY_CHECKSUM_ASSET,
        )?,
    })
}

async fn fetch_checksum_manifest(client: &Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("读取 SHA256SUMS.txt 失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "读取 SHA256SUMS.txt 失败：HTTP {}",
            response.status()
        ));
    }
    response
        .text()
        .await
        .map_err(|error| format!("读取 SHA256SUMS.txt 内容失败：{error}"))
}

fn emit_download_progress(app: &AppHandle, operation_id: &str, downloaded: u64, total: u64) {
    let _ = app.emit(
        "dictionary_download_progress",
        DictionaryDownloadProgress {
            operation_id: operation_id.to_string(),
            downloaded_bytes: downloaded,
            total_bytes: total,
        },
    );
}

async fn try_download_archive(
    client: &Client,
    app: &AppHandle,
    operation_id: &str,
    url: &str,
    path: &Path,
    expected_size: u64,
) -> Result<u64, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("下载词典工件失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载词典工件失败：HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(expected_size);
    let mut file = File::create(path).map_err(|error| format!("创建词典临时文件失败：{error}"))?;
    let mut stream = response.bytes_stream();
    let mut downloaded = 0_u64;
    let mut last_emit = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取词典下载内容失败：{error}"))?;
        file.write_all(&chunk)
            .map_err(|error| format!("写入词典临时文件失败：{error}"))?;
        downloaded += chunk.len() as u64;
        if downloaded.saturating_sub(last_emit) >= 1024 * 1024 || downloaded >= total {
            emit_download_progress(app, operation_id, downloaded, total);
            last_emit = downloaded;
        }
    }
    file.sync_all()
        .map_err(|error| format!("同步词典临时文件失败：{error}"))?;
    emit_download_progress(app, operation_id, downloaded, total);
    Ok(downloaded)
}

async fn download_archive(
    client: &Client,
    app: &AppHandle,
    operation_id: &str,
    url: &str,
    path: &Path,
    expected_size: u64,
) -> Result<u64, String> {
    let mut last_error = String::new();
    for attempt in 0..=MAX_DOWNLOAD_RETRIES {
        match try_download_archive(client, app, operation_id, url, path, expected_size).await {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                last_error = error;
                if attempt < MAX_DOWNLOAD_RETRIES {
                    sleep(Duration::from_secs(2_u64.pow(attempt))).await;
                }
            }
        }
    }
    Err(format!(
        "下载词典工件失败，已重试 {} 次：{}",
        MAX_DOWNLOAD_RETRIES + 1,
        last_error
    ))
}

fn sha256_file<F>(path: &Path, total: u64, mut progress: F) -> Result<String, String>
where
    F: FnMut(u64),
{
    let file = File::open(path).map_err(|error| format!("打开词典工件校验文件失败：{error}"))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut current = 0_u64;
    let mut last_emit = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("读取词典工件校验文件失败：{error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        current += read as u64;
        if current.saturating_sub(last_emit) >= PROGRESS_CHUNK || current >= total {
            progress(current);
            last_emit = current;
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn emit_verify_progress(app: &AppHandle, operation_id: &str, current: u64, total: u64) {
    let _ = app.emit(
        "dictionary_verify_progress",
        DictionaryVerifyProgress {
            operation_id: operation_id.to_string(),
            current,
            total,
        },
    );
}

struct CountingReader<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.count.fetch_add(read as u64, Ordering::Relaxed);
        Ok(read)
    }
}

fn gunzip_file<F>(
    archive_path: &Path,
    destination_path: &Path,
    total: u64,
    mut progress: F,
) -> Result<(), String>
where
    F: FnMut(u64),
{
    let archive =
        File::open(archive_path).map_err(|error| format!("打开词典压缩文件失败：{error}"))?;
    let compressed_read = Arc::new(AtomicU64::new(0));
    let counting_reader = CountingReader {
        inner: BufReader::new(archive),
        count: compressed_read.clone(),
    };
    let mut decoder = GzDecoder::new(counting_reader);
    let output = File::create(destination_path)
        .map_err(|error| format!("创建词典解压临时文件失败：{error}"))?;
    let mut writer = BufWriter::new(output);
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut last_emit = 0_u64;
    loop {
        let read = match decoder.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                drop(writer);
                let _ = fs::remove_file(destination_path);
                return Err(format!("解压词典工件失败：{error}"));
            }
        };
        if read == 0 {
            break;
        }
        if let Err(error) = writer.write_all(&buffer[..read]) {
            drop(writer);
            let _ = fs::remove_file(destination_path);
            return Err(format!("写入解压词典失败：{error}"));
        }
        let current = compressed_read.load(Ordering::Relaxed);
        if current.saturating_sub(last_emit) >= PROGRESS_CHUNK {
            progress(current);
            last_emit = current;
        }
    }
    writer
        .flush()
        .map_err(|error| format!("刷新解压词典失败：{error}"))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| format!("同步解压词典失败：{error}"))?;
    progress(total.max(compressed_read.load(Ordering::Relaxed)));
    Ok(())
}

fn rollback_promoted_database(
    current_path: &Path,
    backup_path: &Path,
    had_old_database: bool,
) -> Result<(), String> {
    if current_path.exists() {
        fs::remove_file(current_path).map_err(|error| format!("回滚词典新文件失败：{error}"))?;
    }
    if had_old_database && backup_path.exists() {
        fs::rename(backup_path, current_path)
            .map_err(|error| format!("恢复旧词典失败：{error}"))?;
    }
    Ok(())
}

fn promote_database(
    part_path: &Path,
    current_path: &Path,
    backup_path: &Path,
) -> Result<bool, String> {
    if backup_path.exists() {
        fs::remove_file(backup_path).map_err(|error| format!("清理旧词典备份失败：{error}"))?;
    }
    let had_old_database = current_path.is_file();
    if had_old_database {
        fs::rename(current_path, backup_path)
            .map_err(|error| format!("暂存当前词典失败：{error}"))?;
    }
    if let Err(error) = fs::rename(part_path, current_path) {
        if had_old_database {
            let _ = fs::rename(backup_path, current_path);
        }
        return Err(format!("提升新词典失败：{error}"));
    }
    Ok(had_old_database)
}

async fn update_impl(
    app: &AppHandle,
    state: &AppState,
    dictionary_dir: &Path,
    operation_id: &str,
) -> Result<DictionaryState, String> {
    fs::create_dir_all(dictionary_dir).map_err(|error| format!("创建词典目录失败：{error}"))?;
    let archive_path = dictionary_dir.join(DISTRIBUTION_ARCHIVE_PART_FILE);
    let part_path = dictionary_dir.join(DISTRIBUTION_DB_PART_FILE);
    let current_path = database_path(dictionary_dir);
    let backup_path = dictionary_dir.join(DISTRIBUTION_DB_BACKUP_FILE);
    let _ = fs::remove_file(&archive_path);
    let _ = fs::remove_file(&part_path);

    let client = Client::builder()
        .user_agent("Lilt/0.1.0")
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|error| format!("创建词典网络客户端失败：{error}"))?;
    let release = fetch_release(&client).await?;
    let manifest = fetch_checksum_manifest(&client, &release.checksums_url).await?;
    let expected_sha256 = parse_sha256_sums(&manifest, DICTIONARY_ARCHIVE_ASSET)
        .ok_or_else(|| format!("SHA256SUMS.txt 缺少 {DICTIONARY_ARCHIVE_ASSET} 的校验值"))?;
    let compressed_bytes = download_archive(
        &client,
        app,
        operation_id,
        &release.archive_url,
        &archive_path,
        release.archive_size,
    )
    .await?;

    let verify_app = app.clone();
    let verify_operation_id = operation_id.to_string();
    let verify_path = archive_path.clone();
    let actual_sha256 = tauri::async_runtime::spawn_blocking(move || {
        sha256_file(&verify_path, compressed_bytes, |current| {
            emit_verify_progress(&verify_app, &verify_operation_id, current, compressed_bytes);
        })
    })
    .await
    .map_err(|error| format!("校验词典工件任务失败：{error}"))??;
    if actual_sha256 != expected_sha256 {
        return Err(format!(
            "词典工件 SHA-256 校验失败：期望 {expected_sha256}，实际 {actual_sha256}"
        ));
    }

    let extract_app = app.clone();
    let extract_operation_id = operation_id.to_string();
    let extract_archive_path = archive_path.clone();
    let extract_part_path = part_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        gunzip_file(
            &extract_archive_path,
            &extract_part_path,
            compressed_bytes,
            |current| {
                let _ = extract_app.emit(
                    "dictionary_extract_progress",
                    DictionaryExtractProgress {
                        operation_id: extract_operation_id.clone(),
                        current,
                        total: compressed_bytes,
                    },
                );
            },
        )
    })
    .await
    .map_err(|error| format!("解压词典工件任务失败：{error}"))??;

    let metadata = validate_distribution_database(&part_path)
        .map_err(|error| format!("解压后的词典校验失败：{error}"))?;
    let database_bytes = fs::metadata(&part_path)
        .map_err(|error| format!("读取解压词典大小失败：{error}"))?
        .len();
    let had_old_database = promote_database(&part_path, &current_path, &backup_path)?;

    let installed_at = Utc::now().to_rfc3339();
    let installation = DictionaryInstallationRecord {
        release_tag: &release.tag,
        artifact_sha256: &expected_sha256,
        installed_at: &installed_at,
        entry_count: metadata.entry_count,
        distribution_schema_version: &metadata.distribution_schema_version,
        sqlite_schema_version: &metadata.sqlite_schema_version,
        compressed_bytes: compressed_bytes as i64,
        database_bytes: database_bytes as i64,
    };
    let persist_result = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())
        .and_then(|connection| db::save_dictionary_installation(&connection, &installation));
    if let Err(error) = persist_result {
        let rollback = rollback_promoted_database(&current_path, &backup_path, had_old_database);
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback_error) => format!("{error}；{rollback_error}"),
        });
    }

    let _ = fs::remove_file(&backup_path);
    let _ = fs::remove_file(&archive_path);
    let installation = db::DictionaryInstallation {
        release_tag: release.tag,
        artifact_sha256: expected_sha256,
        installed_at,
        entry_count: metadata.entry_count,
        distribution_schema_version: metadata.distribution_schema_version,
        sqlite_schema_version: metadata.sqlite_schema_version,
        compressed_bytes: compressed_bytes as i64,
        database_bytes: database_bytes as i64,
    };
    Ok(read_state(dictionary_dir, Some(installation)))
}

pub async fn update_dictionary(
    app: AppHandle,
    state: &AppState,
    dictionary_dir: &Path,
    operation_id: String,
) -> Result<DictionaryCommandResult, String> {
    let mut starting_state = state
        .database
        .lock()
        .ok()
        .and_then(|connection| db::get_dictionary_installation(&connection).ok())
        .map(|installation| read_state(dictionary_dir, installation))
        .unwrap_or_else(DictionaryState::not_installed);
    starting_state.status = DictionaryStatus::Updating;
    starting_state.error = None;
    let _ = app.emit(
        "dictionary_update_started",
        DictionaryUpdateStarted {
            operation_id: operation_id.clone(),
            state: starting_state,
        },
    );
    let result = update_impl(&app, state, dictionary_dir, &operation_id).await;
    if result.is_err() {
        let _ = fs::remove_file(dictionary_dir.join(DISTRIBUTION_ARCHIVE_PART_FILE));
        let _ = fs::remove_file(dictionary_dir.join(DISTRIBUTION_DB_PART_FILE));
    }
    match result {
        Ok(state) => {
            let _ = app.emit(
                "dictionary_update_completed",
                DictionaryUpdateCompleted {
                    operation_id: operation_id.clone(),
                    state: state.clone(),
                },
            );
            Ok(DictionaryCommandResult {
                operation_id,
                state,
            })
        }
        Err(error) => {
            let _ = app.emit(
                "dictionary_update_failed",
                DictionaryUpdateFailed {
                    operation_id: operation_id.clone(),
                    message: error.clone(),
                },
            );
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("lilt-dictionary-test-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn create_fixture(directory: &Path) {
        let connection = Connection::open(directory.join(DISTRIBUTION_DB_FILE)).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY, value_json TEXT NOT NULL);
                 CREATE TABLE entries (
                    entry_id TEXT PRIMARY KEY,
                    schema_version TEXT NOT NULL,
                    headword TEXT NOT NULL,
                    normalized_headword TEXT NOT NULL,
                    headword_language_code TEXT NOT NULL,
                    headword_language_name TEXT NOT NULL,
                    definition_language_code TEXT NOT NULL,
                    definition_language_name TEXT NOT NULL,
                    entry_type TEXT NOT NULL,
                    headword_summary TEXT NOT NULL,
                    memory_hook TEXT NOT NULL,
                    etymology_note TEXT,
                    study_notes_json TEXT NOT NULL,
                    document_json TEXT NOT NULL
                 );
                 CREATE INDEX entries_lookup_idx
                    ON entries (headword_language_code, normalized_headword);",
            )
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO metadata (key, value_json) VALUES
                    ('distribution_schema_version', '\"distribution_entry_v5\"'),
                    ('sqlite_schema_version', '\"distribution_sqlite_v1\"'),
                    ('entry_count', '1');",
            )
            .unwrap();
        let document = serde_json::json!({
            "schema_version": "distribution_entry_v5",
            "entry_id": "fixture-resolve",
            "headword": "resolve",
            "normalized_headword": "resolve",
            "headword_language": {"code": "en", "name": "English"},
            "definition_language": {"code": "zh-Hans", "name": "Chinese (Simplified)"},
            "entry_type": "standard",
            "headword_summary": "解决",
            "memory_hook": "",
            "study_notes": [],
            "etymology_note": null,
            "etymologies": [],
            "pos_groups": [{
                "pos": "verb",
                "etymology_id": null,
                "proper_name": false,
                "summary": "解决问题",
                "usage_note": null,
                "forms": [],
                "pronunciations": [{"ipa": "rɪˈzɑːlv", "text": null, "tags": []}],
                "relations": [{"type": "synonym", "word": "solve", "lang_code": "en"}],
                "meanings": [{
                    "sense_id": "s1",
                    "priority": "core",
                    "short_gloss": "解决",
                    "learner_explanation": "使问题得到解决",
                    "usage_note": null,
                    "labels": [],
                    "topics": [],
                    "examples": [{"text": "Resolve the issue.", "translation": "解决这个问题。"}]
                }]
            }]
        });
        connection
            .execute(
                "INSERT INTO entries VALUES (?1, ?2, ?3, ?4, 'en', 'English', 'zh-Hans',
                    'Chinese (Simplified)', 'standard', '解决', '', NULL, '[]', ?5)",
                params![
                    "fixture-resolve",
                    DICTIONARY_DISTRIBUTION_SCHEMA_VERSION,
                    "resolve",
                    "resolve",
                    document.to_string(),
                ],
            )
            .unwrap();
    }

    fn remove_directory(directory: &Path) {
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn normalizes_trimmed_headwords_case_insensitively() {
        assert_eq!(normalize_headword("  ReSoLvE  "), "resolve");
        assert_eq!(normalize_headword("Ä"), "ä");
    }

    #[test]
    fn parses_sha256sum_manifest_for_the_exact_asset() {
        let manifest = "deadbeef\n\
b8031d1b2e39019a520bcea3a7b84d5dff7df026f13310e5d8dc0eb044df07f3  *distribution.sqlite.gz\n";
        assert_eq!(
            parse_sha256_sums(manifest, DICTIONARY_ARCHIVE_ASSET),
            Some("b8031d1b2e39019a520bcea3a7b84d5dff7df026f13310e5d8dc0eb044df07f3".to_string())
        );
        assert_eq!(parse_sha256_sums(manifest, "other.gz"), None);
    }

    #[test]
    fn queries_fixture_read_only_and_case_insensitively() {
        let directory = temporary_directory();
        create_fixture(&directory);
        let result = query(&directory, "  RESOLVE ").unwrap();
        assert_eq!(result.word, "RESOLVE");
        assert_eq!(result.normalized_word, "resolve");
        assert_eq!(
            result.entry["schema_version"],
            DICTIONARY_DISTRIBUTION_SCHEMA_VERSION
        );
        remove_directory(&directory);
    }

    #[test]
    fn distinguishes_empty_missing_and_not_found_queries() {
        let directory = temporary_directory();
        assert_eq!(
            query(&directory, " ").unwrap_err(),
            DictionaryError::EmptyInput
        );
        assert_eq!(
            query(&directory, "resolve").unwrap_err(),
            DictionaryError::NotInstalled
        );
        create_fixture(&directory);
        assert_eq!(
            query(&directory, "missing").unwrap_err(),
            DictionaryError::NotFound("missing".to_string())
        );
        remove_directory(&directory);
    }

    #[test]
    fn rejects_corrupt_or_mismatched_distribution_database() {
        let directory = temporary_directory();
        fs::write(directory.join(DISTRIBUTION_DB_FILE), b"not sqlite").unwrap();
        assert!(matches!(
            validate_distribution_database(&database_path(&directory)),
            Err(DictionaryError::DatabaseUnreadable(_))
        ));
        remove_directory(&directory);

        let directory = temporary_directory();
        create_fixture(&directory);
        let connection = Connection::open(directory.join(DISTRIBUTION_DB_FILE)).unwrap();
        connection
            .execute(
                "UPDATE metadata SET value_json = '\"wrong\"' WHERE key = 'sqlite_schema_version'",
                [],
            )
            .unwrap();
        assert!(matches!(
            validate_distribution_database(&database_path(&directory)),
            Err(DictionaryError::ContractMismatch(_))
        ));
        remove_directory(&directory);
    }

    #[test]
    fn query_rejects_a_database_with_failed_integrity_check() {
        let directory = temporary_directory();
        create_fixture(&directory);
        let path = database_path(&directory);
        let (page_size, index_root_page): (i64, i64) = {
            let connection = Connection::open(&path).unwrap();
            let page_size = connection
                .query_row("PRAGMA page_size", [], |row| row.get(0))
                .unwrap();
            let index_root_page = connection
                .query_row(
                    "SELECT rootpage FROM sqlite_master WHERE name = 'entries_lookup_idx'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            (page_size, index_root_page)
        };

        let mut bytes = fs::read(&path).unwrap();
        let page_offset = ((index_root_page - 1) * page_size) as usize;
        bytes[page_offset] = 0;
        fs::write(&path, bytes).unwrap();

        assert!(matches!(
            query(&directory, "resolve"),
            Err(DictionaryError::DatabaseUnreadable(_))
        ));
        remove_directory(&directory);
    }

    #[test]
    fn gzip_and_hash_helpers_use_staging_files() {
        let directory = temporary_directory();
        let source = directory.join("source.txt");
        let archive = directory.join("archive.gz");
        let extracted = directory.join("distribution.sqlite.part");
        fs::write(&source, b"dictionary fixture").unwrap();
        {
            let input = File::open(&source).unwrap();
            let output = File::create(&archive).unwrap();
            let mut encoder = flate2::write::GzEncoder::new(output, flate2::Compression::default());
            let mut input = BufReader::new(input);
            std::io::copy(&mut input, &mut encoder).unwrap();
            encoder.finish().unwrap();
        }
        let expected =
            sha256_file(&archive, fs::metadata(&archive).unwrap().len(), |_| {}).unwrap();
        let mut actual = String::new();
        gunzip_file(
            &archive,
            &extracted,
            fs::metadata(&archive).unwrap().len(),
            |_| {},
        )
        .unwrap();
        let mut reader = File::open(&extracted).unwrap();
        reader.read_to_string(&mut actual).unwrap();
        assert_eq!(expected.len(), 64);
        assert_eq!(actual, "dictionary fixture");
        remove_directory(&directory);
    }

    #[test]
    #[ignore = "requires a real open-dictionary distribution.sqlite"]
    fn smoke_real_distribution_lookup() {
        let directory = std::env::var("LILT_DICTIONARY_DIR")
            .expect("LILT_DICTIONARY_DIR must point to a dictionary cache directory");
        let result = query(Path::new(&directory), "RESOLVE").unwrap();
        assert_eq!(
            result.entry["schema_version"],
            DICTIONARY_DISTRIBUTION_SCHEMA_VERSION
        );
        assert_eq!(result.entry["headword"], "resolve");
        let metadata =
            validate_distribution_database(&database_path(Path::new(&directory))).unwrap();
        assert!(metadata.entry_count > 20_000);
    }
}
