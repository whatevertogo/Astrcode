use std::path::PathBuf;

use crate::{internal_io_error, Result};
use astrcode_core::store::StoreError;

pub(crate) fn sessions_dir() -> Result<PathBuf> {
    // 继续复用 core 的 home 解析，避免在 storage 里复制环境变量约定。
    let home = astrcode_core::home::resolve_home_dir().map_err(|error| {
        internal_io_error(format!(
            "failed to resolve Astrcode home directory: {error}"
        ))
    })?;
    Ok(home.join(".astrcode").join("sessions"))
}

pub(crate) fn canonical_session_id(session_id: &str) -> &str {
    session_id.strip_prefix("session-").unwrap_or(session_id)
}

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

pub(crate) fn session_path(session_id: &str) -> Result<PathBuf> {
    let session_id = validated_session_id(session_id)?;
    Ok(sessions_dir()?.join(format!("session-{session_id}.jsonl")))
}

pub(crate) fn resolve_existing_session_path(session_id: &str) -> Result<PathBuf> {
    let _ = validated_session_id(session_id)?;
    let path = session_path(session_id)?;
    if path.exists() {
        return Ok(path);
    }

    Err(StoreError::SessionNotFound(path.display().to_string()))
}
