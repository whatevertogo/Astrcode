//! # MCP 服务器审批管理
//!
//! 项目级 MCP 服务器的审批状态管理。
//! 通过 `McpSettingsStore` trait 读写审批数据，具体实现由 runtime 注入。

use log::{info, warn};

use super::settings_port::{McpApprovalData, McpApprovalStatus, McpSettingsStore};

/// MCP 审批管理器。
///
/// 管理项目级 MCP 服务器的审批流程：
/// 首次连接前需用户审批，审批状态通过 `McpSettingsStore` 持久化。
pub struct McpApprovalManager {
    /// settings 持久化接口。
    store: Box<dyn McpSettingsStore>,
}

impl McpApprovalManager {
    /// 创建审批管理器，注入 settings 存储实现。
    pub fn new(store: Box<dyn McpSettingsStore>) -> Self {
        Self { store }
    }

    /// 检查服务器是否已批准连接。
    ///
    /// 返回 true 表示已批准或不在审批范围内。
    pub fn is_approved(&self, project_path: &str, server_signature: &str) -> bool {
        match self.store.load_approvals(project_path) {
            Ok(approvals) => approvals
                .iter()
                .find(|a| a.server_signature == server_signature)
                .map(|a| a.status == McpApprovalStatus::Approved)
                .unwrap_or(false),
            Err(e) => {
                warn!("MCP approval load error: {}", e);
                false
            },
        }
    }

    /// 获取服务器审批状态。
    pub fn get_status(&self, project_path: &str, server_signature: &str) -> McpApprovalStatus {
        match self.store.load_approvals(project_path) {
            Ok(approvals) => approvals
                .iter()
                .find(|a| a.server_signature == server_signature)
                .map(|a| a.status)
                .unwrap_or(McpApprovalStatus::Pending),
            Err(_) => McpApprovalStatus::Pending,
        }
    }

    /// 批准服务器连接。
    pub fn approve(
        &self,
        project_path: &str,
        server_signature: &str,
        approved_by: &str,
    ) -> Result<(), String> {
        info!(
            "MCP server '{}' approved by {}",
            server_signature, approved_by
        );

        let data = McpApprovalData {
            server_signature: server_signature.to_string(),
            status: McpApprovalStatus::Approved,
            approved_at: Some(chrono_now_iso()),
            approved_by: Some(approved_by.to_string()),
        };

        self.store.save_approval(project_path, &data)
    }

    /// 拒绝服务器连接。
    pub fn reject(&self, project_path: &str, server_signature: &str) -> Result<(), String> {
        info!("MCP server '{}' rejected", server_signature);

        let data = McpApprovalData {
            server_signature: server_signature.to_string(),
            status: McpApprovalStatus::Rejected,
            approved_at: None,
            approved_by: None,
        };

        self.store.save_approval(project_path, &data)
    }

