pub mod loader;
pub mod scorer;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::trace::SessionTrace;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalTask {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceSpec>,
    pub expected_outcome: ExpectedOutcome,
    #[serde(default)]
    pub scoring: ScoringWeights,
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkspaceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ExpectedOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_pattern: Option<ToolPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_changes: Vec<FileChangeExpectation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_contains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_equals: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ToolPattern {
    pub sequence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChangeExpectation {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoringWeights {
    pub tool_pattern: f64,
    pub tool_call_budget: f64,
    pub file_changes: f64,
    pub turn_budget: f64,
    pub output: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            tool_pattern: 0.25,
            tool_call_budget: 0.2,
            file_changes: 0.35,
            turn_budget: 0.1,
            output: 0.1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalScore {
    pub score: f64,
    pub status: EvalStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<DimensionScore>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalStatus {
    Pass,
    Partial,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimensionScore {
    pub name: String,
    pub score: f64,
    pub weight: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl EvalTask {
    pub fn validate(&self) -> Result<(), crate::EvalError> {
        if self.task_id.trim().is_empty() {
            return Err(crate::EvalError::validation(
                "任务缺少必要字段 task_id 或值为空",
            ));
        }
        if self.prompt.trim().is_empty() {
            return Err(crate::EvalError::validation(
                "任务缺少必要字段 prompt 或值为空",
            ));
        }
        if !self
            .task_id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        {
            return Err(crate::EvalError::validation(format!(
                "task_id '{}' 必须为 kebab-case",
                self.task_id
            )));
        }
        if self.expected_outcome == ExpectedOutcome::default() {
            return Err(crate::EvalError::validation(
                "任务缺少必要字段 expected_outcome",
            ));
        }
        Ok(())
    }
}

impl EvalScore {
    pub fn from_dimensions(dimensions: Vec<DimensionScore>, file_changes_failed: bool) -> Self {
        if file_changes_failed {
            return Self {
                score: 0.0,
                status: EvalStatus::Fail,
                dimensions,
            };
        }

        let total_weight = dimensions.iter().map(|item| item.weight).sum::<f64>();
        let weighted_score = if total_weight > 0.0 {
            dimensions
                .iter()
                .map(|item| item.score * item.weight)
                .sum::<f64>()
                / total_weight
        } else {
            1.0
        };

        let status = if (weighted_score - 1.0).abs() < f64::EPSILON {
            EvalStatus::Pass
        } else if weighted_score <= 0.0 {
            EvalStatus::Fail
        } else {
            EvalStatus::Partial
        };

        Self {
            score: weighted_score.clamp(0.0, 1.0),
            status,
            dimensions,
        }
    }
}

pub fn last_assistant_output(trace: &SessionTrace) -> Option<&str> {
    trace
        .turns
        .iter()
        .rev()
        .find_map(|turn| turn.assistant_output.as_deref())
}

#[cfg(test)]
mod tests {
    use super::EvalTask;

    #[test]
    fn eval_task_deserializes_from_yaml() {
        let yaml = r#"
task_id: file-edit-precision
description: update file precisely
prompt: |
  edit the file
workspace:
  setup: fixtures/file-edit
expected_outcome:
  tool_pattern:
    - Read
    - Edit
  max_tool_calls: 4
  file_changes:
    - path: src/main.rs
      contains: "println!"
  max_turns: 1
scoring:
  tool_pattern: 0.3
  tool_call_budget: 0.2
  file_changes: 0.3
  turn_budget: 0.1
  output: 0.1
"#;
        let task: EvalTask = serde_yaml::from_str(yaml).expect("yaml should deserialize");
        task.validate().expect("task should be valid");
        assert_eq!(task.task_id, "file-edit-precision");
        assert_eq!(
            task.expected_outcome
                .tool_pattern
                .as_ref()
                .expect("pattern should exist")
                .sequence,
            vec!["Read", "Edit"]
        );
    }
}
