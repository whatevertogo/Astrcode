use std::fs;
use std::path::{Component, Path, PathBuf};

use astrcode_core::{AstrError, CancelToken, Result, ToolContext};
use serde::Serialize;

// Metadata conventions:
// - Path fields are returned as absolute path strings.
// - count/bytes/truncated/skipped_files are provided when they apply.
// - metadata is the machine-readable contract; output is display text only.
// - Structured machine data should not be embedded into output strings.

pub fn check_cancel(cancel: &CancelToken, _tool_name: &str) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(AstrError::Cancelled);
    }
    Ok(())
}

pub fn resolve_path(ctx: &ToolContext, path: &Path) -> Result<PathBuf> {
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.working_dir.join(path)
    };

    let resolved = normalize_lexically(&base);
    enforce_workspace_sandbox(&ctx.sandbox_root, &resolved)?;
    Ok(resolved)
}

fn enforce_workspace_sandbox(root: &Path, path: &Path) -> Result<()> {
    if !should_enforce_sandbox() {
        return Ok(());
    }

    let root = normalize_lexically(root);
    let normalized_path = normalize_lexically(path);

    if !normalized_path.starts_with(&root) {
        return Err(AstrError::SandboxEscape {
            path: normalized_path.display().to_string(),
        });
    }

    // Resolve the nearest existing ancestor so creating a new file beneath a symlinked
    // directory still gets checked against the real filesystem location.
    let existing_ancestor = nearest_existing_ancestor(&normalized_path).ok_or_else(|| {
        AstrError::ToolError {
            name: "sandbox".to_string(),
            reason: format!(
                "path '{}' has no existing ancestor to validate against sandbox '{}'",
                normalized_path.display(),
                root.display()
            ),
        }
    })?;
    let canonical_root = fs::canonicalize(&root).map_err(|e| {
        AstrError::io(
            format!("failed to canonicalize sandbox root '{}'", root.display()),
            e,
        )
    })?;
    let canonical_ancestor = fs::canonicalize(existing_ancestor).map_err(|e| {
        AstrError::io(
            format!(
                "failed to canonicalize path '{}' while enforcing sandbox",
                existing_ancestor.display()
            ),
            e,
        )
    })?;

    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(AstrError::SandboxEscape {
            path: format!(
                "{} (via symlink {})",
                normalized_path.display(),
                existing_ancestor.display()
            ),
        });
    }

    Ok(())
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
    fs::read_to_string(path).map_err(|e| AstrError::io(format!("failed reading file '{}'", path.display()), e))
}

pub async fn write_text_file(path: &Path, content: &str, create_dirs: bool) -> Result<usize> {
    if create_dirs {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AstrError::io(
                    format!("failed creating parent directory '{}'", parent.display()),
                    e,
                )
            })?;
        }
    }

    fs::write(path, content.as_bytes())
        .map_err(|e| AstrError::io(format!("failed writing file '{}'", path.display()), e))?;

    Ok(content.as_bytes().len())
}

pub fn json_output<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| AstrError::parse("failed to serialize output", e))
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

fn nearest_existing_ancestor(mut path: &Path) -> Option<&Path> {
    loop {
        if path.exists() {
            return Some(path);
        }
        path = path.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use crate::test_support::test_tool_context_for;

    use super::*;

    fn sandbox_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn check_cancel_returns_error_for_cancelled_token() {
        let ctx = test_tool_context_for(std::env::temp_dir());
        ctx.cancel.cancel();

        let err = check_cancel(&ctx.cancel, "grep").expect_err("cancelled token should fail");
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn resolve_path_returns_absolute_normalized_path() {
        let cwd = std::env::current_dir().expect("cwd should resolve");
        let ctx = test_tool_context_for(cwd.clone());
        let resolved =
            resolve_path(&ctx, Path::new("./src/../Cargo.toml")).expect("path should resolve");

        assert!(resolved.is_absolute());
        assert_eq!(resolved, cwd.join("Cargo.toml"));
    }

    #[test]
    fn resolve_path_rejects_escape_when_enforced() {
        let _guard = sandbox_env_lock().lock().expect("lock should work");
        std::env::set_var("ASTRCODE_ENFORCE_TOOL_SANDBOX", "1");

        let temp = tempfile::tempdir().expect("tempdir should be created");
        let outside = temp
            .path()
            .parent()
            .expect("parent should exist")
            .join("outside.txt");
        let ctx = test_tool_context_for(temp.path());
        let err = resolve_path(&ctx, &outside).expect_err("outside path should be rejected");

        std::env::remove_var("ASTRCODE_ENFORCE_TOOL_SANDBOX");
        assert!(err.to_string().contains("sandbox"));
    }

    #[test]
    fn resolve_path_allows_inside_workspace_when_enforced() {
        let _guard = sandbox_env_lock().lock().expect("lock should work");
        std::env::set_var("ASTRCODE_ENFORCE_TOOL_SANDBOX", "1");

        let temp = tempfile::tempdir().expect("tempdir should be created");
        let inside = temp.path().join("nested").join("file.txt");
        let ctx = test_tool_context_for(temp.path());
        let resolved = resolve_path(&ctx, &inside).expect("inside path should resolve");

        std::env::remove_var("ASTRCODE_ENFORCE_TOOL_SANDBOX");
        assert!(resolved.starts_with(temp.path()));
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn resolve_path_rejects_new_file_under_symlinked_directory_when_enforced() {
        let _guard = sandbox_env_lock().lock().expect("lock should work");
        std::env::set_var("ASTRCODE_ENFORCE_TOOL_SANDBOX", "1");

        let workspace = tempfile::tempdir().expect("workspace tempdir should be created");
        let outside = tempfile::tempdir().expect("outside tempdir should be created");
        let link = workspace.path().join("linked-outside");

        if let Err(error) = create_directory_symlink(outside.path(), &link) {
            std::env::remove_var("ASTRCODE_ENFORCE_TOOL_SANDBOX");
            if cfg!(windows) {
                return;
            }
            panic!("failed to create directory symlink: {error}");
        }

        let ctx = test_tool_context_for(workspace.path());
        let err = resolve_path(&ctx, &link.join("new.txt"))
            .expect_err("symlinked parent should be rejected");

        std::env::remove_var("ASTRCODE_ENFORCE_TOOL_SANDBOX");
        assert!(err.to_string().contains("sandbox"));
    }

    #[tokio::test]
    async fn write_and_read_text_file_round_trip() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("note.txt");

        let written: usize = write_text_file(&file, "hello", false)
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
