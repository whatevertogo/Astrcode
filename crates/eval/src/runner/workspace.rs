use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;

use crate::{EvalError, EvalResult, task::EvalTask};

#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    pub root: PathBuf,
    pub keep_workspace: bool,
}

impl WorkspaceManager {
    pub fn new(root: PathBuf, keep_workspace: bool) -> Self {
        Self {
            root,
            keep_workspace,
        }
    }

    pub fn create_root(&self) -> EvalResult<()> {
        fs::create_dir_all(&self.root).map_err(|error| {
            EvalError::io(
                format!("创建评测工作区根目录 {} 失败", self.root.display()),
                error,
            )
        })
    }

    pub fn prepare(&self, task: &EvalTask) -> EvalResult<PathBuf> {
        self.create_root()?;
        let workspace_path = self.root.join(format!(
            "{}-{}",
            task.task_id,
            Utc::now().timestamp_millis()
        ));
        if workspace_path.exists() {
            fs::remove_dir_all(&workspace_path).map_err(|error| {
                EvalError::io(
                    format!("清理旧工作区 {} 失败", workspace_path.display()),
                    error,
                )
            })?;
        }
        fs::create_dir_all(&workspace_path).map_err(|error| {
            EvalError::io(
                format!("创建隔离工作区 {} 失败", workspace_path.display()),
                error,
            )
        })?;

        if let Some(source_dir) = resolve_fixture_dir(task)? {
            copy_dir_recursive(&source_dir, &workspace_path)?;
        }

        Ok(workspace_path)
    }

    pub fn cleanup(&self, workspace_path: &Path) -> EvalResult<()> {
        if self.keep_workspace || !workspace_path.exists() {
            return Ok(());
        }
        fs::remove_dir_all(workspace_path).map_err(|error| {
            EvalError::io(
                format!("删除隔离工作区 {} 失败", workspace_path.display()),
                error,
            )
        })
    }
}

fn resolve_fixture_dir(task: &EvalTask) -> EvalResult<Option<PathBuf>> {
    let Some(setup) = task
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.setup.clone())
    else {
        return Ok(None);
    };

    let path = PathBuf::from(&setup);
    if path.is_absolute() {
        return Ok(Some(path));
    }

    let Some(source_path) = task.source_path.as_ref() else {
        return Ok(Some(path));
    };
    let base_dir = source_path.parent().ok_or_else(|| {
        EvalError::validation(format!("任务文件 {} 没有父目录", source_path.display()))
    })?;
    Ok(Some(base_dir.join(path)))
}

fn copy_dir_recursive(source: &Path, target: &Path) -> EvalResult<()> {
    if !source.exists() {
        return Err(EvalError::validation(format!(
            "fixture 目录不存在: {}",
            source.display()
        )));
    }

    for entry in fs::read_dir(source).map_err(|error| {
        EvalError::io(
            format!("读取 fixture 目录 {} 失败", source.display()),
            error,
        )
    })? {
        let entry = entry.map_err(|error| {
            EvalError::io(
                format!("遍历 fixture 目录 {} 失败", source.display()),
                error,
            )
        })?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| {
            EvalError::io(
                format!("读取文件类型 {} 失败", source_path.display()),
                error,
            )
        })?;
        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| {
                EvalError::io(format!("创建目录 {} 失败", target_path.display()), error)
            })?;
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).map_err(|error| {
                EvalError::io(
                    format!(
                        "复制 fixture 文件 {} -> {} 失败",
                        source_path.display(),
                        target_path.display()
                    ),
                    error,
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::WorkspaceManager;
    use crate::task::EvalTask;

    #[test]
    fn workspace_manager_prepares_and_cleans_fixture_workspace() {
        let temp = tempdir().expect("tempdir should create");
        let fixture_dir = temp.path().join("fixtures");
        fs::create_dir_all(&fixture_dir).expect("fixture dir should create");
        fs::write(fixture_dir.join("README.md"), "hello").expect("fixture file should write");

        let task: EvalTask = serde_yaml::from_str(
            r#"
task_id: file-read
prompt: read
workspace:
  setup: fixtures
expected_outcome:
  max_tool_calls: 1
"#,
        )
        .expect("task should deserialize");
        let mut task = task;
        task.source_path = Some(temp.path().join("task.yaml"));

        let manager = WorkspaceManager::new(temp.path().join("workspaces"), false);
        let workspace = manager.prepare(&task).expect("workspace should prepare");
        assert!(workspace.join("README.md").exists());

        manager
            .cleanup(&workspace)
            .expect("workspace should cleanup");
        assert!(!workspace.exists());
    }
}
