use std::{fs, path::Path};

use super::{
    DimensionScore, EvalScore, EvalTask, FileChangeExpectation, ToolPattern, last_assistant_output,
};
use crate::{EvalError, EvalResult, trace::SessionTrace};

pub struct TaskScorer;

impl TaskScorer {
    pub fn score(task: &EvalTask, trace: &SessionTrace, workspace: &Path) -> EvalResult<EvalScore> {
        let actual_tools: Vec<&str> = trace
            .turns
            .iter()
            .flat_map(|turn| turn.tool_calls.iter().map(|call| call.tool_name.as_str()))
            .collect();
        let total_turns = trace.turns.len();

        let mut dimensions = Vec::new();

        if let Some(pattern) = &task.expected_outcome.tool_pattern {
            dimensions.push(Self::score_tool_pattern(
                pattern,
                &actual_tools,
                task.scoring.tool_pattern,
            ));
        }

        if let Some(max_tool_calls) = task.expected_outcome.max_tool_calls {
            let score = if actual_tools.len() <= max_tool_calls {
                1.0
            } else {
                0.0
            };
            dimensions.push(DimensionScore {
                name: "tool_call_budget".to_string(),
                score,
                weight: task.scoring.tool_call_budget,
                detail: Some(format!(
                    "actual={}, limit={}",
                    actual_tools.len(),
                    max_tool_calls
                )),
            });
        }

        let file_change_score = if task.expected_outcome.file_changes.is_empty() {
            None
        } else {
            Some(Self::score_file_changes(
                &task.expected_outcome.file_changes,
                workspace,
                task.scoring.file_changes,
            )?)
        };
        if let Some(score) = &file_change_score {
            dimensions.push(score.clone());
        }

        if let Some(max_turns) = task.expected_outcome.max_turns {
            let score = if total_turns <= max_turns { 1.0 } else { 0.0 };
            dimensions.push(DimensionScore {
                name: "turn_budget".to_string(),
                score,
                weight: task.scoring.turn_budget,
                detail: Some(format!("actual={}, limit={}", total_turns, max_turns)),
            });
        }

        if !task.expected_outcome.output_contains.is_empty()
            || task.expected_outcome.output_equals.is_some()
        {
            dimensions.push(Self::score_output(task, trace, task.scoring.output));
        }

        let file_changes_failed = file_change_score
            .as_ref()
            .is_some_and(|item| item.score < 1.0);
        Ok(EvalScore::from_dimensions(dimensions, file_changes_failed))
    }

    fn score_tool_pattern(
        pattern: &ToolPattern,
        actual_tools: &[&str],
        weight: f64,
    ) -> DimensionScore {
        let matched = pattern
            .sequence
            .iter()
            .zip(actual_tools.iter())
            .take_while(|(expected, actual)| expected.as_str() == **actual)
            .count();
        let base_score = if pattern.sequence.is_empty() {
            1.0
        } else if matched == pattern.sequence.len() && actual_tools.len() > pattern.sequence.len() {
            pattern.sequence.len() as f64 / actual_tools.len() as f64
        } else {
            matched as f64 / pattern.sequence.len() as f64
        };

        DimensionScore {
            name: "tool_pattern".to_string(),
            score: base_score,
            weight,
            detail: Some(format!(
                "expected={:?}, actual={:?}",
                pattern.sequence, actual_tools
            )),
        }
    }

    fn score_file_changes(
        expectations: &[FileChangeExpectation],
        workspace: &Path,
        weight: f64,
    ) -> EvalResult<DimensionScore> {
        let mut matched = 0usize;
        let mut details = Vec::with_capacity(expectations.len());
        for expectation in expectations {
            let path = workspace.join(&expectation.path);
            let exists = path.exists();
            let expected_exists = expectation
                .exists
                .unwrap_or(expectation.contains.is_some() || expectation.exact.is_some());

            if expected_exists && !exists {
                details.push(format!("{}: 文件不存在", expectation.path));
                continue;
            }

            if !expected_exists && !exists {
                matched += 1;
                details.push(format!("{}: 符合不存在预期", expectation.path));
                continue;
            }

            let content = fs::read_to_string(&path).map_err(|error| {
                EvalError::io(format!("读取工作区文件 {} 失败", path.display()), error)
            })?;
            let exact_ok = expectation
                .exact
                .as_ref()
                .is_none_or(|expected| content == *expected);
            let contains_ok = expectation
                .contains
                .as_ref()
                .is_none_or(|snippet| content.contains(snippet));

            if exact_ok && contains_ok {
                matched += 1;
                details.push(format!("{}: 命中预期", expectation.path));
            } else {
                details.push(format!("{}: 内容不匹配", expectation.path));
            }
        }

        let score = matched as f64 / expectations.len() as f64;
        Ok(DimensionScore {
            name: "file_changes".to_string(),
            score,
            weight,
            detail: Some(details.join("; ")),
        })
    }

