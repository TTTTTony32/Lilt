use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(crate) const BABELDOC_ENGINE_VERSION: &str = "babeldoc-0.6.4";
const ENGINE_MANIFEST_NAME: &str = "runtime.json";

#[derive(Debug, Deserialize)]
struct EngineManifest {
    engine_version: String,
    target: String,
    python: String,
    worker: String,
    python_version: String,
    babeldoc_version: String,
    pdfmathtranslate_revision: String,
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
    python: PathBuf,
    worker: PathBuf,
    python_version: String,
    resource_count: usize,
}

impl PdfEngineRuntime {
    fn load(data_dir: &Path) -> Result<Self, String> {
        let root = data_dir
            .join("engines")
            .join("pdf")
            .join(BABELDOC_ENGINE_VERSION);
        Self::load_from_root(root)
    }

    fn load_from_root(root: PathBuf) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("PDF Engine 目录不存在：{error}"))?;
        if !root.is_dir() {
            return Err("PDF Engine 目录不是文件夹".to_string());
        }

        let manifest_path = root.join(ENGINE_MANIFEST_NAME);
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("读取 PDF Engine manifest 失败：{error}"))?;
        let manifest: EngineManifest = serde_json::from_str(&manifest_text)
            .map_err(|error| format!("解析 PDF Engine manifest 失败：{error}"))?;
        if manifest.engine_version != BABELDOC_ENGINE_VERSION {
            return Err(format!(
                "PDF Engine 版本不匹配：需要 {}，实际 {}",
                BABELDOC_ENGINE_VERSION, manifest.engine_version
            ));
        }
        if manifest.babeldoc_version != "0.6.4" {
            return Err(format!(
                "BabelDOC 版本不匹配：需要 0.6.4，实际 {}",
                manifest.babeldoc_version
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

        let python = resolve_runtime_file(&root, &manifest.python, "Python")?;
        let worker = resolve_runtime_file(&root, &manifest.worker, "Worker")?;
        for resource in &manifest.resources {
            validate_resource(&root, resource)?;
        }
        Ok(Self {
            python,
            worker,
            python_version: manifest.python_version,
            resource_count: manifest.resources.len(),
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.python);
        command.arg(&self.worker);
        command
    }

    fn log_summary(&self) {
        crate::diagnostics::info(format!(
            "pdf.engine.ready python_version={} resource_count={}",
            self.python_version, self.resource_count
        ));
    }
}

pub(crate) fn build_worker_command(data_dir: &Path) -> Result<Command, String> {
    let worker_override = cfg!(debug_assertions)
        .then(|| env::var_os("LILT_PDF_WORKER_SCRIPT"))
        .flatten();
    let worker_script = worker_override
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("python_worker")
                .join("worker.py")
        });

    let python_override = cfg!(debug_assertions)
        .then(|| env::var_os("LILT_PDF_PYTHON"))
        .flatten();
    let command = if let Some(python) = python_override {
        ensure_worker_script(&worker_script)?;
        let mut command = Command::new(python);
        command.arg(worker_script);
        command
    } else if worker_override.is_some() && cfg!(debug_assertions) {
        ensure_worker_script(&worker_script)?;
        let uv = env::var_os("LILT_PDF_UV").unwrap_or_else(|| "uv".into());
        let mut command = Command::new(uv);
        command.args(["tool", "run", "--from", "BabelDOC==0.6.4", "python"]);
        command.arg(worker_script);
        command
    } else {
        let engine_root_override = cfg!(debug_assertions)
            .then(|| env::var_os("LILT_PDF_ENGINE_ROOT"))
            .flatten()
            .map(PathBuf::from);
        let engine_root = engine_root_override.clone().unwrap_or_else(|| {
            data_dir
                .join("engines")
                .join("pdf")
                .join(BABELDOC_ENGINE_VERSION)
        });
        if engine_root_override.is_some() || engine_root.exists() {
            let runtime = if engine_root_override.is_some() {
                PdfEngineRuntime::load_from_root(engine_root)?
            } else {
                PdfEngineRuntime::load(data_dir)?
            };
            runtime.log_summary();
            runtime.command()
        } else if cfg!(debug_assertions) {
            ensure_worker_script(&worker_script)?;
            let uv = env::var_os("LILT_PDF_UV").unwrap_or_else(|| "uv".into());
            let mut command = Command::new(uv);
            command.args(["tool", "run", "--from", "BabelDOC==0.6.4", "python"]);
            command.arg(worker_script);
            command
        } else {
            return Err(format!(
                "未找到 PDF Engine，请先下载 {} 运行环境",
                BABELDOC_ENGINE_VERSION
            ));
        }
    };

    Ok(configure_command(command))
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
    if cfg!(debug_assertions)
        && let Some(python_path) = env::var_os("LILT_PDF_PYTHONPATH")
    {
        command.env("PYTHONPATH", python_path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn ensure_worker_script(path: &Path) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("PDF Worker 脚本不存在：{}", path.display()))
    }
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

fn validate_resource(root: &Path, resource: &EngineResource) -> Result<(), String> {
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
    Ok(())
}

const fn default_required() -> bool {
    true
}

fn current_target() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::{BABELDOC_ENGINE_VERSION, PdfEngineRuntime, current_target, resolve_runtime_file};
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
        let manifest = serde_json::json!({
            "engine_version": BABELDOC_ENGINE_VERSION,
            "target": target,
            "python": python,
            "worker": worker,
            "resources": ["resources/layout.onnx"],
        });
        fs::create_dir_all(root.join("resources")).expect("create resources dir");
        fs::write(root.join("resources/layout.onnx"), b"model").expect("write resource");
        let resource_sha256 = format!("{:x}", Sha256::digest(b"model"));
        fs::write(
            root.join("runtime.json"),
            serde_json::to_vec(&serde_json::json!({
                "engine_version": manifest["engine_version"],
                "target": manifest["target"],
                "python": manifest["python"],
                "worker": manifest["worker"],
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
    fn rejects_a_missing_manifest() {
        let temp = TempDir::new();
        let error = PdfEngineRuntime::load_from_root(temp.path().join("missing"))
            .expect_err("missing runtime should fail");
        assert!(error.contains("目录不存在"));
    }
}
