//! # MCP 连接管理
//!
//! 负责所有 MCP 服务器的连接生命周期管理。
//! McpConnectionManager 批量连接服务器、管理工具桥接、追踪进行中调用。

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use astrcode_core::{AstrError, CapabilityInvoker, Result};
use astrcode_prompt_contract::PromptDeclaration;
use astrcode_runtime_contract::ManagedRuntimeComponent;
use async_trait::async_trait;
use connection::McpConnection;
use futures_util::stream::{self, StreamExt};
use log::{info, warn};
use tokio::sync::{Mutex, broadcast};

use crate::{
    bridge::resource_tool::{ListMcpResourcesTool, ReadMcpResourceTool},
    config::{McpServerConfig, McpTransportConfig},
    protocol::{McpClient, types::*},
    transport::{
        McpTransport, http::StreamableHttpTransport, sse::SseTransport, stdio::StdioTransport,
    },
};

pub mod connection;
pub mod hot_reload;
pub mod reconnect;
pub mod surface;

pub use connection::{McpConnection as McpConnectionExport, McpConnectionState};
pub use surface::{McpIndexedResource, McpServerStatusSnapshot, McpSurfaceSnapshot};

/// MCP reload 的最小回滚点。
///
/// 为什么只保存声明配置而不克隆活跃连接：
/// - 活跃连接包含 transport/client 等运行时句柄，无法也不应直接克隆
/// - 对 MCP 来说，声明配置才是唯一事实源；恢复时重新执行一次 reload 即可重建连接集合
#[derive(Debug, Clone, Default)]
pub struct McpReloadSnapshot {
    declared_configs: Vec<McpServerConfig>,
}

/// 单个服务器的完整管理信息。
pub(crate) struct McpManagedConnection {
    /// 连接状态机。
    pub(crate) connection: McpConnection,
    /// 传输层引用（用于 shutdown 时直接关闭）。
    pub(crate) transport: Arc<Mutex<dyn McpTransport>>,
    /// MCP 协议客户端（共享给工具桥接）。
    pub(crate) client: Arc<Mutex<McpClient>>,
    /// 已注册的能力调用器（MCP 工具桥接）。
    pub(crate) invokers: Vec<Arc<dyn CapabilityInvoker>>,
    /// 服务器最新工具列表快照。
    pub(crate) tools: Vec<McpToolInfo>,
    /// 服务器最新 prompt 模板列表快照。
    pub(crate) prompts: Vec<McpPromptInfo>,
    /// 服务器最新资源索引快照。
    pub(crate) resources: Vec<McpResourceInfo>,
    /// 服务器注入的 prompt 声明（当前仅包含 instructions）。
    pub(crate) prompt_declarations: Vec<PromptDeclaration>,
    /// 原始配置（用于重连）。
    pub(crate) config: McpServerConfig,
}

/// 批量连接结果。
pub struct McpConnectionResults {
    /// 成功连接的服务器名列表。
    pub connected: Vec<String>,
    /// 连接失败的服务器（名称, 错误信息）。
    pub failed: Vec<(String, String)>,
    /// 所有注册的能力调用器。
    pub invokers: Vec<Arc<dyn CapabilityInvoker>>,
    /// MCP 服务器的 prompt 声明。
    pub prompt_declarations: Vec<PromptDeclaration>,
}

/// 批量连接中单个服务器的结果。
type BatchResult = std::result::Result<(String, Vec<Arc<dyn CapabilityInvoker>>), (String, String)>;

/// MCP 连接管理器。
///
/// 负责批量连接所有已声明的 MCP 服务器，
/// 管理工具桥接和连接生命周期。
pub struct McpConnectionManager {
    /// 活跃连接（服务器名 → 管理连接信息）。
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
    /// 进行中的工具调用计数器。
    in_flight_count: Arc<AtomicUsize>,
    /// 最近一次声明的配置快照（含未连接/待审批/禁用项）。
    declared_configs: Arc<Mutex<HashMap<String, McpServerConfig>>>,
    /// 重连管理器。
    reconnect_manager: reconnect::McpReconnectManager,
    /// surface/status 变化通知。
    surface_events: broadcast::Sender<()>,
    /// 审批管理器（可选，runtime 在 bootstrap 时注入）。
    approval_manager: Option<std::sync::Mutex<crate::config::McpApprovalManager>>,
    /// 当前项目路径（用于审批查询）。
    project_path: Option<String>,
}

impl std::fmt::Debug for McpConnectionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConnectionManager")
            .finish_non_exhaustive()
    }
}

