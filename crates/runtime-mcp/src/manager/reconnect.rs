//! # MCP 自动重连逻辑
//!
//! 仅远程传输（HTTP/SSE）支持自动重连，stdio 不重连。
//! 指数退避：1s → 2s → 4s → 8s → 16s（上限 30s），最多 5 次。
//! 使用 tokio::spawn 持有 JoinHandle，支持外部取消。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
};

use log::{info, warn};
use tokio::{
    sync::{Mutex, broadcast},
    task::JoinHandle,
};

use super::{McpManagedConnection, create_transport, establish_connection_inner};
use crate::config::McpServerConfig;

/// MCP 重连管理器。
///
/// 管理活跃的重连任务，每个服务器最多一个并发重连。
pub(crate) struct McpReconnectManager {
    /// 活跃的重连任务（服务器名 → JoinHandle）。
    tasks: StdMutex<HashMap<String, JoinHandle<()>>>,
}

impl McpReconnectManager {
    /// 创建新的重连管理器。
    pub fn new() -> Self {
        Self {
            tasks: StdMutex::new(HashMap::new()),
        }
    }

    /// 启动重连循环。
    ///
    /// stdio 传输直接返回不重连。如果该服务器已有活跃的重连任务则跳过。
    pub fn start_reconnect(
        &self,
        server_name: String,
        config: McpServerConfig,
        connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
        surface_events: broadcast::Sender<()>,
    ) {
        // stdio 传输不重连
        if !config.transport.is_remote() {
            return;
        }

        // 已有活跃重连任务则跳过
        {
            let tasks = self.tasks.lock().unwrap();
            if let Some(handle) = tasks.get(&server_name) {
                if !handle.is_finished() {
                    warn!(
                        "MCP server '{}' already has an active reconnect task, skipping",
                        server_name
                    );
                    return;
                }
            }
        }

        let max_attempts = config.max_reconnect_attempts;
        let name_clone = server_name.clone();
        let handle = tokio::spawn(reconnect_loop(
            server_name,
            config,
            connections,
            max_attempts,
            surface_events,
        ));

        {
            let mut tasks = self.tasks.lock().unwrap();
            // 清理已完成的旧任务
            tasks.retain(|_, h| !h.is_finished());
            tasks.insert(name_clone, handle);
        }
    }

    /// 取消指定服务器的重连任务。
    pub fn cancel_reconnect(&self, server_name: &str) {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(handle) = tasks.remove(server_name) {
            handle.abort();
            info!("MCP reconnect task cancelled for '{}'", server_name);
        }
    }

    /// 取消所有重连任务。
    pub fn cancel_all(&self) {
        let mut tasks = self.tasks.lock().unwrap();
        let count = tasks.len();
        for (_, handle) in tasks.drain() {
            handle.abort();
        }
        if count > 0 {
            info!("MCP cancelled {} reconnect tasks", count);
        }
    }

    /// 指定服务器是否有活跃的重连任务。
    #[allow(dead_code)]
    pub fn is_reconnecting(&self, server_name: &str) -> bool {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .get(server_name)
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    /// 清理已完成的重连任务。
    #[allow(dead_code)]
    pub fn cleanup_finished(&self) {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|_, h| !h.is_finished());
    }
}

/// 重连循环。
///
/// 指数退避重试，每次尝试前检查连接是否已禁用或已移除。
async fn reconnect_loop(
    server_name: String,
    config: McpServerConfig,
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
    max_attempts: u32,
    surface_events: broadcast::Sender<()>,
) {
    info!(
        "MCP starting reconnect loop for '{}' (max {} attempts)",
        server_name, max_attempts
    );

    for attempt in 0..max_attempts {
        let delay = calculate_backoff(attempt);
        info!(
            "MCP reconnect '{}' attempt {}/{}: waiting {}s",
            server_name,
            attempt + 1,
            max_attempts,
            delay.as_secs()
        );
        tokio::time::sleep(delay).await;

        // 每次尝试前检查连接状态
        {
            let conns = connections.lock().await;
            if let Some(managed) = conns.get(&server_name) {
                // 已禁用则终止重连
                if managed.connection.is_disabled() {
                    info!(
                        "MCP reconnect '{}' cancelled: server is disabled",
                        server_name
                    );
                    return;
                }
            } else {
                // 连接已从管理器移除（可能是 disconnect_one）
                info!(
                    "MCP reconnect '{}' cancelled: server removed from manager",
                    server_name
                );
                return;
            }
        }

        // 创建新传输
        let transport = match create_transport(&config.transport) {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    "MCP reconnect '{}' attempt {}: transport creation failed: {}",
                    server_name,
                    attempt + 1,
                    e
                );
                continue;
            },
        };

        // 启动传输
        {
            let mut locked = transport.lock().await;
            if let Err(e) = locked.start().await {
                warn!(
                    "MCP reconnect '{}' attempt {}: transport start failed: {}",
                    server_name,
                    attempt + 1,
                    e
                );
                continue;
            }
        }

        // 重新建立连接（握手 + 工具发现 + 桥接注册）
        match establish_connection_inner(
            config.clone(),
            transport,
            connections.clone(),
            surface_events.clone(),
        )
        .await
        {
            Ok(_invokers) => {
                info!(
                    "MCP server '{}' reconnected successfully on attempt {}",
                    server_name,
                    attempt + 1
                );
                let _ = surface_events.send(());
                return;
            },
            Err(e) => {
                warn!(
                    "MCP reconnect '{}' attempt {} failed: {}",
                    server_name,
                    attempt + 1,
                    e
                );
                // 更新连接状态中的重连计数
                let mut conns = connections.lock().await;
                if let Some(managed) = conns.get_mut(&server_name) {
                    managed.connection.prepare_reconnect();
                }
            },
        }
    }

    warn!(
        "MCP server '{}' exhausted all {} reconnect attempts",
        server_name, max_attempts
    );
}

