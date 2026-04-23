//! MCP 审批设置的文件系统存储。
//!
//! 将 `adapter-mcp` 的审批持久化端口落到本地 JSON 文件，
//! 避免 server/bootstrap 再自带一份临时内存实现。

use std::{
    fs,
    path::{Path, PathBuf},
};

use astrcode_core::{McpApprovalData, McpSettingsStore};
use astrcode_support::hostpaths::resolve_home_dir;
use serde::{Deserialize, Serialize};

/// 基于 JSON 文件的 MCP 审批设置存储。
#[derive(Debug, Clone)]
pub struct FileMcpSettingsStore {
    path: PathBuf,
}

impl FileMcpSettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// 默认审批文件位置：`~/.astrcode/mcp-approvals.json`。
    pub fn default_path() -> astrcode_core::Result<Self> {
        let home = resolve_home_dir()?;
        Ok(Self::new(home.join(".astrcode").join("mcp-approvals.json")))
    }

    fn ensure_parent(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create MCP settings dir '{}': {error}",
                    parent.display()
                )
            })?;
        }
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<StoredApprovalRecord>, String> {
        if !self.path.is_file() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.path)
            .map_err(|error| format!("failed to read '{}': {error}", self.path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|error| format!("failed to parse '{}': {error}", self.path.display()))
    }

    fn save_all(&self, approvals: &[StoredApprovalRecord]) -> Result<(), String> {
        self.ensure_parent()?;
        let json = serde_json::to_vec_pretty(approvals)
            .map_err(|error| format!("failed to serialize MCP approvals: {error}"))?;
        write_atomic(&self.path, &json)
    }
}

impl McpSettingsStore for FileMcpSettingsStore {
    fn load_approvals(&self, project_path: &str) -> Result<Vec<McpApprovalData>, String> {
        Ok(self
            .load_all()?
            .into_iter()
            .filter(|record| record.project_path == project_path)
            .map(|record| record.approval)
            .collect())
    }

    fn save_approval(&self, project_path: &str, data: &McpApprovalData) -> Result<(), String> {
        let mut approvals = self.load_all()?;
        if let Some(existing) = approvals.iter_mut().find(|record| {
            record.project_path == project_path
                && record.approval.server_signature == data.server_signature
        }) {
            existing.approval = data.clone();
        } else {
            approvals.push(StoredApprovalRecord {
                project_path: project_path.to_string(),
                approval: data.clone(),
            });
        }
        self.save_all(&approvals)
    }

    fn clear_approvals(&self, project_path: &str) -> Result<(), String> {
        let approvals = self
            .load_all()?
            .into_iter()
            .filter(|record| record.project_path != project_path)
            .collect::<Vec<_>>();
        self.save_all(&approvals)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredApprovalRecord {
    project_path: String,
    approval: McpApprovalData,
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, bytes).map_err(|error| {
        format!(
            "failed to write temp file '{}': {error}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        format!("failed to replace '{}': {error}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use astrcode_core::McpApprovalStatus;

    use super::*;

    #[test]
    fn approvals_are_isolated_by_project_path() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let store = FileMcpSettingsStore::new(temp_dir.path().join("mcp-approvals.json"));

        store
            .save_approval(
                "/project-a",
                &McpApprovalData {
                    server_signature: "stdio:server-a".to_string(),
                    status: McpApprovalStatus::Approved,
                    approved_at: None,
                    approved_by: None,
                },
            )
            .expect("approval should be saved");
        store
            .save_approval(
                "/project-b",
                &McpApprovalData {
                    server_signature: "stdio:server-b".to_string(),
                    status: McpApprovalStatus::Rejected,
                    approved_at: None,
                    approved_by: None,
                },
            )
            .expect("approval should be saved");

        let project_a = store
            .load_approvals("/project-a")
            .expect("project a approvals should load");
        let project_b = store
            .load_approvals("/project-b")
            .expect("project b approvals should load");

        assert_eq!(project_a.len(), 1);
        assert_eq!(project_b.len(), 1);
        assert_eq!(project_a[0].server_signature, "stdio:server-a");
        assert_eq!(project_b[0].server_signature, "stdio:server-b");
    }
}
