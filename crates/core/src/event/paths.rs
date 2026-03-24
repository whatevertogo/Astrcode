use std::path::PathBuf;

use crate::{AstrError, Result};
use uuid::Uuid;

pub fn generate_session_id() -> String {
    let dt = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
    let uuid = Uuid::new_v4().simple().to_string();
    let short = &uuid[..8];
    format!("{dt}-{short}")
}

pub(crate) fn sessions_dir() -> Result<PathBuf> {
    let home = resolve_home_dir()?;
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
        return Err(AstrError::InvalidSessionId(session_id.to_string()));
    }
    Ok(canonical.to_string())
}

pub(crate) fn session_path(session_id: &str) -> Result<PathBuf> {
    let session_id = validated_session_id(session_id)?;
    Ok(sessions_dir()?.join(format!("session-{session_id}.jsonl")))
}

fn legacy_prefixed_path(session_id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("session-{session_id}.jsonl")))
}

pub(crate) fn resolve_existing_session_path(session_id: &str) -> Result<PathBuf> {
    let _ = validated_session_id(session_id)?;
    let canonical = session_path(session_id)?;
    if canonical.exists() {
        return Ok(canonical);
    }

    let legacy = legacy_prefixed_path(session_id)?;
    if legacy != canonical && legacy.exists() {
        return Ok(legacy);
    }

    Err(AstrError::SessionNotFound(canonical.display().to_string()))
}

pub(crate) fn resolve_home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("ASTRCODE_TEST_HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }

    #[cfg(test)]
    if let Some(home) = crate::test_support::test_home_dir() {
        return Ok(home);
    }

    #[cfg(test)]
    {
        return Err(AstrError::Internal(format!(
            "{} must be set before tests call sessions_dir()",
            crate::test_support::TEST_HOME_ENV
        )));
    }

    #[cfg(not(test))]
    {
        const APP_HOME_OVERRIDE_ENV: &str = "ASTRCODE_HOME_DIR";

        if let Some(home) = std::env::var_os(APP_HOME_OVERRIDE_ENV) {
            if !home.is_empty() {
                return Ok(PathBuf::from(home));
            }
        }

        dirs::home_dir().ok_or(AstrError::HomeDirectoryNotFound)
    }
}