/// 计算指数退避延迟。
///
/// 1s → 2s → 4s → 8s → 16s，上限 30s。
fn calculate_backoff(attempt: u32) -> std::time::Duration {
    let secs = 2u64.pow(attempt);
    std::time::Duration::from_secs(secs.min(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_backoff() {
        assert_eq!(calculate_backoff(0), std::time::Duration::from_secs(1));
        assert_eq!(calculate_backoff(1), std::time::Duration::from_secs(2));
        assert_eq!(calculate_backoff(2), std::time::Duration::from_secs(4));
        assert_eq!(calculate_backoff(3), std::time::Duration::from_secs(8));
        assert_eq!(calculate_backoff(4), std::time::Duration::from_secs(16));
        // 超过 16s 后上限为 30s
        assert_eq!(calculate_backoff(5), std::time::Duration::from_secs(30));
        assert_eq!(calculate_backoff(10), std::time::Duration::from_secs(30));
    }

    #[test]
    fn test_new_manager() {
        let manager = McpReconnectManager::new();
        assert!(!manager.is_reconnecting("nonexistent"));
    }

    #[test]
    fn test_cancel_nonexistent() {
        let manager = McpReconnectManager::new();
        // 取消不存在的任务不应 panic
        manager.cancel_reconnect("nonexistent");
        manager.cancel_all();
    }

    #[test]
    fn test_cleanup_finished() {
        let manager = McpReconnectManager::new();
        manager.cleanup_finished();
    }

    use std::collections::HashMap;

    use crate::config::{McpConfigScope, McpTransportConfig};

    fn stdio_config(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransportConfig::Stdio {
                command: "test".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        }
    }

    fn http_config(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransportConfig::StreamableHttp {
                url: "http://localhost:1/mcp".to_string(),
                headers: HashMap::new(),
                oauth: None,
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        }
    }

    #[test]
    fn test_stdio_does_not_reconnect() {
        let manager = McpReconnectManager::new();
        let connections = Arc::new(Mutex::new(HashMap::new()));
        let config = stdio_config("stdio-server");

        // stdio 不应启动重连任务
        let (surface_events, _) = broadcast::channel(4);
        manager.start_reconnect(
            "stdio-server".to_string(),
            config,
            connections,
            surface_events,
        );
        assert!(!manager.is_reconnecting("stdio-server"));
    }

    #[tokio::test]
    async fn test_start_reconnect_creates_task() {
        let manager = McpReconnectManager::new();
        let connections: Arc<Mutex<HashMap<String, McpManagedConnection>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let config = http_config("remote-server");
        let (surface_events, _) = broadcast::channel(4);

        manager.start_reconnect(
            "remote-server".to_string(),
            config,
            connections,
            surface_events,
        );

        // 应该有活跃的重连任务
        assert!(manager.is_reconnecting("remote-server"));

        // 取消
        manager.cancel_reconnect("remote-server");
        // 给 abort 一点时间
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!manager.is_reconnecting("remote-server"));
    }

    #[tokio::test]
    async fn test_cancel_all() {
        let manager = McpReconnectManager::new();
        let connections: Arc<Mutex<HashMap<String, McpManagedConnection>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (surface_events, _) = broadcast::channel(4);

        for i in 0..3 {
            let config = http_config(&format!("server-{}", i));
            let name = format!("server-{}", i);
            manager.start_reconnect(name, config, connections.clone(), surface_events.clone());
        }

        assert!(manager.is_reconnecting("server-0"));
        assert!(manager.is_reconnecting("server-1"));
        assert!(manager.is_reconnecting("server-2"));

        manager.cancel_all();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(!manager.is_reconnecting("server-0"));
        assert!(!manager.is_reconnecting("server-1"));
        assert!(!manager.is_reconnecting("server-2"));
    }
}