impl Default for McpConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl McpConnectionManager {
    /// 创建新的连接管理器。
    pub fn new() -> Self {
        let (surface_events, _) = broadcast::channel(32);
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            in_flight_count: Arc::new(AtomicUsize::new(0)),
            declared_configs: Arc::new(Mutex::new(HashMap::new())),
            reconnect_manager: reconnect::McpReconnectManager::new(),
            surface_events,
            approval_manager: None,
            project_path: None,
        }
    }

    /// 注入审批管理器（builder 模式，由 runtime bootstrap 调用）。
    pub fn with_approval(
        mut self,
        manager: crate::config::McpApprovalManager,
        project_path: String,
    ) -> Self {
        self.approval_manager = Some(std::sync::Mutex::new(manager));
        self.project_path = Some(project_path);
        self
    }

    /// 订阅 MCP surface/status 变化事件。
    pub fn subscribe_surface_events(&self) -> broadcast::Receiver<()> {
        self.surface_events.subscribe()
    }

    /// 返回当前 MCP surface 快照。
    pub async fn current_surface(&self) -> McpSurfaceSnapshot {
        let declared_configs = {
            let configs = self.declared_configs.lock().await;
            configs.values().cloned().collect::<Vec<_>>()
        };
        let connections = self.connections.lock().await;
        let mut capability_invokers = Vec::new();
        let mut prompt_declarations = Vec::new();
        let mut resource_index = Vec::new();
        let mut server_statuses = Vec::new();
        let mut has_connected_server = false;
        let mut has_resource_server = false;

        for config in &declared_configs {
            let pending_approval = config.scope == crate::config::McpConfigScope::Project
                && config.enabled
                && !self.check_approval(config);
            let connection = connections.get(&config.name);
            server_statuses.push(surface::build_server_status(
                config,
                connection,
                pending_approval,
            ));

            let Some(connection) = connection else {
                continue;
            };
            if !connection.connection.is_connected() {
                continue;
            }
            has_connected_server = true;
            has_resource_server |= !connection.resources.is_empty();

            capability_invokers.extend(connection.invokers.clone());
            prompt_declarations.extend(connection.prompt_declarations.clone());
            resource_index.extend(connection.resources.iter().cloned().map(|resource| {
                surface::McpIndexedResource {
                    server_name: config.name.clone(),
                    uri: resource.uri,
                    name: resource.name,
                    description: resource.description,
                    mime_type: resource.mime_type,
                }
            }));
        }

        if has_connected_server && has_resource_server {
            capability_invokers.push(Arc::new(ListMcpResourcesTool::new(
                self.connections.clone(),
            )));
            capability_invokers.push(Arc::new(ReadMcpResourceTool::new(self.connections.clone())));
        }

        McpSurfaceSnapshot {
            capability_invokers,
            prompt_declarations,
            server_statuses,
            resource_index,
        }
    }

    /// 列出所有已声明 MCP 服务器状态。
    pub async fn list_status(&self) -> Vec<McpServerStatusSnapshot> {
        self.current_surface().await.server_statuses
    }

    /// 批量连接所有已声明的 MCP 服务器。
    ///
    /// 本地（stdio）并发度 ≤ 3，远程（HTTP/SSE）并发度 ≤ 10。
    /// 单个服务器连接失败不阻塞其他服务器。
    pub async fn connect_all(&self, configs: Vec<McpServerConfig>) -> McpConnectionResults {
        self.replace_declared_configs(&configs).await;
        // 审批过滤：Project scope 的服务器需要审批
        let mut local_configs = Vec::new();
        let mut remote_configs = Vec::new();

        for config in configs {
            // 项目级服务器需要审批
            if config.scope == crate::config::McpConfigScope::Project
                && !self.check_approval(&config)
            {
                let name = config.name.clone();
                info!("MCP server '{}' skipped: pending project approval", name);
                continue;
            }
            if !config.enabled {
                info!("MCP server '{}' skipped: disabled", config.name);
                continue;
            }
            if config.transport.is_remote() {
                remote_configs.push(config);
            } else {
                local_configs.push(config);
            }
        }

        // 本地和远程并发连接
        let connections = self.connections.clone();
        let local_fut = Self::run_batch(
            connections.clone(),
            local_configs,
            MAX_LOCAL_CONCURRENCY,
            self.surface_events.clone(),
        );
        let remote_fut = Self::run_batch(
            connections,
            remote_configs,
            MAX_REMOTE_CONCURRENCY,
            self.surface_events.clone(),
        );

        let (local_results, remote_results) = tokio::join!(local_fut, remote_fut);

        // 合并结果
        let mut results = McpConnectionResults {
            connected: Vec::new(),
            failed: Vec::new(),
            invokers: Vec::new(),
            prompt_declarations: Vec::new(),
        };

        for r in local_results.into_iter().chain(remote_results) {
            match r {
                Ok((name, invokers)) => {
                    results.connected.push(name);
                    results.invokers.extend(invokers);
                },
                Err((name, error)) => {
                    results.failed.push((name, error));
                },
            }
        }

        info!(
            "MCP connection batch complete: {} connected, {} failed",
            results.connected.len(),
            results.failed.len()
        );

        let surface = self.current_surface().await;
        results.invokers = surface.capability_invokers;
        results.prompt_declarations = surface.prompt_declarations;
        self.notify_surface_changed();

        results
    }
    /// 创建传输层、执行握手、获取工具列表并注册桥接。
    /// 用于初始连接和热加载新增服务器。
    pub async fn connect_one(
        &self,
        config: McpServerConfig,
    ) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
        self.upsert_declared_config(config.clone()).await;
        let transport = create_transport(&config.transport)?;

        // 启动传输
        {
            let mut locked = transport.lock().await;
            locked.start().await?;
        }

        let invokers = Self::establish_connection(
            config,
            transport,
            self.connections.clone(),
            self.surface_events.clone(),
        )
        .await?;
        self.notify_surface_changed();
        Ok(invokers)
    }

    /// 当前进行中的工具调用数量。
    pub fn in_flight_count(&self) -> usize {
        self.in_flight_count.load(Ordering::Relaxed)
    }

    /// 工具调用开始时调用（计数器 +1）。
    pub fn begin_call(&self) {
        self.in_flight_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 工具调用结束时调用（计数器 -1，含错误）。
    pub fn end_call(&self) {
        self.in_flight_count.fetch_sub(1, Ordering::Relaxed);
    }

    /// 等待所有进行中的调用完成，最长等待 timeout_secs 秒。
    pub async fn wait_idle(&self, timeout_secs: u64) -> Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
            while self.in_flight_count.load(Ordering::Relaxed) > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| AstrError::Network("wait for in-flight MCP calls timed out".into()))
    }

    /// 断开指定服务器的连接。
    ///
    /// 先取消重连任务，等待进行中调用完成（30s 超时），
    /// 然后关闭传输层并从管理器中移除。
    pub async fn disconnect_one(&self, server_name: &str) -> Result<()> {
        // 取消重连
        self.reconnect_manager.cancel_reconnect(server_name);

        // 等待进行中调用完成
        self.wait_idle(30).await?;

        // 关闭传输并移除
        let removed = {
            let mut conns = self.connections.lock().await;
            conns.remove(server_name)
        };

        if let Some(managed) = removed {
            let mut transport = managed.transport.lock().await;
            transport.close().await?;
            info!("MCP server '{}' disconnected and removed", server_name);
            self.notify_surface_changed();
            Ok(())
        } else {
            warn!("MCP server '{}' not found for disconnect", server_name);
            Err(AstrError::Internal(format!(
                "MCP server '{}' not found",
                server_name
            )))
        }
    }

    /// 发送取消通知并设置强制断开计时器。
    ///
    /// 发送 `notifications/cancelled` 后，设置 30 秒计时器，
    /// 超时后强制断开连接。
    pub async fn cancel_and_force_disconnect(
        &self,
        server_name: &str,
        request_id: &str,
    ) -> Result<()> {
        // 发送取消通知
        {
            let conns = self.connections.lock().await;
            if let Some(managed) = conns.get(server_name) {
                let client = managed.client.lock().await;
                if let Err(e) = client
                    .send_cancel(request_id, Some("force disconnect"))
                    .await
                {
                    warn!(
                        "MCP server '{}' cancel notification failed: {}",
                        server_name, e
                    );
                }
            }
        }

        // 设置强制断开计时器
        let connections = self.connections.clone();
        let server_name = server_name.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let mut conns = connections.lock().await;
            if let Some(managed) = conns.get_mut(&server_name) {
                if managed.connection.is_connected() {
                    warn!(
                        "MCP server '{}' force disconnect after cancel timeout",
                        server_name
                    );
                    managed
                        .connection
                        .mark_failed("force disconnect after cancel timeout");
                }
            }
        });

        Ok(())
    }

    /// 根据新配置热加载：新增连接、移除连接、未变化保持。
    ///
    /// 对比新旧配置差异：
    /// - 新增的服务器调用 `connect_one`
    /// - 移除的服务器调用 `disconnect_one`
    /// - 未变化的服务器保持不变
    ///
    /// 返回更新后的所有能力调用器列表。
    pub async fn reload_config(
        &self,
        new_configs: Vec<McpServerConfig>,
    ) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
        self.replace_declared_configs(&new_configs).await;
        let current_connections: HashMap<String, McpServerConfig> = {
            let conns = self.connections.lock().await;
            conns
                .iter()
                .map(|(name, managed)| (name.clone(), managed.config.clone()))
                .collect()
        };
        let new_names: Vec<String> = new_configs.iter().map(|c| c.name.clone()).collect();

        let to_remove: Vec<String> = current_connections
            .keys()
            .filter(|name| !new_names.contains(name))
            .cloned()
            .collect();
        let mut to_disconnect = Vec::new();
        let mut to_connect = Vec::new();

        for config in new_configs {
            let should_connect = config.enabled && self.check_approval(&config);
            match current_connections.get(&config.name) {
                Some(existing) if *existing == config => {
                    if !should_connect {
                        to_disconnect.push(config.name.clone());
                    }
                },
                Some(_) => {
                    to_disconnect.push(config.name.clone());
                    if should_connect {
                        to_connect.push(config);
                    }
                },
                None => {
                    if should_connect {
                        to_connect.push(config);
                    }
                },
            }
        }

        for name in to_remove.iter().chain(to_disconnect.iter()) {
            info!("MCP hot reload: removing server '{}'", name);
            if let Err(e) = self.disconnect_one(name).await {
                warn!(
                    "MCP hot reload: failed to disconnect '{}' during reload: {}",
                    name, e
                );
            }
        }

        let mut connected_count = 0usize;
        for config in to_connect {
            let name = config.name.clone();
            info!("MCP hot reload: connecting server '{}'", name);
            if let Err(e) = self.connect_one(config).await {
                warn!("MCP hot reload: failed to connect '{}': {}", name, e);
            } else {
                connected_count += 1;
            }
        }

        if !to_remove.is_empty() || !to_disconnect.is_empty() || connected_count > 0 {
            info!(
                "MCP hot reload complete: {} removed, {} reloaded, {} connected",
                to_remove.len(),
                to_disconnect.len(),
                connected_count
            );
        }

        self.notify_surface_changed();
        Ok(self.current_surface().await.capability_invokers)
    }

    /// 基于当前已声明配置重新执行连接/断开决策。
    pub async fn reload_declared_configs(&self) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
        let configs = {
            let declared = self.declared_configs.lock().await;
            declared.values().cloned().collect::<Vec<_>>()
        };
        self.reload_config(configs).await
    }

    /// 捕获当前 MCP reload 回滚点。
    pub async fn capture_reload_snapshot(&self) -> McpReloadSnapshot {
        let declared_configs = {
            let declared = self.declared_configs.lock().await;
            declared.values().cloned().collect::<Vec<_>>()
        };
        McpReloadSnapshot { declared_configs }
    }

    /// 按回滚点恢复声明配置与连接集合。
    pub async fn restore_reload_snapshot(
        &self,
        snapshot: &McpReloadSnapshot,
    ) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
        self.reload_config(snapshot.declared_configs.clone()).await
    }

    /// 返回所有已连接服务器的名称。
    pub async fn connected_servers(&self) -> Vec<String> {
        let conns = self.connections.lock().await;
        conns
            .iter()
            .filter(|(_, c)| c.connection.is_connected())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// 返回所有已连接服务器的能力调用器。
    pub async fn connected_invokers(&self) -> Vec<Arc<dyn CapabilityInvoker>> {
        self.current_surface().await.capability_invokers
    }

    /// 通过 MCP 协议读取指定服务器的资源。
    pub async fn read_resource(
        &self,
        server_name: &str,
        uri: &str,
    ) -> Result<crate::protocol::types::McpResourceContent> {
        let conns = self.connections.lock().await;
        let managed = conns.get(server_name).ok_or_else(|| {
            AstrError::Validation(format!(
                "MCP server '{}' not connected for resource read",
                server_name
            ))
        })?;
        let client = managed.client.lock().await;
        client.read_resource(uri).await
    }

    /// 重新连接指定服务器。
    ///
    /// 当配置未变化但用户希望手动重连时，先断开当前连接，再按已声明配置重建。
    pub async fn reconnect_server(&self, server_name: &str) -> Result<()> {
        let config = {
            let declared = self.declared_configs.lock().await;
            declared.get(server_name).cloned().ok_or_else(|| {
                AstrError::Validation(format!("MCP server '{}' not declared", server_name))
            })?
        };

        if let Err(error) = self.disconnect_one(server_name).await {
            warn!(
                "MCP reconnect '{}' ignored disconnect error before reconnect: {}",
                server_name, error
            );
        }

        if !config.enabled {
            self.notify_surface_changed();
            return Ok(());
        }
        if !self.check_approval(&config) {
            self.notify_surface_changed();
            return Ok(());
        }

        self.connect_one(config).await.map(|_| ())
    }

    /// 返回等待审批的项目级服务器列表。
    ///
    /// 供 API 端点查询，返回服务器名和签名。
    pub fn pending_approval_servers(&self) -> Vec<(String, String)> {
        let Some(lock) = &self.approval_manager else {
            return Vec::new();
        };
        let manager = lock.lock().expect("approval_manager lock");
        let project_path = match &self.project_path {
            Some(p) => p,
            None => return Vec::new(),
        };
        let signatures = manager.pending_servers(project_path);
        signatures.into_iter().map(|s| (s.clone(), s)).collect()
    }

    /// 审批指定服务器。
    pub fn approve_server(&self, server_signature: &str) -> Result<()> {
        let lock = self
            .approval_manager
            .as_ref()
            .ok_or_else(|| AstrError::Internal("approval manager not configured".into()))?;
        let manager = lock
            .lock()
            .map_err(|_| AstrError::Internal("approval manager lock poisoned".into()))?;
        let project_path = self
            .project_path
            .as_deref()
            .ok_or_else(|| AstrError::Internal("project path not set".into()))?;
        let result = manager
            .approve(project_path, server_signature, "api")
            .map_err(AstrError::Internal);
        if result.is_ok() {
            self.notify_surface_changed();
        }
        result
    }

    /// 拒绝指定服务器。
    pub fn reject_server(&self, server_signature: &str) -> Result<()> {
        let lock = self
            .approval_manager
            .as_ref()
            .ok_or_else(|| AstrError::Internal("approval manager not configured".into()))?;
        let manager = lock
            .lock()
            .map_err(|_| AstrError::Internal("approval manager lock poisoned".into()))?;
        let project_path = self
            .project_path
            .as_deref()
            .ok_or_else(|| AstrError::Internal("project path not set".into()))?;
        let result = manager
            .reject(project_path, server_signature)
            .map_err(AstrError::Internal);
        if result.is_ok() {
            self.notify_surface_changed();
        }
        result
    }

    /// 清理当前项目的审批记录。
    pub fn reset_project_choices(&self) -> Result<()> {
        let lock = self
            .approval_manager
            .as_ref()
            .ok_or_else(|| AstrError::Internal("approval manager not configured".into()))?;
        let manager = lock
            .lock()
            .map_err(|_| AstrError::Internal("approval manager lock poisoned".into()))?;
        let project_path = self
            .project_path
            .as_deref()
            .ok_or_else(|| AstrError::Internal("project path not set".into()))?;
        manager
            .reset_project(project_path)
            .map_err(AstrError::Internal)?;
        self.notify_surface_changed();
        Ok(())
    }

    // ===== 内部方法 =====

    /// 检查服务器是否已通过审批。
    ///
    /// 仅 Project scope 服务器需要审批，其他 scope 自动放行。
    /// 无审批管理器时也自动放行。
    fn check_approval(&self, config: &McpServerConfig) -> bool {
        // 非 Project scope 的服务器不需要审批
        if config.scope != crate::config::McpConfigScope::Project {
            return true;
        }
        let Some(lock) = &self.approval_manager else {
            return true;
        };
        let Some(project_path) = &self.project_path else {
            return true;
        };
        let signature = crate::config::McpConfigManager::compute_signature(config);
        let manager = lock.lock().expect("approval_manager lock");
        manager.is_approved(project_path, &signature)
    }

    async fn replace_declared_configs(&self, configs: &[McpServerConfig]) {
        let mut declared = self.declared_configs.lock().await;
        declared.clear();
        for config in configs {
            declared.insert(config.name.clone(), config.clone());
        }
    }

    async fn upsert_declared_config(&self, config: McpServerConfig) {
        let mut declared = self.declared_configs.lock().await;
        declared.insert(config.name.clone(), config);
    }

    fn notify_surface_changed(&self) {
        let _ = self.surface_events.send(());
    }

    /// 检查所有连接的健康状态，对断开的远程连接触发重连。
    ///
    /// 遍历所有标记为 Connected 的连接，检查传输层 `is_active`。
    /// 如果传输已断开，标记为 Failed 并对远程传输启动重连。
    pub async fn check_connections_health(&self) {
        let mut to_reconnect = Vec::new();

        {
            let mut conns = self.connections.lock().await;
            for (name, managed) in conns.iter_mut() {
                if !managed.connection.is_connected() {
                    continue;
                }

                // 检查传输层是否还活跃
                let is_active = {
                    let transport = managed.transport.lock().await;
                    transport.is_active()
                };

                if !is_active {
                    let server_name = name.clone();
                    let config = managed.config.clone();
                    let is_remote = config.transport.is_remote();
                    managed.connection.mark_failed("transport disconnected");
                    warn!(
                        "MCP server '{}' transport disconnected (remote={})",
                        server_name, is_remote
                    );

                    if is_remote {
                        to_reconnect.push((server_name, config));
                    }
                }
            }
        }

        // 对断开的远程连接启动重连
        for (server_name, config) in to_reconnect {
            info!("MCP triggering reconnect for '{}'", server_name);
            self.reconnect_manager.start_reconnect(
                server_name,
                config,
                self.connections.clone(),
                self.surface_events.clone(),
            );
        }
    }

    // ===== 内部方法 =====

    /// 带并发限制的批量连接。
    ///
    /// 返回每个服务器的连接结果，成功或失败互不影响。
    async fn run_batch(
        connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
        configs: Vec<McpServerConfig>,
        max_concurrency: usize,
        surface_events: broadcast::Sender<()>,
    ) -> Vec<BatchResult> {
        if configs.is_empty() {
            return Vec::new();
        }

        stream::iter(configs)
            .map(|config| {
                let connections = connections.clone();
                let surface_events = surface_events.clone();
                async move {
                    let name = config.name.clone();
                    let transport = match create_transport(&config.transport) {
                        Ok(t) => t,
                        Err(e) => {
                            warn!("MCP server '{}' transport creation failed: {}", name, e);
                            return Err((name, e.to_string()));
                        },
                    };

                    // 启动传输
                    {
                        let mut locked = transport.lock().await;
                        if let Err(e) = locked.start().await {
                            warn!("MCP server '{}' transport start failed: {}", name, e);
                            return Err((name, e.to_string()));
                        }
                    }

                    match Self::establish_connection(config, transport, connections, surface_events)
                        .await
                    {
                        Ok(invokers) => Ok((name, invokers)),
                        Err(e) => {
                            warn!("MCP server '{}' connection failed: {}", name, e);
                            Err((name, e.to_string()))
                        },
                    }
                }
            })
            .buffer_unordered(max_concurrency)
            .collect()
            .await
    }

    /// 建立完整的 MCP 连接（握手、工具发现、桥接注册）。
    ///
    /// 传输层必须已启动。委托给模块级函数实现。
    async fn establish_connection(
        config: McpServerConfig,
        transport: Arc<Mutex<dyn McpTransport>>,
        connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
        surface_events: broadcast::Sender<()>,
    ) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
        establish_connection_inner(config, transport, connections, surface_events).await
    }
}

