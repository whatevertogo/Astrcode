//! Session key 规范化与工作目录校验。

use std::path::{Path, PathBuf};

use astrcode_core::AstrError;

/// 规范化会话 ID，去除首尾空白并剥离最外层 `session-` 前缀。
pub fn normalize_session_id(session_id: &str) -> String {
    let trimmed = session_id.trim();
    trimmed
        .strip_prefix("session-")
        .unwrap_or(trimmed)
        .to_string()
}

/// 规范化工作目录路径，要求路径存在且必须是目录。
pub fn normalize_working_dir(working_dir: PathBuf) -> Result<PathBuf, AstrError> {
    let path = if working_dir.is_absolute() {
        working_dir
    } else {
        std::env::current_dir()
            .map_err(|error| AstrError::io("failed to get current directory", error))?
            .join(working_dir)
    };

    let metadata = std::fs::metadata(&path).map_err(|error| {
        AstrError::Validation(format!(
            "workingDir '{}' is invalid: {}",
            path.display(),
            error
        ))
    })?;
    if !metadata.is_dir() {
        return Err(AstrError::Validation(format!(
            "workingDir '{}' is not a directory",
            path.display()
        )));
    }

    // canonicalize 折叠大小写/符号链接等路径别名，保证同一物理目录只对应一个
    // session project bucket，避免会话被拆散到多份路径表示里。
    std::fs::canonicalize(&path).map_err(|error| {
        AstrError::io(
            format!("failed to canonicalize workingDir '{}'", path.display()),
            error,
        )
    })
}

/// 从工作目录提取展示名。
pub fn display_name_from_working_dir(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("默认项目")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use astrcode_core::AstrError;

    use super::{display_name_from_working_dir, normalize_session_id, normalize_working_dir};

    #[test]
    fn normalize_session_id_only_removes_outer_prefix() {
        assert_eq!(
            normalize_session_id("session-session-2026-03-08T10-00-00-aaaaaaaa"),
            "session-2026-03-08T10-00-00-aaaaaaaa"
        );
    }

    #[test]
    fn normalize_session_id_trims_outer_whitespace_before_removing_prefix() {
        assert_eq!(normalize_session_id("session-abc "), "abc");
        assert_eq!(normalize_session_id(" session-abc"), "abc");
        assert_eq!(normalize_session_id(" abc "), "abc");
    }

    #[test]
    fn normalize_working_dir_rejects_file_paths() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let file = temp_dir.path().join("file.txt");
        std::fs::write(&file, "demo").expect("file should be created");

        let err =
            normalize_working_dir(file).expect_err("file paths should not be accepted as workdir");

        assert!(matches!(err, AstrError::Validation(_)));
        assert!(err.to_string().contains("is not a directory"));
    }

    #[test]
    fn normalize_working_dir_rejects_missing_paths() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let missing = temp_dir.path().join("missing");

        let err = normalize_working_dir(missing).expect_err("missing workdir should fail");

        assert!(matches!(err, AstrError::Validation(_)));
        assert!(err.to_string().contains("is invalid"));
    }

    #[test]
    fn display_name_from_working_dir_uses_default_for_root() {
        #[cfg(windows)]
        let root = Path::new(r"C:\");
        #[cfg(not(windows))]
        let root = Path::new("/");

        assert_eq!(display_name_from_working_dir(root), "默认项目");
    }

    #[test]
    fn display_name_from_working_dir_ignores_trailing_separator() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let rendered = format!("{}{}", temp_dir.path().display(), std::path::MAIN_SEPARATOR);

        assert_eq!(
            display_name_from_working_dir(Path::new(&rendered)),
            temp_dir
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .expect("tempdir name should be utf-8")
        );
    }
}
