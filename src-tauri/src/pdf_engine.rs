#[cfg(not(debug_assertions))]
use futures_util::StreamExt;
#[cfg(not(debug_assertions))]
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::Read;
#[cfg(not(debug_assertions))]
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;
#[cfg(not(debug_assertions))]
use zip::ZipArchive;

use crate::AppState;

pub(crate) const BABELDOC_ENGINE_VERSION: &str = "babeldoc-0.6.4";
const BABELDOC_VERSION: &str = "0.6.4";
const SUPPORTED_ENGINE_TARGET: &str = "windows-x86_64";
#[cfg(not(debug_assertions))]
const PDF_ENGINE_RELEASE_TAG: &str = "lilt-pdf-engine-babeldoc-0.6.4-r2";
#[cfg(not(debug_assertions))]
const RELEASE_REPOSITORY_OWNER: &str = "TTTTTony32";
#[cfg(not(debug_assertions))]
const RELEASE_REPOSITORY_NAME: &str = "Lilt";
#[cfg(not(debug_assertions))]
const MAX_RELEASE_INDEX_BYTES: usize = 2 * 1024 * 1024;
#[cfg(not(debug_assertions))]
const MAX_ENGINE_ARCHIVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
#[cfg(not(debug_assertions))]
const MAX_ENGINE_UNPACKED_BYTES: u64 = 12 * 1024 * 1024 * 1024;
#[cfg(not(debug_assertions))]
const MAX_ENGINE_ARCHIVE_ENTRIES: usize = 100_000;
#[cfg(not(debug_assertions))]
const EXPECTED_ARCHIVE_ROOT: &str = BABELDOC_ENGINE_VERSION;
#[cfg(debug_assertions)]
const PDFMATH_TRANSLATE_REVISION: &str = "f8dffcf4c3a33b254391d43514439b975ce8d966";
const ENGINE_MANIFEST_NAME: &str = "runtime.json";
const ENGINE_PARENT: &str = "engines/pdf";
#[cfg(debug_assertions)]
const WORKER_RELATIVE_PATH: &str = "pdf-worker/worker.py";

#[cfg(not(debug_assertions))]
fn release_index_url() -> String {
    format!(
        "https://github.com/{RELEASE_REPOSITORY_OWNER}/{RELEASE_REPOSITORY_NAME}/releases/download/{PDF_ENGINE_RELEASE_TAG}/pdf-engine-index.json"
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfEngineStatus {
    pub status: String,
    pub engine_version: String,
    pub target: String,
    pub python_version: Option<String>,
    pub babeldoc_version: Option<String>,
    pub distribution_version: Option<String>,
    pub resource_size_bytes: Option<u64>,
    pub updating: bool,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EngineManifest {
    engine_version: String,
    target: String,
    python: String,
    worker: String,
    python_version: String,
    babeldoc_version: String,
    pdfmathtranslate_revision: String,
    #[serde(default)]
    distribution_version: Option<String>,
    #[serde(default)]
    resource_count: Option<usize>,
    #[serde(default)]
    resource_size_bytes: Option<u64>,
    #[serde(default)]
    resources: Vec<EngineResource>,
    #[serde(default)]
    licenses: Vec<EngineLicense>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EngineResource {
    Path(String),
    Descriptor {
        path: String,
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default)]
        size: Option<u64>,
        #[serde(default = "default_required")]
        required: bool,
    },
}

#[derive(Debug, Deserialize)]
struct EngineLicense {
    name: String,
    license: String,
    source: String,
    #[serde(default)]
    files: Vec<String>,
}

#[cfg(not(debug_assertions))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PdfEngineIndex {
    schema_version: u32,
    engine_version: String,
    distribution_version: String,
    assets: std::collections::HashMap<String, PdfEngineReleaseAsset>,
}

#[cfg(not(debug_assertions))]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PdfEngineReleaseAsset {
    url: String,
    sha256: String,
    size: u64,
    manifest_sha256: String,
}

#[derive(Debug, Clone)]
struct PdfEngineRuntime {
    root: PathBuf,
    python: PathBuf,
    worker: PathBuf,
    python_version: String,
    babeldoc_version: String,
    distribution_version: Option<String>,
    resource_count: usize,
    resource_size_bytes: Option<u64>,
}

impl PdfEngineRuntime {
    #[cfg(test)]
    fn load(data_dir: &Path) -> Result<Self, String> {
        let (data_root, root) = canonical_engine_root(data_dir)?;
        Self::load_canonical_root(root, Some(&data_root), true)
    }

    fn load_lightweight(data_dir: &Path) -> Result<Self, String> {
        let (data_root, root) = canonical_engine_root(data_dir)?;
        Self::load_canonical_root(root, Some(&data_root), false)
    }

    fn load_from_root(root: PathBuf) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("PDF Engine 目录不存在：{error}"))?;
        Self::load_canonical_root(root, None, true)
    }

    fn load_canonical_root(
        root: PathBuf,
        data_root: Option<&Path>,
        verify_resource_digests: bool,
    ) -> Result<Self, String> {
        if let Some(data_root) = data_root {
            if !root.starts_with(data_root) {
                return Err("PDF Engine 目录不在应用数据目录内".to_string());
            }
        }
        if !root.is_dir() {
            return Err("PDF Engine 目录不是文件夹".to_string());
        }

        let manifest_path = root.join(ENGINE_MANIFEST_NAME);
        let manifest_metadata = fs::symlink_metadata(&manifest_path)
            .map_err(|error| format!("读取 PDF Engine manifest 失败：{error}"))?;
        if !manifest_metadata.file_type().is_file() {
            return Err("PDF Engine manifest 不是普通文件".to_string());
        }
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("读取 PDF Engine manifest 失败：{error}"))?;
        let manifest: EngineManifest = serde_json::from_str(&manifest_text)
            .map_err(|error| format!("解析 PDF Engine manifest 失败：{error}"))?;
        validate_manifest(&manifest)?;

        let python = resolve_runtime_file(&root, &manifest.python, "Python")?;
        let worker = resolve_runtime_file(&root, &manifest.worker, "Worker")?;
        if verify_resource_digests {
            for resource in &manifest.resources {
                validate_resource(&root, resource)?;
            }
        }
        validate_licenses(&root, &manifest.licenses)?;
        let resource_size_bytes = match manifest.resource_size_bytes {
            Some(value) => value,
            None => manifest
                .resources
                .iter()
                .filter_map(resource_declared_size)
                .try_fold(0_u64, |total, size| total.checked_add(size))
                .ok_or_else(|| "PDF Engine 资源体积超出可表示范围".to_string())?,
        };
        Ok(Self {
            root,
            python,
            worker,
            python_version: manifest.python_version,
            babeldoc_version: manifest.babeldoc_version,
            distribution_version: manifest.distribution_version,
            resource_count: manifest.resource_count.unwrap_or(manifest.resources.len()),
            resource_size_bytes: (resource_size_bytes > 0).then_some(resource_size_bytes),
        })
    }

    fn python_command(&self) -> Command {
        let python_home = self
            .python
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        let mut command = Command::new(&self.python);
        command
            .current_dir(&self.root)
            .env("PYTHONHOME", python_home)
            .env("PYTHONNOUSERSITE", "1")
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            .env("HOMEDRIVE", &self.root)
            .env("HOMEPATH", &self.root)
            .env("XDG_CACHE_HOME", self.root.join(".cache"))
            .env(
                "TIKTOKEN_CACHE_DIR",
                self.root.join(".cache/babeldoc/tiktoken"),
            )
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONUSERBASE")
            .env_remove("VIRTUAL_ENV");
        command
    }

    fn command(&self) -> Command {
        let mut command = self.python_command();
        command.arg("-s").arg(&self.worker);
        command
    }

    fn log_summary(&self) {
        crate::diagnostics::info(format!(
            "pdf.engine.ready python_version={} babeldoc_version={} resource_count={}",
            self.python_version, self.babeldoc_version, self.resource_count
        ));
    }

    fn status(&self) -> PdfEngineStatus {
        PdfEngineStatus {
            status: "available".to_string(),
            engine_version: BABELDOC_ENGINE_VERSION.to_string(),
            target: current_target(),
            python_version: Some(self.python_version.clone()),
            babeldoc_version: Some(self.babeldoc_version.clone()),
            distribution_version: self.distribution_version.clone(),
            resource_size_bytes: self.resource_size_bytes,
            updating: false,
            error: None,
        }
    }
}