/// 建立完整的 MCP 连接（握手、工具发现、桥接注册）。
///
/// 传输层必须已启动。模块级函数，pub(crate) 可供 reconnect 子模块调用。
pub(crate) async fn establish_connection_inner(
    config: McpServerConfig,
    transport: Arc<Mutex<dyn McpTransport>>,
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
    surface_events: broadcast::Sender<()>,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    let name = config.name.clone();

    // MCP 握手
    let client = McpClient::connect(transport.clone()).await?;

    // 获取服务器信息
    let capabilities = client.capabilities().cloned();
    let instructions = client.instructions().map(|s| s.to_string());

    // 包装客户端为共享引用
    let client = Arc::new(Mutex::new(client));

    // 获取初始化 surface
    let (tools, prompts, resources) = {
        let locked = client.lock().await;
        (
            locked.list_tools().await.unwrap_or_default(),
            locked.list_prompts().await.unwrap_or_default(),
            locked.list_resources().await.unwrap_or_default(),
        )
    };
    let invokers = surface::build_server_invokers(&name, &tools, &prompts, client.clone());
    let prompt_declarations = surface::build_prompt_declarations(&name, instructions.as_deref());
    let prompt_count = prompts.len();
    let resource_count = resources.len();

    // 注册 list_changed 通知处理器
    if client.lock().await.supports_tools() {
        let connections_ref = connections.clone();
        let client_ref = client.clone();
        let server_name = name.clone();
        let surface_events_ref = surface_events.clone();

        let mut locked = client.lock().await;
        locked.on_list_changed(
            McpListKind::Tools,
            Box::new(move || {
                let conns = connections_ref.clone();
                let cli = client_ref.clone();
                let srv = server_name.clone();
                let surface_events = surface_events_ref.clone();
                Box::pin(async move {
                    surface::refresh_tools_for_server(conns, cli, &srv).await;
                    let _ = surface_events.send(());
                })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            }),
        );
    }
    if client.lock().await.supports_prompts() {
        let connections_ref = connections.clone();
        let client_ref = client.clone();
        let server_name = name.clone();
        let surface_events_ref = surface_events.clone();

        let mut locked = client.lock().await;
        locked.on_list_changed(
            McpListKind::Prompts,
            Box::new(move || {
                let conns = connections_ref.clone();
                let cli = client_ref.clone();
                let srv = server_name.clone();
                let surface_events = surface_events_ref.clone();
                Box::pin(async move {
                    surface::refresh_prompts_for_server(conns, cli, &srv).await;
                    let _ = surface_events.send(());
                })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            }),
        );
    }
    if client.lock().await.supports_resources() {
        let connections_ref = connections.clone();
        let client_ref = client.clone();
        let server_name = name.clone();
        let surface_events_ref = surface_events.clone();

        let mut locked = client.lock().await;
        locked.on_list_changed(
            McpListKind::Resources,
            Box::new(move || {
                let conns = connections_ref.clone();
                let cli = client_ref.clone();
                let srv = server_name.clone();
                let surface_events = surface_events_ref.clone();
                Box::pin(async move {
                    surface::refresh_resources_for_server(conns, cli, &srv).await;
                    let _ = surface_events.send(());
                })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            }),
        );
    }

    // 创建并存储连接状态
    let mut connection = McpConnection::new(&name);
    if let Some(caps) = capabilities {
        connection.mark_connected(caps, instructions);
    } else {
        connection.mark_failed("no capabilities received");
    }

    {
        let mut conns = connections.lock().await;
        conns.insert(
            name.clone(),
            McpManagedConnection {
                connection,
                transport,
                client,
                invokers: invokers.clone(),
                tools,
                prompts,
                resources,
                prompt_declarations,
                config,
            },
        );
    }
    info!(
        "MCP server '{}' connected: {} tools, {} prompts, {} resources",
        name,
        invokers.len(),
        prompt_count,
        resource_count
    );

    Ok(invokers)
}

