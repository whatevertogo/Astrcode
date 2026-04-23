use std::{fs, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    EvalError, EvalResult,
    diagnosis::DiagnosisReport,
    task::{EvalScore, EvalStatus},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvalTaskResultStatus {
    Pass,
    Partial,
    Fail,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct EvalTaskMetrics {
    pub tool_calls: usize,
    pub duration_ms: u64,
    pub estimated_tokens: u64,
    pub turn_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalTaskResult {
    pub task_id: String,
    pub status: EvalTaskResultStatus,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<DiagnosisReport>,
    pub metrics: EvalTaskMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalBaselineTaskDiff {
    pub task_id: String,
    pub score_delta: f64,
    pub tool_calls_delta: i64,
    pub duration_ms_delta: i64,
    pub estimated_tokens_delta: i64,
    pub regression: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalBaselineComparison {
    pub baseline_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diffs: Vec<EvalBaselineTaskDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReportSummary {
    pub total_tasks: usize,
    pub pass_rate: f64,
    pub avg_score: f64,
    pub avg_tool_calls: f64,
    pub avg_duration_ms: f64,
    pub avg_estimated_tokens: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(with = "astrcode_core::local_rfc3339")]
    pub timestamp: DateTime<Utc>,
    pub task_set: String,
    pub results: Vec<EvalTaskResult>,
    pub summary: EvalReportSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<EvalBaselineComparison>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub struct ReportWriter;

impl ReportWriter {
    pub fn build(task_set: String, results: Vec<EvalTaskResult>) -> EvalReport {
        let total_tasks = results.len();
        let pass_count = results
            .iter()
            .filter(|result| result.status == EvalTaskResultStatus::Pass)
            .count();
        let avg_score = average(results.iter().map(|result| result.score));
        let avg_tool_calls = average(
            results
                .iter()
                .map(|result| result.metrics.tool_calls as f64),
        );
        let avg_duration_ms = average(
            results
                .iter()
                .map(|result| result.metrics.duration_ms as f64),
        );
        let avg_estimated_tokens = average(
            results
                .iter()
                .map(|result| result.metrics.estimated_tokens as f64),
        );

        EvalReport {
            commit: current_commit_sha(),
            timestamp: Utc::now(),
            task_set,
            results,
            summary: EvalReportSummary {
                total_tasks,
                pass_rate: if total_tasks == 0 {
                    0.0
                } else {
                    pass_count as f64 / total_tasks as f64
                },
                avg_score,
                avg_tool_calls,
                avg_duration_ms,
                avg_estimated_tokens,
            },
            baseline: None,
            warnings: Vec::new(),
        }
    }

    pub fn attach_baseline(
        report: &mut EvalReport,
        baseline_path: &Path,
        regression_threshold: f64,
    ) -> EvalResult<()> {
        if !baseline_path.exists() {
            report.baseline = Some(EvalBaselineComparison {
                baseline_path: baseline_path.display().to_string(),
                diffs: Vec::new(),
                warnings: vec![format!(
                    "baseline 文件不存在，跳过对比: {}",
                    baseline_path.display()
                )],
            });
            return Ok(());
        }

        let baseline = Self::load(baseline_path)?;
        let mut diffs = Vec::new();
        for result in &report.results {
            let Some(previous) = baseline
                .results
                .iter()
                .find(|candidate| candidate.task_id == result.task_id)
            else {
                continue;
            };

            let score_delta = result.score - previous.score;
            diffs.push(EvalBaselineTaskDiff {
                task_id: result.task_id.clone(),
                score_delta,
                tool_calls_delta: result.metrics.tool_calls as i64
                    - previous.metrics.tool_calls as i64,
                duration_ms_delta: result.metrics.duration_ms as i64
                    - previous.metrics.duration_ms as i64,
                estimated_tokens_delta: result.metrics.estimated_tokens as i64
                    - previous.metrics.estimated_tokens as i64,
                regression: score_delta < -regression_threshold,
            });
        }

        report.baseline = Some(EvalBaselineComparison {
            baseline_path: baseline_path.display().to_string(),
            diffs,
            warnings: Vec::new(),
        });
        Ok(())
    }

    pub fn persist(report: &EvalReport, path: &Path) -> EvalResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                EvalError::io(format!("创建报告目录 {} 失败", parent.display()), error)
            })?;
        }
        let payload = serde_json::to_string_pretty(report)
            .map_err(|error| EvalError::validation(format!("序列化报告失败: {error}")))?;
        fs::write(path, payload)
            .map_err(|error| EvalError::io(format!("写入报告 {} 失败", path.display()), error))
    }

    pub fn load(path: &Path) -> EvalResult<EvalReport> {
        let content = fs::read_to_string(path)
            .map_err(|error| EvalError::io(format!("读取报告 {} 失败", path.display()), error))?;
        serde_json::from_str(&content)
            .map_err(|error| EvalError::validation(format!("解析报告失败: {error}")))
    }
}

pub fn status_from_score(score: &EvalScore) -> EvalTaskResultStatus {
    match score.status {
        EvalStatus::Pass => EvalTaskResultStatus::Pass,
        EvalStatus::Partial => EvalTaskResultStatus::Partial,
        EvalStatus::Fail => EvalTaskResultStatus::Fail,
    }
}

fn average(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn current_commit_sha() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim();
    if sha.is_empty() {
        None
    } else {
        Some(sha.to_string())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{EvalTaskMetrics, EvalTaskResult, EvalTaskResultStatus, ReportWriter};

    #[test]
    fn report_writer_persists_and_loads_report() {
        let report = ReportWriter::build(
            "core".to_string(),
            vec![EvalTaskResult {
                task_id: "task-1".to_string(),
                status: EvalTaskResultStatus::Pass,
                score: 1.0,
                diagnosis: None,
                metrics: EvalTaskMetrics {
                    tool_calls: 2,
                    duration_ms: 100,
                    estimated_tokens: 50,
                    turn_count: 1,
                },
                session_id: Some("session-1".to_string()),
                workspace_path: None,
                error: None,
            }],
        );
        let dir = tempdir().expect("tempdir should create");
        let path = dir.path().join("report.json");
        ReportWriter::persist(&report, &path).expect("report should persist");
        let loaded = ReportWriter::load(&path).expect("report should load");
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(loaded.summary.total_tasks, 1);
    }

    #[test]
    fn report_writer_builds_baseline_diff() {
        let dir = tempdir().expect("tempdir should create");
        let path = dir.path().join("baseline.json");
        let baseline = ReportWriter::build(
            "core".to_string(),
            vec![EvalTaskResult {
                task_id: "task-1".to_string(),
                status: EvalTaskResultStatus::Pass,
                score: 1.0,
                diagnosis: None,
                metrics: EvalTaskMetrics {
                    tool_calls: 2,
                    duration_ms: 100,
                    estimated_tokens: 50,
                    turn_count: 1,
                },
                session_id: None,
                workspace_path: None,
                error: None,
            }],
        );
        ReportWriter::persist(&baseline, &path).expect("baseline should persist");

        let mut current = ReportWriter::build(
            "core".to_string(),
            vec![EvalTaskResult {
                task_id: "task-1".to_string(),
                status: EvalTaskResultStatus::Partial,
                score: 0.7,
                diagnosis: None,
                metrics: EvalTaskMetrics {
                    tool_calls: 3,
                    duration_ms: 150,
                    estimated_tokens: 70,
                    turn_count: 1,
                },
                session_id: None,
                workspace_path: None,
                error: None,
            }],
        );
        ReportWriter::attach_baseline(&mut current, &path, 0.1).expect("baseline should attach");
        let diff = &current
            .baseline
            .as_ref()
            .expect("baseline should exist")
            .diffs[0];
        assert!(diff.regression);
        assert!(diff.score_delta < 0.0);
    }
}