fn validate_manifest(manifest: &EngineManifest) -> Result<(), String> {
    if manifest.engine_version != BABELDOC_ENGINE_VERSION {
        return Err(format!(
            "PDF Engine 版本不匹配：需要 {}，实际 {}",
            BABELDOC_ENGINE_VERSION, manifest.engine_version
        ));
    }
    if manifest.babeldoc_version != BABELDOC_VERSION {
        return Err(format!(
            "BabelDOC 版本不匹配：需要 {}，实际 {}",
            BABELDOC_VERSION, manifest.babeldoc_version
        ));
    }
    if manifest.python_version.trim().is_empty() {
        return Err("PDF Engine manifest 缺少 Python 版本".to_string());
    }
    if manifest.pdfmathtranslate_revision.trim().is_empty() {
        return Err("PDF Engine manifest 缺少 PDFMathTranslate 版本".to_string());
    }
    if manifest
        .distribution_version
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err("PDF Engine manifest 的分发修订号不能为空".to_string());
    }
    let expected_target = current_target();
    ensure_supported_target(&expected_target)?;
    if manifest.target != expected_target {
        return Err(format!(
            "PDF Engine 架构不匹配：需要 {}，实际 {}",
            expected_target, manifest.target
        ));
    }
    Ok(())
}

pub(crate) fn build_worker_command(data_dir: &Path) -> Result<Command, String> {
    let runtime = PdfEngineRuntime::load_lightweight(data_dir)?;
    runtime.log_summary();
    Ok(configure_command(runtime.command()))
}

#[cfg(test)]
pub(crate) fn build_worker_command_from_root(root: &Path) -> Result<Command, String> {
    let runtime = PdfEngineRuntime::load_from_root(root.to_path_buf())?;
    runtime.log_summary();
    Ok(configure_command(runtime.command()))
}

fn configure_command(mut command: Command) -> Command {
    command
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn canonical_engine_root(data_dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    if !data_dir.is_absolute() {
        return Err("应用数据目录必须是绝对路径".to_string());
    }
    let data_root = data_dir
        .canonicalize()
        .map_err(|error| format!("应用数据目录不可用：{error}"))?;
    let root = data_root.join(ENGINE_PARENT).join(BABELDOC_ENGINE_VERSION);
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("PDF Engine 目录不存在：{error}"))?;
    if !canonical_root.starts_with(&data_root) {
        return Err("PDF Engine 目录不在应用数据目录内".to_string());
    }
    Ok((data_root, canonical_root))
}

fn resolve_runtime_file(root: &Path, relative: &str, label: &str) -> Result<PathBuf, String> {
    let resolved = resolve_runtime_path(root, relative, label)?;
    if !resolved.is_file() {
        return Err(format!("PDF Engine {label} 不是普通文件"));
    }
    Ok(resolved)
}

fn resolve_runtime_path(root: &Path, relative: &str, label: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("PDF Engine {label} 路径必须是相对路径"));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return Err(format!("PDF Engine {label} 路径不能跳出 Engine 目录"));
    }
    reject_symlink_components(root, path, label)?;
    let resolved = root
        .join(path)
        .canonicalize()
        .map_err(|error| format!("PDF Engine {label} 文件不存在：{error}"))?;
    if !resolved.starts_with(root) {
        return Err(format!("PDF Engine {label} 文件不在受控目录内"));
    }
    Ok(resolved)
}

fn reject_symlink_components(root: &Path, relative: &Path, label: &str) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        current.push(name);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!("读取 PDF Engine {label} 路径失败：{error}"));
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(format!("PDF Engine {label} 路径不能包含符号链接"));
        }
    }
    Ok(())
}

fn validate_resource(root: &Path, resource: &EngineResource) -> Result<(), String> {
    let (path, expected_sha256, required) = match resource {
        EngineResource::Path(path) => (path.as_str(), None, true),
        EngineResource::Descriptor {
            path,
            sha256,
            size: _,
            required,
        } => (path.as_str(), sha256.as_deref(), *required),
    };
    let resolved = match resolve_runtime_path(root, path, "资源") {
        Ok(value) => value,
        Err(_error) if !required => return Ok(()),
        Err(error) => return Err(error),
    };
    if !resolved.is_file() {
        return if required {
            Err(format!(
                "PDF Engine 资源不是普通文件：{}",
                resolved.display()
            ))
        } else {
            Ok(())
        };
    }
    let (expected_size, expected_sha256) = match resource {
        EngineResource::Path(_) => (None, expected_sha256),
        EngineResource::Descriptor { size, sha256, .. } => (*size, sha256.as_deref()),
    };
    if let Some(expected_size) = expected_size {
        let actual_size = fs::metadata(&resolved)
            .map_err(|error| format!("读取 PDF Engine 资源大小失败：{error}"))?
            .len();
        if actual_size != expected_size {
            return Err(format!(
                "PDF Engine 资源大小校验失败：{}",
                resolved.display()
            ));
        }
    }
    if let Some(expected) = expected_sha256 {
        let expected = normalize_sha256(expected, "PDF Engine 资源")?;
        let actual = hash_file(&resolved)?;
        if actual != expected {
            return Err(format!("PDF Engine 资源校验失败：{}", resolved.display()));
        }
    }
    Ok(())
}

