use std::{fs, path::Path};
use tauri::ipc::Response;

const EMPTY_PATH_ERROR: &str = "PDF 文件路径不能为空";
const INVALID_EXTENSION_ERROR: &str = "PDF 文件扩展名必须为 .pdf";
const NOT_FILE_ERROR: &str = "PDF 路径必须指向普通文件";
const READ_ERROR: &str = "读取 PDF 文件失败";

fn validate_pdf_path(file_path: &str) -> Result<&Path, String> {
    let file_path = file_path.trim();
    if file_path.is_empty() {
        return Err(EMPTY_PATH_ERROR.to_string());
    }

    let path = Path::new(file_path);
    let is_pdf = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Err(INVALID_EXTENSION_ERROR.to_string());
    }
    if !path.is_file() {
        return Err(NOT_FILE_ERROR.to_string());
    }
    Ok(path)
}

fn read_file_bytes(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|_| READ_ERROR.to_string())
}

fn read_pdf_file(file_path: &str) -> Result<Vec<u8>, String> {
    let path = validate_pdf_path(file_path)?;
    read_file_bytes(path)
}

#[tauri::command]
pub fn read_pdf_bytes(file_path: String) -> Result<Response, String> {
    let bytes = read_pdf_file(&file_path)?;
    Ok(Response::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::{
        EMPTY_PATH_ERROR, INVALID_EXTENSION_ERROR, NOT_FILE_ERROR, READ_ERROR, read_file_bytes,
        read_pdf_file, validate_pdf_path,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("lilt-pdf-test-{suffix}"));
            fs::create_dir_all(&path).expect("test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn path_string(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn rejects_an_empty_pdf_path() {
        let error = validate_pdf_path(" \t\n").unwrap_err();

        assert_eq!(error, EMPTY_PATH_ERROR);
    }

    #[test]
    fn rejects_paths_without_a_pdf_extension() {
        let directory = TestDirectory::new();
        let path = directory.path().join("document.txt");
        fs::write(&path, b"not a pdf").expect("test file should be written");

        let error = read_pdf_file(&path_string(&path)).unwrap_err();

        assert_eq!(error, INVALID_EXTENSION_ERROR);
        assert!(!error.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn rejects_directories_even_when_the_extension_matches() {
        let directory = TestDirectory::new();
        let path = directory.path().join("document.PDF");
        fs::create_dir(&path).expect("test directory should be created");

        let error = validate_pdf_path(&path_string(&path)).unwrap_err();

        assert_eq!(error, NOT_FILE_ERROR);
        assert!(!error.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn reads_pdf_bytes_with_a_case_insensitive_extension() {
        let directory = TestDirectory::new();
        let path = directory.path().join("document.PdF");
        let expected = vec![0, 1, 2, 127, 128, 255];
        fs::write(&path, &expected).expect("test file should be written");

        let actual = read_pdf_file(&path_string(&path)).expect("PDF bytes should be read");

        assert_eq!(actual, expected);
    }

    #[test]
    fn hides_filesystem_details_when_reading_fails() {
        let directory = TestDirectory::new();
        let path = directory.path().join("missing.pdf");

        let error = read_file_bytes(&path).unwrap_err();

        assert_eq!(error, READ_ERROR);
        assert!(!error.contains(path.to_string_lossy().as_ref()));
    }
}
