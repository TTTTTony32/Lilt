use super::{DictionaryError, load_form_index, normalize_headword};
use crate::contracts::{DICTIONARY_DISTRIBUTION_SCHEMA_VERSION, DICTIONARY_SQLITE_SCHEMA_VERSION};
use crate::{db, diagnostics};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub(super) const INDEX_FILE: &str = "form-index.json";
const FORMAT_VERSION: u32 = 1;
// Bump when the SQL, normalization, sorting, or deduplication rules change.
const ALGORITHM_VERSION: u32 = 1;

type FormIndex = HashMap<String, Vec<(String, String)>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileFingerprint {
    length: u64,
    modified_seconds: u64,
    modified_nanoseconds: u32,
}

fn file_fingerprint(path: &Path) -> Result<FileFingerprint, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("dictionary source is not a file".to_string());
    }
    let modified = metadata
        .modified()
        .map_err(|error| error.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    Ok(FileFingerprint {
        length: metadata.len(),
        modified_seconds: modified.as_secs(),
        modified_nanoseconds: modified.subsec_nanos(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstallationIdentity {
    release_tag: String,
    artifact_sha256: String,
    distribution_schema_version: String,
    sqlite_schema_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SourceIdentity {
    installation: Option<InstallationIdentity>,
    file: FileFingerprint,
    distribution_schema_version: String,
    sqlite_schema_version: String,
}

impl SourceIdentity {
    fn capture(
        path: &Path,
        installation: Option<&db::DictionaryInstallation>,
    ) -> Result<Self, String> {
        Ok(Self {
            installation: installation.map(|record| InstallationIdentity {
                release_tag: record.release_tag.clone(),
                artifact_sha256: record.artifact_sha256.clone(),
                distribution_schema_version: record.distribution_schema_version.clone(),
                sqlite_schema_version: record.sqlite_schema_version.clone(),
            }),
            file: file_fingerprint(path)?,
            distribution_schema_version: DICTIONARY_DISTRIBUTION_SCHEMA_VERSION.to_string(),
            sqlite_schema_version: DICTIONARY_SQLITE_SCHEMA_VERSION.to_string(),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Snapshot<I> {
    format_version: u32,
    algorithm_version: u32,
    identity: SourceIdentity,
    key_count: usize,
    candidate_count: usize,
    index: I,
}

fn read_cached(path: &Path, expected: &SourceIdentity) -> Result<FormIndex, String> {
    let bytes = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "missing".to_string()
        } else {
            format!("read_failed: {error}")
        }
    })?;
    let snapshot: Snapshot<FormIndex> =
        serde_json::from_slice(&bytes).map_err(|error| format!("decode_failed: {error}"))?;
    if snapshot.format_version != FORMAT_VERSION {
        return Err("format_version_mismatch".to_string());
    }
    if snapshot.algorithm_version != ALGORITHM_VERSION {
        return Err("algorithm_version_mismatch".to_string());
    }
    if &snapshot.identity != expected {
        return Err("source_identity_mismatch".to_string());
    }
    if snapshot.key_count != snapshot.index.len()
        || snapshot.candidate_count != snapshot.index.values().map(Vec::len).sum::<usize>()
        || snapshot.index.iter().any(|(form, candidates)| {
            form.is_empty()
                || normalize_headword(form) != *form
                || candidates.is_empty()
                || candidates
                    .iter()
                    .any(|(word, normalized)| word.trim().is_empty() || normalized.is_empty())
                || candidates.windows(2).any(|pair| pair[0].1 >= pair[1].1)
        })
    {
        return Err("invalid_index_structure".to_string());
    }
    Ok(snapshot.index)
}

fn temporary_path(index_path: &Path) -> PathBuf {
    index_path.with_extension("json.part")
}

pub(super) fn cleanup_temporary(index_path: &Path) {
    if let Err(error) = fs::remove_file(temporary_path(index_path)) {
        if error.kind() != std::io::ErrorKind::NotFound {
            diagnostics::warn(format!(
                "dictionary.form_index.cleanup_failed error={error}"
            ));
        }
    }
}

fn save(index_path: &Path, identity: &SourceIdentity, index: &FormIndex) -> Result<(), String> {
    let part_path = temporary_path(index_path);
    let result = (|| {
        let file = File::create(&part_path).map_err(|error| error.to_string())?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(
            &mut writer,
            &Snapshot {
                format_version: FORMAT_VERSION,
                algorithm_version: ALGORITHM_VERSION,
                identity: identity.clone(),
                key_count: index.len(),
                candidate_count: index.values().map(Vec::len).sum::<usize>(),
                index,
            },
        )
        .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|error| error.to_string())?;
        drop(writer);

        // Windows rename cannot replace an existing file. The complete, synced
        // part is published after removal; interruption in this gap is a cache miss.
        #[cfg(windows)]
        if index_path.is_file() {
            fs::remove_file(index_path).map_err(|error| error.to_string())?;
        }
        fs::rename(&part_path, index_path).map_err(|error| error.to_string())
    })();
    cleanup_temporary(index_path);
    result
}

pub(super) fn save_if_current(
    database_path: &Path,
    index_path: &Path,
    identity: &SourceIdentity,
    index: &FormIndex,
    origin: &str,
) -> Duration {
    let started = Instant::now();
    let result = match file_fingerprint(database_path) {
        Ok(fingerprint) if fingerprint == identity.file => save(index_path, identity, index),
        Ok(_) => Err("source_changed_before_save".to_string()),
        Err(error) => Err(format!("source_fingerprint_unavailable: {error}")),
    };
    let elapsed = started.elapsed();
    if let Err(error) = result {
        diagnostics::warn(format!(
            "dictionary.form_index.save_failed origin={origin} save_ms={:.3} error={error}",
            elapsed.as_secs_f64() * 1000.0
        ));
    } else {
        diagnostics::info(format!(
            "dictionary.form_index.saved origin={origin} save_ms={:.3}",
            elapsed.as_secs_f64() * 1000.0
        ));
    }
    elapsed
}

#[derive(Debug)]
pub(super) struct LoadMeasurement {
    pub source: &'static str,
    pub miss_reason: Option<String>,
    pub load_elapsed: Duration,
    pub build_elapsed: Duration,
    pub save_elapsed: Duration,
    pub key_count: usize,
    pub candidate_count: usize,
}

pub(super) struct LoadedIndex {
    pub index: FormIndex,
    pub identity: Option<SourceIdentity>,
    pub pending_save: bool,
    #[cfg(test)]
    pub measurement: LoadMeasurement,
}

pub(super) fn load_or_build(
    connection: &Connection,
    database_path: &Path,
    index_path: &Path,
    installation: Option<&db::DictionaryInstallation>,
    origin: &str,
    publish: bool,
) -> Result<LoadedIndex, DictionaryError> {
    let load_started = Instant::now();
    let captured = SourceIdentity::capture(database_path, installation);
    let cached = match &captured {
        Ok(identity) => read_cached(index_path, identity).and_then(|index| {
            if matches!(file_fingerprint(database_path), Ok(current) if current == identity.file) {
                Ok(index)
            } else {
                Err("source_changed_during_load".to_string())
            }
        }),
        Err(error) => Err(format!("source_fingerprint_unavailable: {error}")),
    };
    let load_elapsed = load_started.elapsed();
    let identity = captured.ok();
    let (index, source, miss_reason, build_elapsed) = match cached {
        Ok(index) => (index, "cache_hit", None, Duration::ZERO),
        Err(reason) => {
            let started = Instant::now();
            let index = load_form_index(connection)?;
            (index, "rebuilt", Some(reason), started.elapsed())
        }
    };
    let pending_save = source == "rebuilt" && identity.is_some();
    let save_elapsed = if publish && pending_save {
        save_if_current(
            database_path,
            index_path,
            identity.as_ref().unwrap(),
            &index,
            origin,
        )
    } else {
        Duration::ZERO
    };
    let measurement = LoadMeasurement {
        source,
        miss_reason,
        load_elapsed,
        build_elapsed,
        save_elapsed,
        key_count: index.len(),
        candidate_count: index.values().map(Vec::len).sum(),
    };
    diagnostics::info(format!(
        "dictionary.form_index.ready origin={origin} source={} miss_reason={:?} load_ms={:.3} build_ms={:.3} save_ms={:.3} key_count={} candidate_count={} publication={}",
        measurement.source,
        measurement.miss_reason,
        measurement.load_elapsed.as_secs_f64() * 1000.0,
        measurement.build_elapsed.as_secs_f64() * 1000.0,
        measurement.save_elapsed.as_secs_f64() * 1000.0,
        measurement.key_count,
        measurement.candidate_count,
        if publish { "enabled" } else { "deferred" },
    ));
    Ok(LoadedIndex {
        index,
        identity,
        pending_save: pending_save && !publish,
        #[cfg(test)]
        measurement,
    })
}
