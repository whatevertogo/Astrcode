use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

// Metadata conventions:
// - Path fields are returned as absolute path strings.
// - count/bytes/truncated/skipped_files are provided when they apply.
// - metadata is the machine-readable contract; output is display text only.
// - Structured machine data should not be embedded into output strings.

pub fn check_cancel(cancel: &CancellationToken, tool_name: &str) -> Result<()> {
    if cancel.is_cancelled() {
        anyhow::bail!("{tool_name} interrupted");
    }
    Ok(())
}

pub fn resolve_path(path: &Path) -> Result<PathBuf> {
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(path)
    };

    let resolved = normalize_lexically(&base);
    enforce_workspace_sandbox(&resolved)?;
    Ok(resolved)
}

fn enforce_workspace_sandbox(path: &Path) -> Result<()> {
    if !should_enforce_sandbox() {
        return Ok(());
    }

    let root = normalize_lexically(
        &std::env::current_dir().context("failed to resolve current directory")?,
    );

    if path.starts_with(&root) {
        return Ok(());
    }

    anyhow::bail!(
        "path escapes workspace sandbox: '{}' is outside '{}'",
        path.display(),
        root.display()
    );
}

fn should_enforce_sandbox() -> bool {
    if std::env::var("ASTRCODE_DISABLE_TOOL_SANDBOX")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        return false;
    }

    #[cfg(test)]
    {
        std::env::var("ASTRCODE_ENFORCE_TOOL_SANDBOX")
            .map(|value| value == "1")
            .unwrap_or(false)
    }

    #[cfg(not(test))]
    {
        true
    }
}

pub async fn read_utf8_file(path: &Path) -> Result<String> {
    tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed reading file '{}'", path.display()))
}

pub async fn write_text_file(path: &Path, content: &str, create_dirs: bool) -> Result<usize> {
    if create_dirs {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed creating parent directory '{}'", parent.display())
            })?;
        }
    }

    tokio::fs::write(path, content.as_bytes())
        .await
        .with_context(|| format!("failed writing file '{}'", path.display()))?;

    Ok(content.as_bytes().len())
}

pub fn json_output<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).context("failed to serialize output")
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = normalized.pop();
                if !popped {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use crate::test_support::TestEnvGuard;

    use super::*;

    #[test]
    fn check_cancel_returns_error_for_cancelled_token() {
        let token = CancellationToken::new();
        token.cancel();

        let err = check_cancel(&token, "grep").expect_err("cancelled token should fail");
        assert!(err.to_string().contains("grep interrupted"));
    }

    #[test]
    fn resolve_path_returns_absolute_normalized_path() {
        let _guard = TestEnvGuard::new();
        let cwd = std::env::current_dir().expect("cwd should resolve");
        let resolved = resolve_path(Path::new("./src/../Cargo.toml")).expect("path should resolve");

        assert!(resolved.is_absolute());
        assert_eq!(resolved, cwd.join("Cargo.toml"));
    }

    #[test]
    fn resolve_path_rejects_escape_when_enforced() {
        let test_guard = TestEnvGuard::new();
        test_guard.set_current_dir(test_guard.home_dir());
        std::env::set_var("ASTRCODE_ENFORCE_TOOL_SANDBOX", "1");

        let outside = test_guard
            .home_dir()
            .parent()
            .expect("parent should exist")
            .join("outside.txt");
        let err = resolve_path(&outside).expect_err("outside path should be rejected");

        std::env::remove_var("ASTRCODE_ENFORCE_TOOL_SANDBOX");
        assert!(err.to_string().contains("escapes workspace sandbox"));
    }

    #[test]
    fn resolve_path_allows_inside_workspace_when_enforced() {
        let test_guard = TestEnvGuard::new();
        test_guard.set_current_dir(test_guard.home_dir());
        std::env::set_var("ASTRCODE_ENFORCE_TOOL_SANDBOX", "1");

        let inside = test_guard.home_dir().join("nested").join("file.txt");
        let resolved = resolve_path(&inside).expect("inside path should resolve");

        std::env::remove_var("ASTRCODE_ENFORCE_TOOL_SANDBOX");
        assert!(resolved.starts_with(test_guard.home_dir()));
    }

    #[tokio::test]
    async fn write_and_read_text_file_round_trip() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("note.txt");

        let written = write_text_file(&file, "hello", false)
            .await
            .expect("write should succeed");
        let content = read_utf8_file(&file).await.expect("read should succeed");

        assert_eq!(written, 5);
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn write_text_file_creates_parent_directories() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("nested").join("dir").join("note.txt");

        write_text_file(&file, "hello", true)
            .await
            .expect("write should succeed");

        assert!(file.exists());
    }
}