fn resource_declared_size(resource: &EngineResource) -> Option<u64> {
    match resource {
        EngineResource::Path(_) => None,
        EngineResource::Descriptor { size, .. } => *size,
    }
}

fn validate_licenses(root: &Path, licenses: &[EngineLicense]) -> Result<(), String> {
    for license in licenses {
        if license.name.trim().is_empty()
            || license.license.trim().is_empty()
            || license.source.trim().is_empty()
        {
            return Err("PDF Engine manifest 的许可证信息不完整".to_string());
        }
        for file in &license.files {
            let _ = resolve_runtime_file(root, file, "许可证")?;
        }
    }
    Ok(())
}

fn normalize_sha256(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64
        || !normalized
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("{label} sha256 格式无效"));
    }
    Ok(normalized)
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("读取 PDF Engine 文件失败：{}：{error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("读取 PDF Engine 文件失败：{}：{error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn default_required() -> bool {
    true
}

fn current_target() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

fn ensure_supported_target(target: &str) -> Result<(), String> {
    if target == SUPPORTED_ENGINE_TARGET {
        Ok(())
    } else {
        Err(format!(
            "当前平台不受 PDF Engine 支持：{target}；正式版本只支持 Windows x64"
        ))
    }
}

fn status_for_data_dir(data_dir: &Path, preparing: bool) -> PdfEngineStatus {
    let base = |status: &str, error: Option<String>, updating: bool| PdfEngineStatus {
        status: status.to_string(),
        engine_version: BABELDOC_ENGINE_VERSION.to_string(),
        target: current_target(),
        python_version: None,
        babeldoc_version: None,
        distribution_version: None,
        resource_size_bytes: None,
        updating,
        error,
    };
    if let Err(error) = ensure_supported_target(&current_target()) {
        return base("invalid", Some(error), false);
    }
    if preparing {
        let current_root = data_dir.join(ENGINE_PARENT).join(BABELDOC_ENGINE_VERSION);
        return base("preparing", None, current_root.is_dir());
    }
    if !data_dir.is_absolute() {
        return base(
            "invalid",
            Some("应用数据目录必须是绝对路径".to_string()),
            false,
        );
    }
    let root = data_dir.join(ENGINE_PARENT).join(BABELDOC_ENGINE_VERSION);
    if !root.exists() {
        return base("missing", None, false);
    }
    match PdfEngineRuntime::load_lightweight(data_dir) {
        Ok(runtime) => runtime.status(),
        Err(error) => base("invalid", Some(summarize_error(&error)), false),
    }
}

#[tauri::command]
pub fn get_pdf_engine_status(state: State<'_, AppState>) -> PdfEngineStatus {
    status_for_data_dir(
        &state.data_dir,
        state
            .pdf_engine_preparing
            .load(std::sync::atomic::Ordering::Acquire),
    )
}

#[tauri::command]
pub async fn prepare_pdf_engine(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PdfEngineStatus, String> {
    use std::sync::atomic::Ordering;

    let _transition = state
        .pdf_engine_transition
        .lock()
        .map_err(|_| "PDF Engine 切换状态锁已损坏".to_string())?;
    if state
        .pdf_jobs
        .lock()
        .map_err(|_| "PDF 任务状态锁已损坏".to_string())?
        .values()
        .next()
        .is_some()
    {
        return Err("PDF 翻译任务仍在运行，请完成或取消任务后再更新 Engine".to_string());
    }
    let preparing = state.pdf_engine_preparing.clone();
    if preparing
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(status_for_data_dir(&state.data_dir, true));
    }
    drop(_transition);

    let operation_id = Uuid::new_v4().to_string();
    let _ = app.emit(
        "pdf_engine_prepare_started",
        serde_json::json!({"operationId": operation_id}),
    );
    let _ = app.emit(
        "pdf_engine_status_changed",
        status_for_data_dir(&state.data_dir, true),
    );
    let data_dir = state.data_dir.clone();
    let result = {
        #[cfg(debug_assertions)]
        {
            let data_dir_for_task = data_dir.clone();
            tauri::async_runtime::spawn_blocking(move || {
                prepare_development_engine(&data_dir_for_task)
            })
            .await
            .map_err(|error| format!("准备 PDF Engine 的后台任务失败：{error}"))?
        }
        #[cfg(not(debug_assertions))]
        {
            prepare_release_engine(&app, &data_dir, &operation_id).await
        }
    };
    preparing.store(false, Ordering::Release);

    match result {
        Ok(()) => {
            let status = status_for_data_dir(&data_dir, false);
            let _ = app.emit(
                "pdf_engine_prepare_completed",
                serde_json::json!({"operationId": operation_id, "status": status}),
            );
            let _ = app.emit("pdf_engine_status_changed", status.clone());
            Ok(status)
        }
        Err(error) => {
            let message = summarize_error(&error);
            let mut failed_status = status_for_data_dir(&data_dir, false);
            if failed_status.status != "available" {
                failed_status.status = "invalid".to_string();
            }
            failed_status.error = Some(message.clone());
            let _ = app.emit(
                "pdf_engine_prepare_failed",
                serde_json::json!({
                    "operationId": operation_id,
                    "message": message,
                    "status": failed_status.clone(),
                }),
            );
            let _ = app.emit("pdf_engine_status_changed", failed_status);
            Err(error)
        }
    }
}

#[cfg(not(debug_assertions))]
async fn prepare_release_engine(
    app: &AppHandle,
    data_dir: &Path,
    operation_id: &str,
) -> Result<(), String> {
    let target = current_target();
    ensure_supported_target(&target)?;
    let data_root = prepare_data_root(data_dir)?;
    let parent = data_root.join(ENGINE_PARENT);
    fs::create_dir_all(&parent).map_err(|error| format!("创建 PDF Engine 目录失败：{error}"))?;
    emit_engine_progress(
        app,
        operation_id,
        "index",
        None,
        None,
        None,
        Some("读取正式资源索引"),
    );

    let client = reqwest::Client::builder()
        .user_agent("Lilt PDF Engine downloader")
        .build()
        .map_err(|error| format!("创建 PDF Engine 下载器失败：{error}"))?;
    let index = fetch_release_index(&client).await?;
    let asset = index
        .assets
        .get(&target)
        .cloned()
        .ok_or_else(|| format!("正式资源索引没有当前架构：{target}"))?;
    validate_release_asset(&index, &target, &asset)?;

    let archive_path = parent.join(format!(".download-{}.zip", Uuid::new_v4()));
    let staging = parent.join(format!(".staging-{}", Uuid::new_v4()));
    let download_result = async {
        download_release_archive(&client, &asset, &archive_path, app, operation_id).await?;
        emit_engine_progress(
            app,
            operation_id,
            "extract",
            None,
            None,
            None,
            Some("校验并解压 PDF Engine"),
        );
        let archive_path_for_task = archive_path.clone();
        let staging_for_task = staging.clone();
        let index_for_task = index;
        tauri::async_runtime::spawn_blocking(move || {
            install_release_archive(
                &archive_path_for_task,
                &staging_for_task,
                &target,
                &asset,
                &index_for_task,
            )
        })
        .await
        .map_err(|error| format!("安装 PDF Engine 的后台任务失败：{error}"))?
    }
    .await;

    let _ = fs::remove_file(&archive_path);
    if download_result.is_err() {
        let _ = remove_path(&staging);
    }
    download_result
}

#[cfg(not(debug_assertions))]
async fn fetch_release_index(client: &reqwest::Client) -> Result<PdfEngineIndex, String> {
    let response = client
        .get(release_index_url())
        .send()
        .await
        .map_err(|error| format!("读取 PDF Engine 资源索引失败：{error}"))?;
    if response.status() != StatusCode::OK {
        return Err(format!(
            "读取 PDF Engine 资源索引失败：HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RELEASE_INDEX_BYTES as u64)
    {
        return Err("PDF Engine 资源索引超过大小限制".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 PDF Engine 资源索引失败：{error}"))?;
    if bytes.len() > MAX_RELEASE_INDEX_BYTES {
        return Err("PDF Engine 资源索引超过大小限制".to_string());
    }
    let index: PdfEngineIndex = serde_json::from_slice(&bytes)
        .map_err(|error| format!("解析 PDF Engine 资源索引失败：{error}"))?;
    validate_release_index(&index)?;
    Ok(index)
}

#[cfg(not(debug_assertions))]
fn validate_release_index(index: &PdfEngineIndex) -> Result<(), String> {
    if index.schema_version != 1 {
        return Err(format!(
            "PDF Engine 资源索引版本不受支持：{}",
            index.schema_version
        ));
    }
    if index.engine_version != BABELDOC_ENGINE_VERSION {
        return Err(format!(
            "PDF Engine 资源索引版本不匹配：需要 {}，实际 {}",
            BABELDOC_ENGINE_VERSION, index.engine_version
        ));
    }
    if index.distribution_version.trim().is_empty() {
        return Err("PDF Engine 资源索引缺少分发修订号".to_string());
    }
    if index.assets.is_empty() {
        return Err("PDF Engine 资源索引没有可用工件".to_string());
    }
    if !index.assets.contains_key(SUPPORTED_ENGINE_TARGET) {
        return Err("PDF Engine 资源索引缺少 Windows x64 工件".to_string());
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn validate_release_asset(
    index: &PdfEngineIndex,
    target: &str,
    asset: &PdfEngineReleaseAsset,
) -> Result<(), String> {
    if asset.size == 0 || asset.size > MAX_ENGINE_ARCHIVE_BYTES {
        return Err("PDF Engine 压缩包大小超出允许范围".to_string());
    }
    normalize_sha256(&asset.sha256, "PDF Engine 压缩包")?;
    normalize_sha256(&asset.manifest_sha256, "PDF Engine manifest")?;
    validate_release_asset_url(&asset.url, target)?;
    if !index.assets.contains_key(target) {
        return Err(format!("PDF Engine 资源索引缺少架构：{target}"));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn validate_release_asset_url(url: &str, target: &str) -> Result<(), String> {
    let parsed =
        url::Url::parse(url).map_err(|error| format!("PDF Engine 资源 URL 无效：{error}"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("PDF Engine 资源 URL 必须是固定 GitHub HTTPS 地址".to_string());
    }
    let segments = parsed
        .path_segments()
        .ok_or_else(|| "PDF Engine 资源 URL 缺少路径".to_string())?
        .collect::<Vec<_>>();
    if segments.len() != 6
        || segments[0] != RELEASE_REPOSITORY_OWNER
        || segments[1] != RELEASE_REPOSITORY_NAME
        || segments[2] != "releases"
        || segments[3] != "download"
        || segments[4].trim().is_empty()
        || segments[5].trim().is_empty()
        || !segments[5].ends_with(".zip")
        || !segments[5].contains(target)
    {
        return Err("PDF Engine 资源 URL 不属于固定 Release 工件路径".to_string());
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
async fn download_release_archive(
    client: &reqwest::Client,
    asset: &PdfEngineReleaseAsset,
    destination: &Path,
    app: &AppHandle,
    operation_id: &str,
) -> Result<(), String> {
    let expected_sha256 = normalize_sha256(&asset.sha256, "PDF Engine 压缩包")?;
    let response = client
        .get(&asset.url)
        .send()
        .await
        .map_err(|error| format!("下载 PDF Engine 失败：{error}"))?;
    if response.status() != StatusCode::OK {
        return Err(format!("下载 PDF Engine 失败：HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length != asset.size)
    {
        return Err("PDF Engine 下载响应大小与资源索引不一致".to_string());
    }
    let mut file = File::create(destination)
        .map_err(|error| format!("创建 PDF Engine 下载临时文件失败：{error}"))?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("下载 PDF Engine 失败：{error}"))?;
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| "PDF Engine 下载大小溢出".to_string())?;
        if downloaded > asset.size || downloaded > MAX_ENGINE_ARCHIVE_BYTES {
            return Err("PDF Engine 下载超过资源索引声明大小".to_string());
        }
        file.write_all(&chunk)
            .map_err(|error| format!("写入 PDF Engine 下载文件失败：{error}"))?;
        hasher.update(&chunk);
        let fraction = (downloaded as f64 / asset.size as f64).min(1.0);
        emit_engine_progress(
            app,
            operation_id,
            "download",
            Some(downloaded),
            Some(asset.size),
            Some(fraction),
            None,
        );
    }
    file.flush()
        .map_err(|error| format!("刷新 PDF Engine 下载文件失败：{error}"))?;
    if downloaded != asset.size {
        return Err("PDF Engine 下载未达到资源索引声明大小".to_string());
    }
    let actual_sha256 = format!("{:x}", hasher.finalize());
    if actual_sha256 != expected_sha256 {
        return Err("PDF Engine 压缩包摘要校验失败".to_string());
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn install_release_archive(
    archive_path: &Path,
    staging: &Path,
    target: &str,
    asset: &PdfEngineReleaseAsset,
    index: &PdfEngineIndex,
) -> Result<(), String> {
    extract_engine_archive(archive_path, staging)?;
    let manifest_path = staging.join(ENGINE_MANIFEST_NAME);
    let manifest_sha256 = hash_file(&manifest_path)?;
    if manifest_sha256 != normalize_sha256(&asset.manifest_sha256, "PDF Engine manifest")? {
        return Err("PDF Engine manifest 摘要校验失败".to_string());
    }
    let runtime = PdfEngineRuntime::load_from_root(staging.to_path_buf())?;
    if runtime.distribution_version.as_deref() != Some(index.distribution_version.as_str()) {
        return Err("PDF Engine 分发修订号与资源索引不一致".to_string());
    }
    if runtime.status().target != target {
        return Err("PDF Engine 架构校验失败".to_string());
    }
    verify_runtime_startup(&runtime)?;
    let parent = staging
        .parent()
        .ok_or_else(|| "PDF Engine 暂存目录无效".to_string())?;
    let target_root = parent.join(BABELDOC_ENGINE_VERSION);
    let target_root_for_validation = target_root.clone();
    commit_staging_engine_with_validation(staging, &target_root, move || {
        let runtime = PdfEngineRuntime::load_from_root(target_root_for_validation.clone())?;
        verify_runtime_startup(&runtime)
    })
}

#[cfg(not(debug_assertions))]
fn extract_engine_archive(archive_path: &Path, staging: &Path) -> Result<(), String> {
    let file =
        File::open(archive_path).map_err(|error| format!("打开 PDF Engine 压缩包失败：{error}"))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("读取 PDF Engine 压缩包失败：{error}"))?;
    if archive.is_empty() || archive.len() > MAX_ENGINE_ARCHIVE_ENTRIES {
        return Err("PDF Engine 压缩包条目数量无效".to_string());
    }
    fs::create_dir_all(staging)
        .map_err(|error| format!("创建 PDF Engine 暂存目录失败：{error}"))?;
    let mut seen = HashSet::new();
    let mut unpacked_size = 0_u64;
    let mut saw_expected_root = false;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("读取 PDF Engine 压缩包条目失败：{error}"))?;
        let name = entry.name().to_string();
        if name.contains('\\') {
            return Err("PDF Engine 压缩包包含反斜杠路径".to_string());
        }
        let archive_relative = Path::new(&name);
        let components = archive_relative.components().collect::<Vec<_>>();
        if components.is_empty()
            || components.iter().any(|component| {
                matches!(
                    component,
                    Component::CurDir
                        | Component::ParentDir
                        | Component::Prefix(_)
                        | Component::RootDir
                )
            })
        {
            return Err("PDF Engine 压缩包包含不安全路径".to_string());
        }
        let Component::Normal(root_name) = components[0] else {
            return Err("PDF Engine 压缩包根目录无效".to_string());
        };
        if root_name != EXPECTED_ARCHIVE_ROOT {
            return Err("PDF Engine 压缩包根目录版本不匹配".to_string());
        }
        saw_expected_root = true;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("PDF Engine 压缩包不允许符号链接".to_string());
        }
        unpacked_size = unpacked_size
            .checked_add(entry.size())
            .ok_or_else(|| "PDF Engine 解压体积溢出".to_string())?;
        if unpacked_size > MAX_ENGINE_UNPACKED_BYTES {
            return Err("PDF Engine 解压体积超过大小限制".to_string());
        }
        let relative = components
            .iter()
            .skip(1)
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value),
                _ => None,
            })
            .collect::<PathBuf>();
        if relative.as_os_str().is_empty() {
            continue;
        }
        if !seen.insert(relative.clone()) {
            return Err("PDF Engine 压缩包包含重复路径".to_string());
        }
        let destination = staging.join(&relative);
        if !destination.starts_with(staging) {
            return Err("PDF Engine 压缩包路径跳出暂存目录".to_string());
        }
        if entry.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(|error| format!("创建 PDF Engine 解压目录失败：{error}"))?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建 PDF Engine 解压目录失败：{error}"))?;
        }
        if fs::symlink_metadata(&destination).is_ok() {
            return Err("PDF Engine 压缩包包含文件与目录冲突".to_string());
        }
        let mut output = File::create(&destination)
            .map_err(|error| format!("创建 PDF Engine 解压文件失败：{error}"))?;
        let written = std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("解压 PDF Engine 文件失败：{error}"))?;
        if written != entry.size() {
            return Err("PDF Engine 解压文件大小不一致".to_string());
        }
    }
    if !saw_expected_root || !staging.join(ENGINE_MANIFEST_NAME).is_file() {
        return Err("PDF Engine 压缩包缺少 runtime.json".to_string());
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn verify_runtime_startup(runtime: &PdfEngineRuntime) -> Result<(), String> {
    let mut import = runtime.python_command();
    import.args([
        "-s",
        "-c",
        "from babeldoc.format.pdf.high_level import do_translate, get_translation_stage; from babeldoc.format.pdf.document_il.backend.pdf_creater import PDFCreater; from babeldoc.format.pdf.translation_config import TranslationConfig, WatermarkOutputMode; from babeldoc.progress_monitor import ProgressMonitor; import babeldoc; assert babeldoc.__version__ == '0.6.4'",
    ]);
    run_preparation_command(import, "验证 PDF Engine Python 运行时")
}

#[cfg(not(debug_assertions))]
fn emit_engine_progress(
    app: &AppHandle,
    operation_id: &str,
    stage: &str,
    current: Option<u64>,
    total: Option<u64>,
    fraction: Option<f64>,
    message: Option<&str>,
) {
    let _ = app.emit(
        "pdf_engine_prepare_progress",
        serde_json::json!({
            "operationId": operation_id,
            "stage": stage,
            "current": current,
            "total": total,
            "fraction": fraction,
            "message": message,
        }),
    );
}

#[cfg(debug_assertions)]
fn prepare_development_engine(data_dir: &Path) -> Result<(), String> {
    ensure_supported_target(&current_target())?;
    if !data_dir.is_absolute() {
        return Err("PDF Engine 只能写入绝对路径的应用数据目录".to_string());
    }
    let (project_root, babeldoc_source, worker_source) = development_sources()?;
    let data_root = prepare_data_root(data_dir)?;
    let parent = data_root.join(ENGINE_PARENT);
    fs::create_dir_all(&parent).map_err(|error| format!("创建 PDF Engine 目录失败：{error}"))?;
    let staging = parent.join(format!(
        ".{BABELDOC_ENGINE_VERSION}-staging-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&staging)
        .map_err(|error| format!("创建 PDF Engine 临时目录失败：{error}"))?;

    let result = prepare_staging_engine(&staging, &project_root, &babeldoc_source, &worker_source)
        .and_then(|()| {
            let target = parent.join(BABELDOC_ENGINE_VERSION);
            commit_staging_engine(&staging, &target)
        });
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(debug_assertions)]
fn development_sources() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir
        .parent()
        .ok_or_else(|| "无法定位 Lilt 项目目录".to_string())?
        .to_path_buf();
    let babeldoc_source = project_root.join("reference-projects/BabelDOC");
    let worker_source = manifest_dir.join("python_worker/worker.py");
    if !babeldoc_source.is_dir() {
        return Err(format!(
            "开发版 PDF Engine 来源不存在：{}",
            babeldoc_source.display()
        ));
    }
    if !worker_source.is_file() {
        return Err(format!(
            "开发版 PDF Worker 来源不存在：{}",
            worker_source.display()
        ));
    }
    Ok((project_root, babeldoc_source, worker_source))
}

fn prepare_data_root(data_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(data_dir).map_err(|error| format!("创建应用数据目录失败：{error}"))?;
    data_dir
        .canonicalize()
        .map_err(|error| format!("规范化应用数据目录失败：{error}"))
}

#[cfg(debug_assertions)]
fn prepare_staging_engine(
    staging: &Path,
    project_root: &Path,
    babeldoc_source: &Path,
    worker_source: &Path,
) -> Result<(), String> {
    let uv = env::var_os("LILT_PDF_UV")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("uv"));
    let base_python_dir = staging.join(".python-base");
    let mut install_python = Command::new(&uv);
    install_python
        .args(["python", "install", "3.12", "--install-dir"])
        .arg(&base_python_dir)
        .arg("--no-bin")
        .current_dir(project_root);
    run_preparation_command(install_python, "安装 Python 3.12")?;

    let python_dir = staging.join("python");
    fs::create_dir_all(&python_dir)
        .map_err(|error| format!("创建 Engine Python 目录失败：{error}"))?;
    let source_python = find_python_executable(&base_python_dir)?;
    let source_python_root = source_python
        .parent()
        .ok_or_else(|| "无法定位 Python 运行时目录".to_string())?;
    copy_directory_contents(source_python_root, &python_dir)?;
    fs::remove_dir_all(&base_python_dir)
        .map_err(|error| format!("清理 Python 临时目录失败：{error}"))?;

    let final_python = engine_python_path(staging);
    if !final_python.is_file() {
        return Err(format!(
            "Python 运行时未生成预期解释器：{}",
            final_python.display()
        ));
    }
    let site_packages = python_dir.join("Lib/site-packages");
    fs::create_dir_all(&site_packages)
        .map_err(|error| format!("创建 Python 依赖目录失败：{error}"))?;
    let mut install_babeldoc = Command::new(&uv);
    install_babeldoc
        .args(["pip", "install", "--python"])
        .arg(&final_python)
        .args(["--target"])
        .arg(&site_packages)
        .arg("--no-editable")
        .arg(babeldoc_source)
        .current_dir(project_root);
    run_preparation_command(install_babeldoc, "安装 BabelDOC 0.6.4 及依赖")?;

    let worker_path = staging.join(WORKER_RELATIVE_PATH);
    if let Some(parent) = worker_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建 PDF Worker 目录失败：{error}"))?;
    }
    fs::copy(worker_source, &worker_path)
        .map_err(|error| format!("复制 PDF Worker 失败：{error}"))?;
    fs::create_dir_all(staging.join("resources"))
        .map_err(|error| format!("创建 PDF Engine 资源目录失败：{error}"))?;

    let python_version = python_version(&final_python)?;
    let mut verify_import = Command::new(&final_python);
    verify_import
        .args([
            "-c",
            "import babeldoc; assert babeldoc.__version__ == '0.6.4'",
        ])
        .current_dir(staging);
    run_preparation_command(verify_import, "验证 BabelDOC 运行时")?;

    let manifest = serde_json::json!({
        "engine_version": BABELDOC_ENGINE_VERSION,
        "target": current_target(),
        "python": manifest_python_path(),
        "worker": WORKER_RELATIVE_PATH,
        "python_version": python_version,
        "babeldoc_version": BABELDOC_VERSION,
        "pdfmathtranslate_revision": PDFMATH_TRANSLATE_REVISION,
        "resources": [],
    });
    fs::write(
        staging.join(ENGINE_MANIFEST_NAME),
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("生成 PDF Engine manifest 失败：{error}"))?,
    )
    .map_err(|error| format!("写入 PDF Engine manifest 失败：{error}"))?;
    PdfEngineRuntime::load_from_root(staging.to_path_buf())
        .map(|runtime| runtime.log_summary())
        .map_err(|error| format!("准备后的 PDF Engine 校验失败：{error}"))
}

fn commit_staging_engine_with_validation<F>(
    staging: &Path,
    target: &Path,
    validate: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    let parent = target
        .parent()
        .ok_or_else(|| "PDF Engine 目标目录无效".to_string())?;
    if staging.parent() != Some(parent) {
        return Err("PDF Engine 暂存目录与目标目录不在同一父目录".to_string());
    }
    let staging_metadata = fs::symlink_metadata(staging)
        .map_err(|error| format!("读取 PDF Engine 暂存目录失败：{error}"))?;
    if !staging_metadata.file_type().is_dir() {
        return Err("PDF Engine 暂存路径不是目录".to_string());
    }
    let backup = parent.join(format!(
        ".{BABELDOC_ENGINE_VERSION}-backup-{}",
        Uuid::new_v4()
    ));
    let target_metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("读取现有 PDF Engine 失败：{error}")),
    };
    if target_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("现有 PDF Engine 目录不能是符号链接".to_string());
    }
    let had_target = target_metadata.is_some();
    if had_target {
        fs::rename(target, &backup).map_err(|error| format!("暂存旧 PDF Engine 失败：{error}"))?;
    }

    if let Err(error) = fs::rename(staging, target) {
        let restore_error = if had_target {
            fs::rename(&backup, target).err()
        } else {
            None
        };
        return Err(match restore_error {
            Some(restore_error) => {
                format!("切换 PDF Engine 版本失败：{error}；恢复旧版本失败：{restore_error}")
            }
            None => format!("切换 PDF Engine 版本失败：{error}"),
        });
    }

    if let Err(error) = validate() {
        let remove_new_error = remove_path(target).err();
        let restore_error = if had_target {
            fs::rename(&backup, target).err()
        } else {
            None
        };
        return Err(match (remove_new_error, restore_error) {
            (Some(remove_error), Some(restore_error)) => format!(
                "新 PDF Engine 校验失败：{error}；清理新版本失败：{remove_error}；恢复旧版本失败：{restore_error}"
            ),
            (Some(remove_error), None) => {
                format!("新 PDF Engine 校验失败：{error}；清理新版本失败：{remove_error}")
            }
            (None, Some(restore_error)) => {
                format!("新 PDF Engine 校验失败：{error}；恢复旧版本失败：{restore_error}")
            }
            (None, None) => format!("新 PDF Engine 校验失败：{error}"),
        });
    }

    if had_target {
        if let Err(error) = remove_path(&backup) {
            crate::diagnostics::warn(format!(
                "pdf.engine.old_version_cleanup_failed error={error}"
            ));
        }
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn commit_staging_engine(staging: &Path, target: &Path) -> Result<(), String> {
    commit_staging_engine_with_validation(staging, target, || Ok(()))
}

fn remove_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取待清理路径失败：{error}")),
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path).map_err(|error| format!("清理目录失败：{error}"))
    } else {
        fs::remove_file(path).map_err(|error| format!("清理文件失败：{error}"))
    }
}

#[cfg(debug_assertions)]
fn engine_python_path(root: &Path) -> PathBuf {
    root.join("python").join(if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    })
}

#[cfg(debug_assertions)]
fn manifest_python_path() -> &'static str {
    if cfg!(windows) {
        "python/python.exe"
    } else {
        "python/python"
    }
}

#[cfg(debug_assertions)]
fn find_python_executable(root: &Path) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut visited = HashSet::new();
    while let Some(directory) = pending.pop() {
        let directory = fs::canonicalize(&directory)
            .map_err(|error| format!("读取 Python 运行时目录失败：{error}"))?;
        if !visited.insert(directory.clone()) {
            continue;
        }
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("读取 Python 运行时目录失败：{error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("读取 Python 运行时条目失败：{error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("读取 Python 运行时条目类型失败：{error}"))?;
            let resolved_path = if file_type.is_symlink() {
                fs::canonicalize(&path)
                    .map_err(|error| format!("解析 Python 运行时链接失败：{error}"))?
            } else {
                path.clone()
            };
            let metadata = fs::metadata(&resolved_path)
                .map_err(|error| format!("读取 Python 运行时条目失败：{error}"))?;
            if metadata.is_dir() {
                pending.push(resolved_path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let matches = if cfg!(windows) {
                name.eq_ignore_ascii_case("python.exe")
            } else {
                name == "python" || name == "python3" || name.starts_with("python3.")
            };
            if matches {
                let parent_is_scripts = path
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name.eq_ignore_ascii_case("Scripts"));
                if !parent_is_scripts {
                    candidates.push(resolved_path);
                }
            }
        }
    }
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| "uv 未生成可复制的 Python 解释器".to_string())
}

