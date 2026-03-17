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

    Ok(normalize_lexically(&base))
}

pub async fn read_utf8_file(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|e| AstrError::io(format!("failed reading file '{}'", path.display()), e))
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

#[cfg(test)]
mod tests {
    use crate::test_support::test_tool_context_for;

    use super::*;

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