#[async_trait]
impl ManagedRuntimeComponent for McpConnectionManager {
    fn component_name(&self) -> String {
        "mcp_connection_manager".to_string()
    }

    async fn shutdown_component(&self) -> std::result::Result<(), AstrError> {
        info!("MCP connection manager shutting down");

        // 取消所有重连任务
        self.reconnect_manager.cancel_all();

        // 等待进行中调用完成
        if let Err(e) = self.wait_idle(30).await {
            warn!("MCP shutdown wait idle: {}", e);
        }

        // 关闭所有连接
        let mut conns = self.connections.lock().await;
        for (name, managed) in conns.drain() {
            let mut transport = managed.transport.lock().await;
            if let Err(e) = transport.close().await {
                warn!("MCP server '{}' close error: {}", name, e);
            }
        }

        info!("MCP connection manager shutdown complete");
        Ok(())
    }
}

/// 根据传输配置创建对应的传输实例。
///
/// 返回未启动的传输（调用方负责 start）。pub(crate) 可供 reconnect 子模块调用。
pub(crate) fn create_transport(
    config: &McpTransportConfig,
) -> Result<Arc<Mutex<dyn McpTransport>>> {
    match config {
        McpTransportConfig::Stdio { command, args, env } => {
            let env_pairs: Vec<(String, String)> =
                env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let transport = StdioTransport::new(command, args.clone(), env_pairs);
            Ok(Arc::new(Mutex::new(transport)))
        },
        McpTransportConfig::StreamableHttp { url, headers, .. } => {
            let header_pairs: Vec<(String, String)> = headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let transport = StreamableHttpTransport::new(url, header_pairs);
            Ok(Arc::new(Mutex::new(transport)))
        },
        McpTransportConfig::Sse { url, headers, .. } => {
            let header_pairs: Vec<(String, String)> = headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let transport = SseTransport::new(url, header_pairs);
            Ok(Arc::new(Mutex::new(transport)))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpConfigScope;

    #[test]
    fn test_new_manager() {
        let manager = McpConnectionManager::new();
        assert_eq!(manager.in_flight_count(), 0);
    }

    #[tokio::test]
    async fn test_connect_all_empty() {
        let manager = McpConnectionManager::new();
        let results = manager.connect_all(Vec::new()).await;

        assert!(results.connected.is_empty());
        assert!(results.failed.is_empty());
        assert!(results.invokers.is_empty());
    }

    #[tokio::test]
    async fn test_connect_all_error_isolation() {
        // 两个无效配置，各自独立失败
        let configs = vec![
            McpServerConfig {
                name: "bad-server-1".to_string(),
                transport: McpTransportConfig::Stdio {
                    command: "nonexistent-command-xyz".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                },
                scope: McpConfigScope::Project,
                enabled: true,
                timeout_secs: 120,
                init_timeout_secs: 30,
                max_reconnect_attempts: 5,
            },
            McpServerConfig {
                name: "bad-server-2".to_string(),
                transport: McpTransportConfig::Stdio {
                    command: "another-nonexistent-abc".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                },
                scope: McpConfigScope::Project,
                enabled: true,
                timeout_secs: 120,
                init_timeout_secs: 30,
                max_reconnect_attempts: 5,
            },
        ];

        let manager = McpConnectionManager::new();
        let results = manager.connect_all(configs).await;

        // 两个都应失败，但互不影响
        assert!(results.connected.is_empty());
        assert_eq!(results.failed.len(), 2);

        let failed_names: Vec<&str> = results.failed.iter().map(|(n, _)| n.as_str()).collect();
        assert!(failed_names.contains(&"bad-server-1"));
        assert!(failed_names.contains(&"bad-server-2"));
    }

    #[tokio::test]
    async fn test_connect_one_remote_connection_fails() {
        // 远程服务器无法连接时会失败（不是 transport 创建失败）
        let config = McpServerConfig {
            name: "remote-server".to_string(),
            transport: McpTransportConfig::StreamableHttp {
                url: "http://localhost:1/mcp".to_string(),
                headers: HashMap::new(),
                oauth: None,
            },
            scope: McpConfigScope::User,
            enabled: true,
            timeout_secs: 5,
            init_timeout_secs: 5,
            max_reconnect_attempts: 5,
        };

        let manager = McpConnectionManager::new();
        match manager.connect_one(config).await {
            Err(e) => {
                let msg = e.to_string();
                // 连接失败（refused 或 timeout）都是预期行为
                assert!(
                    msg.contains("HTTP") || msg.contains("connection") || msg.contains("refused"),
                    "unexpected error: {}",
                    msg
                );
            },
            Ok(_) => panic!("expected error for unreachable server"),
        }
    }

    #[tokio::test]
    async fn test_connect_one_duplicate_replaces() {
        // 连接失败的服务器，重复调用应替换（不 panic）
        let config = McpServerConfig {
            name: "dup-server".to_string(),
            transport: McpTransportConfig::Stdio {
                command: "nonexistent".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };

        let manager = McpConnectionManager::new();
        let _ = manager.connect_one(config.clone()).await;
        let _ = manager.connect_one(config).await;

        // 第二次调用不应 panic（HashMap insert 替换）
    }

    #[tokio::test]
    async fn test_connected_servers_empty() {
        let manager = McpConnectionManager::new();
        let servers = manager.connected_servers().await;
        assert!(servers.is_empty());
    }

    #[tokio::test]
    async fn test_connected_invokers_empty() {
        let manager = McpConnectionManager::new();
        let invokers = manager.connected_invokers().await;
        assert!(invokers.is_empty());
    }

    #[tokio::test]
    async fn test_shutdown_clean() {
        let manager = McpConnectionManager::new();
        let result = manager.shutdown_component().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_idle_no_in_flight() {
        let manager = McpConnectionManager::new();
        let result = manager.wait_idle(1).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_transport_config_is_remote() {
        assert!(
            !McpTransportConfig::Stdio {
                command: "test".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            }
            .is_remote()
        );

        assert!(
            McpTransportConfig::StreamableHttp {
                url: "http://localhost".to_string(),
                headers: HashMap::new(),
                oauth: None,
            }
            .is_remote()
        );

        assert!(
            McpTransportConfig::Sse {
                url: "http://localhost".to_string(),
                headers: HashMap::new(),
                oauth: None,
            }
            .is_remote()
        );
    }

    #[test]
    fn test_no_approval_manager_passes_all() {
        // 无审批管理器时所有服务器自动放行
        let manager = McpConnectionManager::new();
        let config = McpServerConfig {
            name: "test-server".to_string(),
            transport: McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };
        assert!(manager.check_approval(&config));
    }

    #[test]
    fn test_pending_approval_servers_empty_without_manager() {
        let manager = McpConnectionManager::new();
        let pending = manager.pending_approval_servers();
        assert!(pending.is_empty());
    }

    #[test]
    fn test_with_approval_builder_pattern() {
        use crate::config::{McpApprovalManager, settings_port::McpSettingsStore};

        struct NopStore;
        impl McpSettingsStore for NopStore {
            fn load_approvals(
                &self,
                _: &str,
            ) -> std::result::Result<Vec<crate::config::McpApprovalData>, String> {
                Ok(Vec::new())
            }
            fn save_approval(
                &self,
                _: &str,
                _: &crate::config::McpApprovalData,
            ) -> std::result::Result<(), String> {
                Ok(())
            }
            fn clear_approvals(&self, _: &str) -> std::result::Result<(), String> {
                Ok(())
            }
        }

        let manager = McpConnectionManager::new().with_approval(
            McpApprovalManager::new(Box::new(NopStore)),
            "/test/project".to_string(),
        );

        // 无已审批数据时项目级服务器应被拒绝
        let config = McpServerConfig {
            name: "test-server".to_string(),
            transport: McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };
        assert!(!manager.check_approval(&config));

        // User scope 的服务器不受审批限制
        let user_config = McpServerConfig {
            name: "user-server".to_string(),
            transport: McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::User,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };
        assert!(manager.check_approval(&user_config));
    }

    #[tokio::test]
    async fn test_connect_all_skips_unapproved_project_servers() {
        use crate::config::{McpApprovalManager, settings_port::McpSettingsStore};

        struct NopStore;
        impl McpSettingsStore for NopStore {
            fn load_approvals(
                &self,
                _: &str,
            ) -> std::result::Result<Vec<crate::config::McpApprovalData>, String> {
                Ok(Vec::new())
            }
            fn save_approval(
                &self,
                _: &str,
                _: &crate::config::McpApprovalData,
            ) -> std::result::Result<(), String> {
                Ok(())
            }
            fn clear_approvals(&self, _: &str) -> std::result::Result<(), String> {
                Ok(())
            }
        }

        let manager = McpConnectionManager::new().with_approval(
            McpApprovalManager::new(Box::new(NopStore)),
            "/test/project".to_string(),
        );

        // 项目级未审批服务器应被跳过（不连接也不报错）
        let configs = vec![McpServerConfig {
            name: "unapproved".to_string(),
            transport: McpTransportConfig::Stdio {
                command: "nonexistent".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        }];

        let results = manager.connect_all(configs).await;
        // 未审批的服务器不进入连接流程，不算失败
        assert!(results.connected.is_empty());
        assert!(results.failed.is_empty());
    }

    #[tokio::test]
    async fn reload_snapshot_restores_declared_server_set() {
        let manager = McpConnectionManager::new();
        let alpha = McpServerConfig {
            name: "alpha".to_string(),
            transport: McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::User,
            enabled: false,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };
        let beta = McpServerConfig {
            name: "beta".to_string(),
            ..alpha.clone()
        };

        manager
            .reload_config(vec![alpha])
            .await
            .expect("alpha reload");
        let snapshot = manager.capture_reload_snapshot().await;

        manager
            .reload_config(vec![beta])
            .await
            .expect("beta reload");
        let names = manager
            .current_surface()
            .await
            .server_statuses
            .into_iter()
            .map(|status| status.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["beta".to_string()]);

        manager
            .restore_reload_snapshot(&snapshot)
            .await
            .expect("restore should succeed");
        let restored_names = manager
            .current_surface()
            .await
            .server_statuses
            .into_iter()
            .map(|status| status.name)
            .collect::<Vec<_>>();
        assert_eq!(restored_names, vec!["alpha".to_string()]);
    }
}