#[cfg(debug_assertions)]
fn copy_directory_contents(source: &Path, destination: &Path) -> Result<(), String> {
    let mut active_sources = HashSet::new();
    copy_directory_contents_inner(source, destination, &mut active_sources)
}

#[cfg(debug_assertions)]
fn copy_directory_contents_inner(
    source: &Path,
    destination: &Path,
    active_sources: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    let source =
        fs::canonicalize(source).map_err(|error| format!("解析 Python 运行时目录失败：{error}"))?;
    if !active_sources.insert(source.clone()) {
        return Err("Python 运行时包含循环链接".to_string());
    }

    let result = (|| {
        fs::create_dir_all(destination)
            .map_err(|error| format!("创建 Python 运行时目录失败：{error}"))?;
        let entries = fs::read_dir(&source)
            .map_err(|error| format!("读取 Python 运行时文件失败：{error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("读取 Python 运行时文件失败：{error}"))?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry
                .file_type()
                .map_err(|error| format!("读取 Python 运行时文件类型失败：{error}"))?;
            let resolved_path = if file_type.is_symlink() {
                fs::canonicalize(&source_path)
                    .map_err(|error| format!("解析 Python 运行时链接失败：{error}"))?
            } else {
                source_path
            };
            let metadata = fs::metadata(&resolved_path)
                .map_err(|error| format!("读取 Python 运行时文件失败：{error}"))?;
            if metadata.is_dir() {
                copy_directory_contents_inner(&resolved_path, &destination_path, active_sources)?;
            } else if metadata.is_file() {
                fs::copy(&resolved_path, &destination_path)
                    .map_err(|error| format!("复制 Python 运行时文件失败：{error}"))?;
            }
        }
        Ok(())
    })();
    active_sources.remove(&source);
    result
}

#[cfg(debug_assertions)]
fn python_version(python: &Path) -> Result<String, String> {
    let output = run_preparation_output(Command::new(python).arg("--version"), "读取 Python 版本")?;
    let combined = command_output_text(&output);
    let version = combined
        .split_whitespace()
        .find(|part| {
            part.chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        })
        .ok_or_else(|| "Python 版本输出无效".to_string())?;
    Ok(version.to_string())
}

fn run_preparation_command(mut command: Command, label: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("{label}失败，无法启动准备工具：{error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{label}失败：{}",
        summarize_command_output(&output)
    ))
}

