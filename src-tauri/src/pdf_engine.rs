use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
#[cfg(debug_assertions)]
use std::process::Output;
#[cfg(debug_assertions)]
use tauri::Emitter;
use tauri::{AppHandle, State};
#[cfg(debug_assertions)]
use uuid::Uuid;

use crate::AppState;

pub(crate) const BABELDOC_ENGINE_VERSION: &str = "babeldoc-0.6.4";
const BABELDOC_VERSION: &str = "0.6.4";
#[cfg(debug_assertions)]
const PDFMATH_TRANSLATE_REVISION: &str = "f8dffcf4c3a33b254391d43514439b975ce8d966";
const ENGINE_MANIFEST_NAME: &str = "runtime.json";
const ENGINE_PARENT: &str = "engines/pdf";
#[cfg(debug_assertions)]
const WORKER_RELATIVE_PATH: &str = "pdf-worker/worker.py";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfEngineStatus {
    pub status: String,
    pub engine_version: String,
    pub target: String,
    pub python_version: Option<String>,
    pub babeldoc_version: Option<String>,
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
    resources: Vec<EngineResource>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EngineResource {
    Path(String),
    Descriptor {
        path: String,
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default = "default_required")]
        required: bool,
    },
}

#[derive(Debug, Clone)]
struct PdfEngineRuntime {
    root: PathBuf,
    python: PathBuf,
    worker: PathBuf,
    python_version: String,
    babeldoc_version: String,
    resource_count: usize,
}

impl PdfEngineRuntime {
    fn load(data_dir: &Path) -> Result<Self, String> {
        let (data_root, root) = canonical_engine_root(data_dir)?;
        Self::load_canonical_root(root, Some(&data_root), true)
    }

    fn load_lightweight(data_dir: &Path) -> Result<Self, String> {
        let (data_root, root) = canonical_engine_root(data_dir)?;
        Self::load_canonical_root(root, Some(&data_root), false)
    }

    #[cfg(any(test, debug_assertions))]
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
        for resource in &manifest.resources {
            validate_resource(&root, resource, verify_resource_digests)?;
        }
        Ok(Self {
            root,
            python,
            worker,
            python_version: manifest.python_version,
            babeldoc_version: manifest.babeldoc_version,
            resource_count: manifest.resources.len(),
        })
    }

    fn command(&self) -> Command {
        let python_home = self
            .python
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        let mut command = Command::new(&self.python);
        command
            .arg("-s")
            .arg(&self.worker)
            .current_dir(&self.root)
            .env("PYTHONHOME", python_home)
            .env("PYTHONNOUSERSITE", "1")
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONUSERBASE")
            .env_remove("VIRTUAL_ENV");
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
    let expected_target = current_target();
    if manifest.target != expected_target {
        return Err(format!(
            "PDF Engine 架构不匹配：需要 {}，实际 {}",
            expected_target, manifest.target
        ));
    }
    Ok(())
}

pub(crate) fn build_worker_command(data_dir: &Path) -> Result<Command, String> {
    let runtime = PdfEngineRuntime::load(data_dir)?;
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
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("PDF Engine {label} 路径不能跳出 Engine 目录"));
    }
    let resolved = root
        .join(path)
        .canonicalize()
        .map_err(|error| format!("PDF Engine {label} 文件不存在：{error}"))?;
    if !resolved.starts_with(root) {
        return Err(format!("PDF Engine {label} 文件不在受控目录内"));
    }
    Ok(resolved)
}

