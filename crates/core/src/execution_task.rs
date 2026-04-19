//! 执行期 task 领域模型。
//!
//! 这套类型专门表达执行阶段的工作清单，与 canonical session plan 严格分层。

use std::fmt;

use serde::{Deserialize, Serialize};

/// durable tool result metadata 中使用的稳定 schema 名称。
pub const EXECUTION_TASK_SNAPSHOT_SCHEMA: &str = "executionTaskSnapshot";

/// 执行期 task 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTaskStatus {
    Pending,
    InProgress,
    Completed,
}

impl ExecutionTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress)
    }
}

impl fmt::Display for ExecutionTaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 单条执行期 task。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionTaskItem {
    /// 面向用户与面板展示的祈使句标题。
    pub content: String,
    /// 当前任务状态。
    pub status: ExecutionTaskStatus,
    /// 面向 prompt 注入与进行中展示的动词短语。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
}

impl ExecutionTaskItem {
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

/// 单个 owner 的最新 task 快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    pub owner: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ExecutionTaskItem>,
}

impl TaskSnapshot {
    pub fn has_active_items(&self) -> bool {
        self.items.iter().any(ExecutionTaskItem::is_active)
    }

    pub fn active_items(&self) -> Vec<ExecutionTaskItem> {
        self.items
            .iter()
            .filter(|item| item.is_active())
            .cloned()
            .collect()
    }

    pub fn should_clear(&self) -> bool {
        self.items.is_empty()
            || self
                .items
                .iter()
                .all(|item| item.status == ExecutionTaskStatus::Completed)
    }
}

/// `taskWrite` durable tool result metadata。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionTaskSnapshotMetadata {
    pub schema: String,
    pub owner: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ExecutionTaskItem>,
    pub cleared: bool,
}

impl ExecutionTaskSnapshotMetadata {
    pub fn from_snapshot(snapshot: &TaskSnapshot) -> Self {
        Self {
            schema: EXECUTION_TASK_SNAPSHOT_SCHEMA.to_string(),
            owner: snapshot.owner.clone(),
            items: snapshot.items.clone(),
            cleared: snapshot.should_clear(),
        }
    }

    pub fn into_snapshot(self) -> TaskSnapshot {
        TaskSnapshot {
            owner: self.owner,
            items: self.items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_snapshot_metadata_marks_completed_snapshots_as_cleared() {
        let metadata = ExecutionTaskSnapshotMetadata::from_snapshot(&TaskSnapshot {
            owner: "owner-1".to_string(),
            items: vec![ExecutionTaskItem {
                content: "收尾验证".to_string(),
                status: ExecutionTaskStatus::Completed,
                active_form: Some("正在收尾验证".to_string()),
            }],
        });

        assert_eq!(metadata.schema, EXECUTION_TASK_SNAPSHOT_SCHEMA);
        assert!(metadata.cleared);
    }
}
