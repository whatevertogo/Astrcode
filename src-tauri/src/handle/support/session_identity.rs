use std::path::PathBuf;

use astrcode_core::AgentRuntime;

pub(crate) fn canonical_session_id(session_id: &str) -> &str {
    session_id.strip_prefix("session-").unwrap_or(session_id)
}

fn normalize_working_dir(working_dir: &str) -> String {
    let trimmed = working_dir.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        working_dir.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn same_working_dir(a: &str, b: &str) -> bool {
    let left = normalize_working_dir(a);
    let right = normalize_working_dir(b);
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub(crate) fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(dirs::home_dir)
}

pub(crate) fn sync_runtime_working_dir(runtime: &AgentRuntime) {
    if let Ok(state) = runtime.state() {
        let _ = std::env::set_current_dir(&state.working_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_session_id_strips_prefix_once() {
        assert_eq!(
            canonical_session_id("session-2026-03-08T10-00-00-aaaaaaaa"),
            "2026-03-08T10-00-00-aaaaaaaa"
        );
        assert_eq!(
            canonical_session_id("2026-03-08T10-00-00-aaaaaaaa"),
            "2026-03-08T10-00-00-aaaaaaaa"
        );
    }
}