    fn score_output(task: &EvalTask, trace: &SessionTrace, weight: f64) -> DimensionScore {
        let output = last_assistant_output(trace).unwrap_or_default();
        let contains_score = if task.expected_outcome.output_contains.is_empty() {
            1.0
        } else {
            task.expected_outcome
                .output_contains
                .iter()
                .filter(|snippet| output.contains(snippet.as_str()))
                .count() as f64
                / task.expected_outcome.output_contains.len() as f64
        };
        let exact_score = task
            .expected_outcome
            .output_equals
            .as_ref()
            .map(|expected| if output == expected { 1.0 } else { 0.0 })
            .unwrap_or(1.0);

        DimensionScore {
            name: "output".to_string(),
            score: contains_score.min(exact_score),
            weight,
            detail: Some(format!("output={output}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::TaskScorer;
    use crate::{
        task::{EvalStatus, EvalTask},
        trace::{SessionTrace, ToolCallRecord, TurnTrace},
    };

    fn trace_with_tools(tool_names: &[&str], assistant_output: &str, turns: usize) -> SessionTrace {
        SessionTrace {
            session_id: Some("session-1".to_string()),
            working_dir: None,
            started_at: None,
            parent_session_id: None,
            parent_storage_seq: None,
            turns: (0..turns)
                .map(|index| TurnTrace {
                    turn_id: format!("turn-{index}"),
                    user_input: None,
                    assistant_output: Some(assistant_output.to_string()),
                    assistant_reasoning: None,
                    thinking_deltas: Vec::new(),
                    tool_calls: if index == 0 {
                        tool_names
                            .iter()
                            .enumerate()
                            .map(|(tool_index, tool_name)| ToolCallRecord {
                                tool_call_id: format!("call-{tool_index}"),
                                tool_name: (*tool_name).to_string(),
                                args: serde_json::Value::Null,
                                output: None,
                                success: Some(true),
                                error: None,
                                metadata: None,
                                continuation: None,
                                duration_ms: None,
                                started_storage_seq: None,
                                finished_storage_seq: None,
                                stream_deltas: Vec::new(),
                                persisted_reference: None,
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                    prompt_metrics: Vec::new(),
                    compactions: Vec::new(),
                    sub_runs: Vec::new(),
                    collaboration_facts: Vec::new(),
                    errors: Vec::new(),
                    timeline: Vec::new(),
                    agent_lineage: Vec::new(),
                    storage_seq_range: None,
                    completed_at: None,
                    completion_reason: None,
                    incomplete: false,
                })
                .collect(),
            agent_lineage: Vec::new(),
        }
    }

    #[test]
    fn scorer_reports_pass_when_all_constraints_match() {
        let task: EvalTask = serde_yaml::from_str(
            r#"
task_id: read-edit
prompt: do it
expected_outcome:
  tool_pattern:
    - Read
    - Edit
  max_tool_calls: 2
  file_changes:
    - path: out.txt
      contains: "updated"
  max_turns: 1
  output_contains:
    - success
"#,
        )
        .expect("yaml should deserialize");
        let dir = tempdir().expect("tempdir should create");
        fs::write(dir.path().join("out.txt"), "updated content").expect("file should write");
        let trace = trace_with_tools(&["Read", "Edit"], "success", 1);

        let score = TaskScorer::score(&task, &trace, dir.path()).expect("score should succeed");
        assert_eq!(score.status, EvalStatus::Pass);
        assert_eq!(score.score, 1.0);
    }

    #[test]
    fn scorer_fails_when_file_change_is_missing() {
        let task: EvalTask = serde_yaml::from_str(
            r#"
task_id: edit-file
prompt: do it
expected_outcome:
  file_changes:
    - path: out.txt
      contains: "updated"
"#,
        )
        .expect("yaml should deserialize");
        let dir = tempdir().expect("tempdir should create");
        let trace = trace_with_tools(&[], "", 1);

        let score = TaskScorer::score(&task, &trace, dir.path()).expect("score should succeed");
        assert_eq!(score.status, EvalStatus::Fail);
        assert_eq!(score.score, 0.0);
    }

    #[test]
    fn scorer_returns_partial_when_tool_budget_is_exceeded() {
        let task: EvalTask = serde_yaml::from_str(
            r#"
task_id: read-once
prompt: do it
expected_outcome:
  tool_pattern:
    - Read
  max_tool_calls: 1
"#,
        )
        .expect("yaml should deserialize");
        let dir = tempdir().expect("tempdir should create");
        let trace = trace_with_tools(&["Read", "Edit"], "", 1);

        let score = TaskScorer::score(&task, &trace, dir.path()).expect("score should succeed");
        assert_eq!(score.status, EvalStatus::Partial);
        assert!(score.score > 0.0 && score.score < 1.0);
    }

    #[test]
    fn scorer_zeros_turn_budget_when_turn_limit_is_exceeded() {
        let task: EvalTask = serde_yaml::from_str(
            r#"
task_id: single-turn
prompt: do it
expected_outcome:
  max_turns: 1
"#,
        )
        .expect("yaml should deserialize");
        let dir = tempdir().expect("tempdir should create");
        let trace = trace_with_tools(&[], "", 2);

        let score = TaskScorer::score(&task, &trace, dir.path()).expect("score should succeed");
        let turn_budget = score
            .dimensions
            .iter()
            .find(|item| item.name == "turn_budget")
            .expect("turn budget dimension should exist");
        assert_eq!(turn_budget.score, 0.0);
    }
}
