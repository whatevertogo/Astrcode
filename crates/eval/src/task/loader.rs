use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::EvalTask;
use crate::{EvalError, EvalResult, TaskLoadWarning};

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedTaskSet {
    pub tasks: Vec<EvalTask>,
    pub warnings: Vec<TaskLoadWarning>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskSetIndex {
    #[serde(default)]
    tasks: Vec<String>,
}

pub struct TaskLoader;

impl TaskLoader {
    pub fn load_task(path: impl AsRef<Path>) -> EvalResult<EvalTask> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|error| {
            EvalError::io(format!("读取任务文件 {} 失败", path.display()), error)
        })?;
        let mut task = serde_yaml::from_str::<EvalTask>(&content)
            .map_err(|error| EvalError::yaml(path.display().to_string(), error))?;
        task.source_path = Some(path.to_path_buf());
        task.validate()?;
        Ok(task)
    }

    pub fn load_dir(path: impl AsRef<Path>) -> EvalResult<Vec<EvalTask>> {
        let path = path.as_ref();
        let mut files: Vec<PathBuf> = fs::read_dir(path)
            .map_err(|error| EvalError::io(format!("读取目录 {} 失败", path.display()), error))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|entry| {
                entry.is_file()
                    && entry
                        .extension()
                        .is_some_and(|ext| ext == "yaml" || ext == "yml")
            })
            .collect();
        files.sort();

        let mut seen = HashSet::new();
        let mut tasks = Vec::with_capacity(files.len());
        for file in files {
            let task = Self::load_task(&file)?;
            if !seen.insert(task.task_id.clone()) {
                return Err(EvalError::validation(format!(
                    "重复的 task_id '{}'",
                    task.task_id
                )));
            }
            tasks.push(task);
        }
        Ok(tasks)
    }

    pub fn load_task_set(path: impl AsRef<Path>) -> EvalResult<LoadedTaskSet> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|error| {
            EvalError::io(format!("读取任务集索引 {} 失败", path.display()), error)
        })?;
        let index = serde_yaml::from_str::<TaskSetIndex>(&content)
            .map_err(|error| EvalError::yaml(path.display().to_string(), error))?;
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        let mut tasks = Vec::new();

        for relative_path in index.tasks {
            let task_path = base_dir.join(&relative_path);
            match Self::load_task(&task_path) {
                Ok(task) => {
                    if seen.insert(task.task_id.clone()) {
                        tasks.push(task);
                    } else {
                        warnings.push(TaskLoadWarning {
                            path: task_path,
                            message: format!("跳过重复 task_id '{}'", task.task_id),
                        });
                    }
                },
                Err(error) => warnings.push(TaskLoadWarning {
                    path: task_path,
                    message: error.to_string(),
                }),
            }
        }

        Ok(LoadedTaskSet { tasks, warnings })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::TaskLoader;

    #[test]
    fn loader_loads_directory_and_validates_required_fields() {
        let dir = tempdir().expect("tempdir should create");
        let good = dir.path().join("good.yaml");
        let bad = dir.path().join("bad.yaml");

        fs::write(
            &good,
            r#"
task_id: file-read
prompt: read the file
expected_outcome:
  max_tool_calls: 2
"#,
        )
        .expect("good file should write");
        fs::write(
            &bad,
            r#"
task_id: ""
prompt: ""
expected_outcome:
  max_tool_calls: 1
"#,
        )
        .expect("bad file should write");

        let error = TaskLoader::load_dir(dir.path()).expect_err("invalid task should fail");
        assert!(error.to_string().contains("task_id"));
    }

    #[test]
    fn loader_loads_task_set_and_collects_warnings() {
        let dir = tempdir().expect("tempdir should create");
        fs::write(
            dir.path().join("task-set.yaml"),
            r#"
tasks:
  - valid.yaml
  - missing.yaml
  - invalid.yaml
"#,
        )
        .expect("task-set should write");
        fs::write(
            dir.path().join("valid.yaml"),
            r#"
task_id: valid-task
prompt: do it
expected_outcome:
  max_tool_calls: 1
"#,
        )
        .expect("valid task should write");
        fs::write(
            dir.path().join("invalid.yaml"),
            r#"
task_id: invalid-task
prompt: ""
expected_outcome:
  max_tool_calls: 1
"#,
        )
        .expect("invalid task should write");

        let loaded = TaskLoader::load_task_set(dir.path().join("task-set.yaml"))
            .expect("task-set should load");
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.warnings.len(), 2);
    }
}