fn validate_resource(
    root: &Path,
    resource: &EngineResource,
    verify_digest: bool,
) -> Result<(), String> {
    let (path, expected_sha256, required) = match resource {
        EngineResource::Path(path) => (path.as_str(), None, true),
        EngineResource::Descriptor {
            path,
            sha256,
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
    if verify_digest {
        if let Some(expected) = expected_sha256 {
            let expected = expected.trim().to_ascii_lowercase();
            if expected.len() != 64
                || !expected
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            {
                return Err("PDF Engine 资源 sha256 格式无效".to_string());
            }
            let bytes = fs::read(&resolved).map_err(|error| {
                format!("读取 PDF Engine 资源失败：{}：{error}", resolved.display())
            })?;
            let actual = format!("{:x}", Sha256::digest(bytes));
            if actual != expected {
                return Err(format!("PDF Engine 资源校验失败：{}", resolved.display()));
            }
        }
    }
    Ok(())
}

fn default_required() -> bool {
    true
}

fn current_target() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

fn status_for_data_dir(data_dir: &Path, preparing: bool) -> PdfEngineStatus {
    let base = |status: &str, error: Option<String>| PdfEngineStatus {
        status: status.to_string(),
        engine_version: BABELDOC_ENGINE_VERSION.to_string(),
        target: current_target(),
        python_version: None,
        babeldoc_version: None,
        error,
    };
    if preparing {
        return base("preparing", None);
    }
    if !data_dir.is_absolute() {
        return base("invalid", Some("应用数据目录必须是绝对路径".to_string()));
    }
    let root = data_dir.join(ENGINE_PARENT).join(BABELDOC_ENGINE_VERSION);
    if !root.exists() {
        return base("missing", None);
    }
    match PdfEngineRuntime::load_lightweight(data_dir) {
        Ok(runtime) => runtime.status(),
        Err(error) => base("invalid", Some(summarize_error(&error))),
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
    #[cfg(not(debug_assertions))]
    {
        let _ = (app, state);
        return Err("正式版 PDF Engine 下载尚未实现，请等待运行时下载功能完成后再试".to_string());
    }

    #[cfg(debug_assertions)]
    {
        use std::sync::atomic::Ordering;

        let preparing = state.pdf_engine_preparing.clone();
        if preparing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(status_for_data_dir(&state.data_dir, true));
        }
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
        let data_dir_for_task = data_dir.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            prepare_development_engine(&data_dir_for_task)
        })
        .await;
        preparing.store(false, Ordering::Release);
        let result = result.map_err(|error| format!("准备 PDF Engine 的后台任务失败：{error}"));
        let result = match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => {
                let status = status_for_data_dir(&data_dir, false);
                let _ = app.emit(
                    "pdf_engine_prepare_completed",
                    serde_json::json!({"operationId": operation_id, "status": status}),
                );
                let _ = app.emit(
                    "pdf_engine_status_changed",
                    status_for_data_dir(&data_dir, false),
                );
                Ok(status)
            }
            Err(error) => {
                let _ = app.emit(
                    "pdf_engine_prepare_failed",
                    serde_json::json!({"operationId": operation_id, "message": error}),
                );
                let _ = app.emit(
                    "pdf_engine_status_changed",
                    status_for_data_dir(&data_dir, false),
                );
                Err(error)
            }
        }
    }
}

#[cfg(debug_assertions)]
fn prepare_development_engine(data_dir: &Path) -> Result<(), String> {
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

#[cfg(debug_assertions)]
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

#[cfg(debug_assertions)]
fn commit_staging_engine(staging: &Path, target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "PDF Engine 目标目录无效".to_string())?;
    let backup = parent.join(format!(
        ".{BABELDOC_ENGINE_VERSION}-backup-{}",
        Uuid::new_v4()
    ));
    let had_target = target.exists();
    if had_target {
        fs::rename(target, &backup).map_err(|error| format!("暂存旧 PDF Engine 失败：{error}"))?;
    }
    match fs::rename(staging, target) {
        Ok(()) => {
            if had_target {
                if let Err(error) = fs::remove_dir_all(&backup) {
                    crate::diagnostics::warn(format!(
                        "pdf.engine.old_version_cleanup_failed error={error}"
                    ));
                }
            }
            Ok(())
        }
        Err(error) => {
            if had_target && !target.exists() {
                let _ = fs::rename(&backup, target);
            }
            Err(format!("切换 PDF Engine 版本失败：{error}"))
        }
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
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("读取 Python 运行时目录失败：{error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("读取 Python 运行时条目失败：{error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("读取 Python 运行时条目类型失败：{error}"))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "Python 运行时包含不受支持的符号链接：{}",
                    path.display()
                ));
            }
            if file_type.is_dir() {
                pending.push(path);
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
                    candidates.push(path);
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
    fs::create_dir_all(destination)
        .map_err(|error| format!("创建 Python 运行时目录失败：{error}"))?;
    let entries =
        fs::read_dir(source).map_err(|error| format!("读取 Python 运行时文件失败：{error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取 Python 运行时文件失败：{error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取 Python 运行时文件类型失败：{error}"))?;
        if file_type.is_symlink() {
            return Err(format!(
                "Python 运行时包含不受支持的符号链接：{}",
                source_path.display()
            ));
        }
        if file_type.is_dir() {
            copy_directory_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path)
                .map_err(|error| format!("复制 Python 运行时文件失败：{error}"))?;
        }
    }
    Ok(())
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

#[cfg(debug_assertions)]
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

#[cfg(debug_assertions)]
fn command_output_text(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{stdout}\n{stderr}")
}

#[cfg(debug_assertions)]
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
        BABELDOC_ENGINE_VERSION, PdfEngineRuntime, build_worker_command, current_target,
        resolve_runtime_file, status_for_data_dir,
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
        assert_eq!(
            command.get_program(),
            root.join("python/python.exe").as_os_str()
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
        assert_eq!(command.get_args().next(), Some(std::ffi::OsStr::new("-s")));
        assert_eq!(
            command.get_args().nth(1),
            Some(root.join("pdf-worker/worker.py").as_os_str())
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

    #[test]
    fn rejects_a_missing_manifest() {
        let temp = TempDir::new();
        let error = PdfEngineRuntime::load_from_root(temp.path().join("missing"))
            .expect_err("missing runtime should fail");
        assert!(error.contains("目录不存在"));
    }
}
