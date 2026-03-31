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
    let working_dir = canonicalize_path(
        ctx.working_dir(),
        &format!(
            "failed to canonicalize working directory '{}'",
            ctx.working_dir().display()
        ),
    )?;
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_dir.join(path)
    };

    let resolved = resolve_for_boundary_check(&normalize_lexically(&base))?;
    if is_path_within_root(&resolved, &working_dir) {
        return Ok(resolved);
    }

    Err(AstrError::Validation(format!(
        "path '{}' escapes working directory '{}'",
        path.display(),
        working_dir.display()
    )))
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

fn resolve_for_boundary_check(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return canonicalize_path(
            path,
            &format!("failed to canonicalize path '{}'", path.display()),
        );
    }

    let mut missing_components = Vec::new();
    let mut current = path;
    while !current.exists() {
        let Some(name) = current.file_name() else {
            return Err(AstrError::Validation(format!(
                "path '{}' cannot be resolved under the working directory",
                path.display()
            )));
        };
        let Some(parent) = current.parent() else {
            return Err(AstrError::Validation(format!(
                "path '{}' cannot be resolved under the working directory",
                path.display()
            )));
        };
        missing_components.push(name.to_os_string());
        current = parent;
    }

    let mut resolved_parent = canonicalize_path(
        current,
        &format!("failed to canonicalize path '{}'", current.display()),
    )?;
    for component in missing_components.iter().rev() {
        resolved_parent.push(component);
    }

    Ok(normalize_lexically(&resolved_parent))
}

fn canonicalize_path(path: &Path, context: &str) -> Result<PathBuf> {
    fs::canonicalize(path)
        .map(normalize_absolute_path)
        .map_err(|e| AstrError::io(context.to_string(), e))
}

fn is_path_within_root(path: &Path, root: &Path) -> bool {
    let normalized_path = normalize_lexically(path);
    let normalized_root = normalize_lexically(root);
    normalized_path == normalized_root || normalized_path.starts_with(&normalized_root)
}

fn normalize_absolute_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(rendered) = path.to_str() {
            if let Some(stripped) = rendered.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{}", stripped));
            }
            if let Some(stripped) = rendered.strip_prefix(r"\\?\") {
                return PathBuf::from(stripped);
            }
        }
    }

    path
}

#[cfg(test)]
mod tests {
    use crate::test_support::test_tool_context_for;

    use super::*;

    #[test]
    fn check_cancel_returns_error_for_cancelled_token() {
        let ctx = test_tool_context_for(std::env::temp_dir());
        ctx.cancel().cancel();

        let err = check_cancel(ctx.cancel(), "grep").expect_err("cancelled token should fail");
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
    fn resolve_path_rejects_relative_escape_from_working_dir() {
        let parent = tempfile::tempdir().expect("tempdir should be created");
        let working_dir = parent.path().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should be created");
        let ctx = test_tool_context_for(&working_dir);

        let err = resolve_path(&ctx, Path::new("../outside.txt"))
            .expect_err("escaping path should be rejected");

        assert!(matches!(err, AstrError::Validation(_)));
        assert!(err.to_string().contains("escapes working directory"));
    }

    #[test]
    fn resolve_path_rejects_absolute_path_outside_working_dir() {
        let working_dir = tempfile::tempdir().expect("tempdir should be created");
        let outside_dir = tempfile::tempdir().expect("tempdir should be created");
        let outside = outside_dir.path().join("outside.txt");
        fs::write(&outside, "outside").expect("outside file should be created");
        let ctx = test_tool_context_for(working_dir.path());

        let err =
            resolve_path(&ctx, &outside).expect_err("absolute path outside working dir fails");

        assert!(matches!(err, AstrError::Validation(_)));
        assert!(err.to_string().contains("escapes working directory"));
    }

    #[test]
    fn resolve_path_allows_absolute_path_inside_working_dir() {
        let working_dir = tempfile::tempdir().expect("tempdir should be created");
        let file = working_dir.path().join("notes.txt");
        fs::write(&file, "hello").expect("file should be created");
        let ctx = test_tool_context_for(working_dir.path());

        let resolved = resolve_path(&ctx, &file).expect("path should resolve");

        assert_eq!(resolved, file);
    }

    #[test]
    fn is_path_within_root_ignores_trailing_separators() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let root = temp.path().join("workspace");
        fs::create_dir_all(root.join("nested")).expect("workspace should be created");
        let root_with_separator =
            PathBuf::from(format!("{}{}", root.display(), std::path::MAIN_SEPARATOR));

        assert!(is_path_within_root(
            &root.join("nested"),
            &root_with_separator
        ));
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