    /// 获取项目中所有等待审批的服务器签名。
    pub fn pending_servers(&self, project_path: &str) -> Vec<String> {
        match self.store.load_approvals(project_path) {
            Ok(approvals) => approvals
                .into_iter()
                .filter(|a| a.status == McpApprovalStatus::Pending)
                .map(|a| a.server_signature)
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    /// 批准项目中所有已知的服务器。
    ///
    /// 遍历所有已记录的服务器，将 Pending/Rejected 状态改为 Approved。
    pub fn approve_all(&self, project_path: &str, approved_by: &str) -> Result<usize, String> {
        let approvals = self.store.load_approvals(project_path)?;
        let mut count = 0;
        for data in &approvals {
            if data.status != McpApprovalStatus::Approved {
                self.approve(project_path, &data.server_signature, approved_by)?;
                count += 1;
            }
        }
        if count > 0 {
            info!(
                "MCP approved {} server(s) for project '{}'",
                count, project_path
            );
        }
        Ok(count)
    }

    /// 获取项目中所有已知服务器的审批状态。
    ///
    /// 返回所有已持久化的审批记录，供 API 端点查询。
    pub fn all_server_statuses(&self, project_path: &str) -> Vec<McpApprovalData> {
        self.store.load_approvals(project_path).unwrap_or_default()
    }
}

/// 生成当前时间的 ISO 8601 字符串（不依赖 chrono）。
fn chrono_now_iso() -> String {
    // 使用 std::time 而非 chrono，避免额外依赖
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("t={}", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// 测试用内存 settings 存储。
    struct MockSettingsStore {
        approvals: Mutex<Vec<McpApprovalData>>,
    }

    impl MockSettingsStore {
        fn new() -> Self {
            Self {
                approvals: Mutex::new(Vec::new()),
            }
        }
    }

    impl McpSettingsStore for MockSettingsStore {
        fn load_approvals(
            &self,
            _project_path: &str,
        ) -> std::result::Result<Vec<McpApprovalData>, String> {
            Ok(self.approvals.lock().unwrap().clone())
        }

        fn save_approval(
            &self,
            _project_path: &str,
            data: &McpApprovalData,
        ) -> std::result::Result<(), String> {
            let mut approvals = self.approvals.lock().unwrap();
            // 更新或新增
            if let Some(existing) = approvals
                .iter_mut()
                .find(|a| a.server_signature == data.server_signature)
            {
                *existing = data.clone();
            } else {
                approvals.push(data.clone());
            }
            Ok(())
        }
    }

    #[test]
    fn test_initial_status_pending() {
        let store = MockSettingsStore::new();
        let manager = McpApprovalManager::new(Box::new(store));
        let status = manager.get_status("/project", "stdio:npx:server");
        assert_eq!(status, McpApprovalStatus::Pending);
    }

    #[test]
    fn test_approve_and_check() {
        let store = MockSettingsStore::new();
        let manager = McpApprovalManager::new(Box::new(store));

        assert!(!manager.is_approved("/project", "stdio:npx:server"));

        manager
            .approve("/project", "stdio:npx:server", "user")
            .unwrap();

        assert!(manager.is_approved("/project", "stdio:npx:server"));
        assert_eq!(
            manager.get_status("/project", "stdio:npx:server"),
            McpApprovalStatus::Approved
        );
    }

    #[test]
    fn test_reject_and_check() {
        let store = MockSettingsStore::new();
        let manager = McpApprovalManager::new(Box::new(store));

        manager.reject("/project", "stdio:npx:server").unwrap();

        assert!(!manager.is_approved("/project", "stdio:npx:server"));
        assert_eq!(
            manager.get_status("/project", "stdio:npx:server"),
            McpApprovalStatus::Rejected
        );
    }

    #[test]
    fn test_pending_servers() {
        let store = MockSettingsStore::new();

        // 预先添加一个 pending 状态的服务器
        store.approvals.lock().unwrap().push(McpApprovalData {
            server_signature: "stdio:npx:server-a".to_string(),
            status: McpApprovalStatus::Pending,
            approved_at: None,
            approved_by: None,
        });

        let manager = McpApprovalManager::new(Box::new(store));

        let pending = manager.pending_servers("/project");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], "stdio:npx:server-a");
    }

    #[test]
    fn test_approve_overrides_reject() {
        let store = MockSettingsStore::new();
        let manager = McpApprovalManager::new(Box::new(store));

        manager.reject("/project", "stdio:npx:server").unwrap();
        assert!(!manager.is_approved("/project", "stdio:npx:server"));

        manager
            .approve("/project", "stdio:npx:server", "admin")
            .unwrap();
        assert!(manager.is_approved("/project", "stdio:npx:server"));
    }

    #[test]
    fn test_approve_all() {
        let store = MockSettingsStore::new();
        store.approvals.lock().unwrap().push(McpApprovalData {
            server_signature: "stdio:npx:server-a".to_string(),
            status: McpApprovalStatus::Pending,
            approved_at: None,
            approved_by: None,
        });
        store.approvals.lock().unwrap().push(McpApprovalData {
            server_signature: "stdio:npx:server-b".to_string(),
            status: McpApprovalStatus::Rejected,
            approved_at: None,
            approved_by: None,
        });

        let manager = McpApprovalManager::new(Box::new(store));
        let count = manager.approve_all("/project", "admin").unwrap();
        assert_eq!(count, 2);

        assert!(manager.is_approved("/project", "stdio:npx:server-a"));
        assert!(manager.is_approved("/project", "stdio:npx:server-b"));
    }

    #[test]
    fn test_all_server_statuses() {
        let store = MockSettingsStore::new();
        store.approvals.lock().unwrap().push(McpApprovalData {
            server_signature: "stdio:npx:server-a".to_string(),
            status: McpApprovalStatus::Approved,
            approved_at: Some("t=123".to_string()),
            approved_by: Some("user".to_string()),
        });
        store.approvals.lock().unwrap().push(McpApprovalData {
            server_signature: "stdio:npx:server-b".to_string(),
            status: McpApprovalStatus::Pending,
            approved_at: None,
            approved_by: None,
        });

        let manager = McpApprovalManager::new(Box::new(store));
        let statuses = manager.all_server_statuses("/project");
        assert_eq!(statuses.len(), 2);
    }
}
