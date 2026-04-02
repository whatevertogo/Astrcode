use std::fs;
use std::path::{Path, PathBuf};

use astrcode_core::project::{project_dir, project_dir_name, projects_dir};
use astrcode_core::store::StoreError;

use crate::{internal_io_error, AstrError, Result};

const SESSIONS_DIR_NAME: &str = "sessions";

pub(crate) fn projects_root_dir() -> Result<PathBuf> {
    projects_dir().map_err(|error| {
        internal_io_error(format!(
            "failed to resolve Astrcode projects directory: {error}"
        ))
    })
}

pub(crate) fn project_sessions_dir(working_dir: &Path) -> Result<PathBuf> {
    Ok(project_dir(working_dir)
        .map_err(|error| {
            internal_io_error(format!(
                "failed to resolve project directory for '{}': {error}",
                working_dir.display()
            ))
        })?
        .join(SESSIONS_DIR_NAME))
}

pub(crate) fn project_sessions_dir_from_root(projects_root: &Path, working_dir: &Path) -> PathBuf {
    projects_root
        .join(project_dir_name(working_dir))
        .join(SESSIONS_DIR_NAME)
}

pub(crate) fn session_dir(session_id: &str, working_dir: &Path) -> Result<PathBuf> {
    let session_id = validated_session_id(session_id)?;
    Ok(project_sessions_dir(working_dir)?.join(&session_id))
}

pub(crate) fn session_path(session_id: &str, working_dir: &Path) -> Result<PathBuf> {
    let session_id = validated_session_id(session_id)?;
    Ok(session_dir(&session_id, working_dir)?.join(session_file_name(&session_id)))
}

pub(crate) fn resolve_existing_session_path(session_id: &str) -> Result<PathBuf> {
    let session_id = validated_session_id(session_id)?;
    let projects_root = projects_root_dir()?;
    let candidate_name = session_file_name(&session_id);

    for sessions_dir in session_storage_dirs(&projects_root)? {
        let candidate = sessions_dir.join(&session_id).join(&candidate_name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(StoreError::SessionNotFound(
        projects_root
            .join("<project>")
            .join(SESSIONS_DIR_NAME)
            .join(&session_id)
            .join(candidate_name)
            .display()
            .to_string(),
    ))
}

pub(crate) fn session_storage_dirs(projects_root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    if projects_root.exists() {
        for entry in fs::read_dir(projects_root).map_err(|error| {
            AstrError::io(
                format!(
                    "failed to read projects directory: {}",
                    projects_root.display()
                ),
                error,
            )
        })? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let sessions_dir = entry.path().join(SESSIONS_DIR_NAME);
            if sessions_dir.is_dir() {
                dirs.push(sessions_dir);
            }
        }
    }
    Ok(dirs)
}

pub(crate) fn session_file_name(session_id: &str) -> String {
    format!("session-{session_id}.jsonl")
}

/// 宽容归一化：接受带 "session-" 前缀和不带前缀的 ID。
/// 设计意图：API 调用者可能传入 "session-xxx" 或 "xxx" 两种格式，
/// 此函数统一剥离前缀，避免调用方各自处理前缀逻辑。
pub(crate) fn canonical_session_id(session_id: &str) -> &str {
    session_id.strip_prefix("session-").unwrap_or(session_id)
}

/// 验证会话 ID 只含安全字符。显式允许 'T' 是因为 ID 中嵌入了类 ISO-8601
/// 时间戳（如 "2026-03-08T10-00-00"），'T' 是日期与时间的分隔符。
/// 不允许 ':' 是因为冒号在 Windows 文件名中非法（session ID 直接用于文件名）。
pub(crate) fn is_valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == 'T')
}

pub(crate) fn validated_session_id(session_id: &str) -> Result<String> {
    let canonical = canonical_session_id(session_id);
    if !is_valid_session_id(canonical) {
        return Err(StoreError::InvalidSessionId(session_id.to_string()));
    }
    Ok(canonical.to_string())
}

#[cfg(test)]
mod tests {
    use astrcode_core::test_support::TestEnvGuard;

    use super::*;

    #[test]
    fn session_path_rejects_invalid_session_id() {
        let err = session_path("../../etc/passwd", Path::new(r"D:\project"))
            .expect_err("invalid id should fail");
        assert!(err.to_string().contains("invalid session id"));
    }

    #[test]
    fn session_path_uses_project_session_directory() {
        let guard = TestEnvGuard::new();
        let working_dir = Path::new(r"D:\project1");

        let path =
            session_path("2026-04-02T10-00-00-aaaaaaaa", working_dir).expect("path should resolve");

        assert!(path.starts_with(guard.home_dir().join(".astrcode").join("projects")));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("session-2026-04-02T10-00-00-aaaaaaaa.jsonl")
        );
        assert_eq!(
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str()),
            Some("2026-04-02T10-00-00-aaaaaaaa")
        );
        assert_eq!(
            path.parent()
                .and_then(|parent| parent.parent())
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str()),
            Some("sessions")
        );
    }

    #[test]
    fn session_storage_dirs_lists_project_session_folders_only() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        fs::create_dir_all(temp.path().join("project-a").join("sessions"))
            .expect("sessions dir should exist");
        fs::create_dir_all(temp.path().join("project-b").join("sessions"))
            .expect("sessions dir should exist");
        fs::create_dir_all(temp.path().join("project-c").join("notes"))
            .expect("non-session dir should exist");

        let dirs = session_storage_dirs(temp.path()).expect("project session dirs should resolve");

        assert_eq!(dirs.len(), 2);
        assert!(dirs.iter().all(|dir| dir.ends_with("sessions")));
    }
}