#[cfg(debug_assertions)]
fn run_preparation_output(command: &mut Command, label: &str) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("{label}失败，无法启动准备工具：{error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{label}失败：{}",
            summarize_command_output(&output)
        ))
    }
}

fn command_output_text(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{stdout}\n{stderr}")
}

fn summarize_command_output(output: &Output) -> String {
    let text = command_output_text(output);
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("准备工具未提供错误详情");
    summarize_error(line)
}

fn summarize_error(error: &str) -> String {
    let error = error.trim();
    let mut summary = error.chars().take(500).collect::<String>();
    if error.chars().count() > 500 {
        summary.push('…');
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::{
        BABELDOC_ENGINE_VERSION, PdfEngineRuntime, build_worker_command, copy_directory_contents,
        current_target, ensure_supported_target, find_python_executable, resolve_runtime_file,
        status_for_data_dir,
    };
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("lilt-pdf-engine-{suffix}"));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &PathBuf {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_runtime(root: &std::path::Path, target: &str, python: &str, worker: &str) {
        fs::create_dir_all(root.join("python")).expect("create python dir");
        fs::create_dir_all(root.join("pdf-worker")).expect("create worker dir");
        fs::write(root.join("python/python.exe"), b"python").expect("write python");
        fs::write(root.join("pdf-worker/worker.py"), b"worker").expect("write worker");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).expect("create resources dir");
        fs::write(resources.join("layout.onnx"), b"model").expect("write resource");
        let resource_sha256 = format!("{:x}", Sha256::digest(b"model"));
        fs::write(
            root.join("runtime.json"),
            serde_json::to_vec(&serde_json::json!({
                "engine_version": BABELDOC_ENGINE_VERSION,
                "target": target,
                "python": python,
                "worker": worker,
                "python_version": "3.12.0",
                "babeldoc_version": "0.6.4",
                "pdfmathtranslate_revision": "test-revision",
                "resources": [{"path": "resources/layout.onnx", "sha256": resource_sha256}]
            }))
            .expect("encode manifest"),
        )
        .expect("write manifest");
    }

    #[test]
    fn rejects_an_unsupported_engine_target() {
        let error = ensure_supported_target("windows-other").expect_err("target should fail");
        assert!(error.contains("只支持 Windows x64"));
    }

    #[test]
    fn missing_app_data_engine_is_reported_without_scanning_or_creating_files() {
        let temp = TempDir::new();
        let data_dir = temp.path().join("app-data");
        let status = status_for_data_dir(&data_dir, false);
        assert_eq!(status.status, "missing");
        assert!(!data_dir.exists());
    }

    #[test]
    fn loads_a_valid_external_runtime() {
        let temp = TempDir::new();
        let root = temp.path().join("engine");
        fs::create_dir_all(&root).expect("create engine");
        write_runtime(
            &root,
            &current_target(),
            "python/python.exe",
            "pdf-worker/worker.py",
        );

        let runtime = PdfEngineRuntime::load_from_root(root).expect("runtime should load");
        assert!(runtime.python.ends_with("python.exe"));
        assert!(runtime.worker.ends_with("worker.py"));
    }

    #[test]
    fn worker_command_uses_only_the_versioned_app_data_engine() {
        let temp = TempDir::new();
        let data_dir = temp.path().join("app-data");
        let root = data_dir.join("engines/pdf").join(BABELDOC_ENGINE_VERSION);
        fs::create_dir_all(&root).expect("create engine");
        write_runtime(
            &root,
            &current_target(),
            "python/python.exe",
            "pdf-worker/worker.py",
        );

        let command = build_worker_command(&data_dir).expect("engine should load");
        let canonical_root = root
            .canonicalize()
            .expect("engine root should canonicalize");
        assert_eq!(
            command.get_program(),
            canonical_root.join("python/python.exe").as_os_str()
        );
        assert_eq!(command.get_current_dir(), Some(canonical_root.as_path()));
        assert_eq!(command.get_args().next(), Some(std::ffi::OsStr::new("-s")));
        assert_eq!(
            command.get_args().nth(1),
            Some(canonical_root.join("pdf-worker/worker.py").as_os_str())
        );
    }

    #[test]
    fn rejects_a_runtime_for_another_architecture() {
        let temp = TempDir::new();
        let root = temp.path().join("engine");
        fs::create_dir_all(&root).expect("create engine");
        write_runtime(
            &root,
            "windows-other",
            "python/python.exe",
            "pdf-worker/worker.py",
        );

        let error = PdfEngineRuntime::load_from_root(root).expect_err("target should fail");
        assert!(error.contains("架构不匹配"));
    }

    #[test]
    fn rejects_a_runtime_file_that_escapes_the_engine_root() {
        let temp = TempDir::new();
        let root = temp.path().join("engine");
        fs::create_dir_all(&root).expect("create engine");
        fs::write(temp.path().join("outside.py"), b"worker").expect("write outside file");

        let error = resolve_runtime_file(&root, "../outside.py", "Worker")
            .expect_err("path traversal should fail");
        assert!(error.contains("不能跳出"));
    }

    #[test]
    fn rejects_a_resource_with_the_wrong_digest() {
        let temp = TempDir::new();
        let root = temp.path().join("engine");
        fs::create_dir_all(&root).expect("create engine");
        write_runtime(
            &root,
            &current_target(),
            "python/python.exe",
            "pdf-worker/worker.py",
        );
        fs::write(root.join("resources/layout.onnx"), b"changed").expect("change resource");

        let error = PdfEngineRuntime::load_from_root(root).expect_err("digest should fail");
        assert!(error.contains("资源校验失败"));
    }

    #[test]
    fn status_does_not_hash_resources_during_a_lightweight_check() {
        let temp = TempDir::new();
        let data_dir = temp.path().join("app-data");
        let root = data_dir.join("engines/pdf").join(BABELDOC_ENGINE_VERSION);
        fs::create_dir_all(&root).expect("create engine");
        write_runtime(
            &root,
            &current_target(),
            "python/python.exe",
            "pdf-worker/worker.py",
        );
        fs::write(root.join("resources/layout.onnx"), b"changed").expect("change resource");

        let status = status_for_data_dir(&data_dir, false);
        assert_eq!(status.status, "available");
        assert_eq!(status.python_version.as_deref(), Some("3.12.0"));
        assert!(PdfEngineRuntime::load(&data_dir).is_err());
    }

    #[test]
    fn invalid_manifest_is_reported_as_a_frontend_readable_status() {
        let temp = TempDir::new();
        let data_dir = temp.path().join("app-data");
        let root = data_dir.join("engines/pdf").join(BABELDOC_ENGINE_VERSION);
        fs::create_dir_all(&root).expect("create engine");
        fs::write(root.join("runtime.json"), b"not-json").expect("write invalid manifest");

        let status = status_for_data_dir(&data_dir, false);
        assert_eq!(status.status, "invalid");
        assert!(
            status
                .error
                .as_deref()
                .is_some_and(|error| error.contains("解析"))
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn failed_atomic_switch_restores_the_existing_runtime() {
        let temp = TempDir::new();
        let parent = temp.path().join("engines/pdf");
        let target = parent.join(BABELDOC_ENGINE_VERSION);
        fs::create_dir_all(&target).expect("create current engine");
        fs::write(target.join("marker"), b"current").expect("write current marker");

        let missing_staging = parent.join("missing-staging");
        assert!(super::commit_staging_engine(&missing_staging, &target).is_err());
        assert_eq!(
            fs::read(target.join("marker")).expect("read restored marker"),
            b"current"
        );
    }

    #[cfg(all(debug_assertions, windows))]
    #[test]
    fn debug_python_copy_materializes_a_linked_uv_install() {
        use std::os::windows::fs::symlink_dir;

        let temp = TempDir::new();
        let uv_root = temp.path().join("uv-python");
        let managed_root = uv_root.join("cpython-3.12-windows-x86_64-none");
        let install_root = temp.path().join("python-base");
        let linked_install = install_root.join("cpython-3.12-windows-x86_64-none");
        fs::create_dir_all(&managed_root).expect("create managed Python");
        fs::write(managed_root.join("python.exe"), b"python").expect("write Python");
        fs::create_dir_all(&install_root).expect("create install directory");
        if symlink_dir(&managed_root, &linked_install).is_err() {
            return;
        }

        let executable = find_python_executable(&install_root).expect("find linked Python");
        assert!(executable.ends_with("python.exe"));

        let destination = temp.path().join("materialized-python");
        copy_directory_contents(
            executable.parent().expect("Python parent directory"),
            &destination,
        )
        .expect("copy linked Python");
        assert_eq!(
            fs::read(destination.join("python.exe")).expect("read materialized Python"),
            b"python"
        );
        assert!(
            !fs::symlink_metadata(&destination)
                .expect("read destination metadata")
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn rejects_a_missing_manifest() {
        let temp = TempDir::new();
        let error = PdfEngineRuntime::load_from_root(temp.path().join("missing"))
            .expect_err("missing runtime should fail");
        assert!(error.contains("目录不存在"));
    }
}
