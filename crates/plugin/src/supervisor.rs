//! 插件 Supervisor—— 管理插件的完整生命周期。
//!
//! `Supervisor` 是插件系统的核心门面（facade），组合了：
//!
//! - `PluginProcess` — 子进程管理
//! - `Peer` — JSON-RPC 通信
//! - `InitializeResultData` — 握手协商结果
//!
//! ## 职责
//!
//! - 启动插件进程并完成握手
//! - 提供能力调用接口（一元和流式）
//! - 健康检查
//! - 优雅关闭（先中止 peer 后台任务，再终止进程）
//!
//! ## 与 Runtime 的集成
//!
//! `Supervisor` 实现了 `ManagedRuntimeComponent` trait，
//! 可以被 runtime 统一管理生命周期。

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use astrcode_core::{ManagedRuntimeComponent, PluginManifest, Result};
use astrcode_protocol::plugin::{
    CapabilityDescriptor, InitializeMessage, InitializeResultData, InvokeMessage, PeerDescriptor,
    ProfileDescriptor, ResultMessage, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{CapabilityRouter, Peer, PluginProcess, StreamExecution};

/// 插件健康状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorHealth {
    /// 插件正常运行
    Healthy,
    /// 插件不可用（进程退出或 peer 关闭）
    Unavailable,
}

/// 插件健康检查报告。
///
/// 包含健康状态和可选的描述信息。
/// 当状态为 `Unavailable` 时，`message` 通常包含具体原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorHealthReport {
    pub health: SupervisorHealth,
    pub message: Option<String>,
}

/// 插件 Supervisor—— 管理插件的完整生命周期。
///
/// 作为插件系统的门面，封装了进程管理、通信和握手协商的所有细节。
/// 调用方只需与 `Supervisor` 交互，无需直接操作 `PluginProcess` 或 `Peer`。
pub struct Supervisor {
    manifest_name: String,
    process: Mutex<PluginProcess>,
    peer: Peer,
    remote_initialize: InitializeResultData,
}

impl Supervisor {
    /// 启动插件并完成握手的便捷方法。
    ///
    /// 等价于 `PluginProcess::start()` + `from_process()`。
    pub async fn start(manifest: &PluginManifest, local_peer: PeerDescriptor) -> Result<Self> {
        let process = PluginProcess::start(manifest).await?;
        Self::from_process(process, local_peer, None).await
    }

    /// 从已有的进程创建 Supervisor 并完成握手。
    ///
    /// # 流程
    ///
    /// 1. 创建默认的 `CapabilityRouter`（用于处理插件→宿主的反向调用）
    /// 2. 构建 `InitializeMessage`（使用默认值或自定义值）
    /// 3. 创建 `Peer` 并启动读循环
    /// 4. 发送 `InitializeMessage` 并等待响应
    /// 5. 如果握手失败，终止进程并返回错误
    ///
    /// # 错误处理
    ///
    /// 如果 `initialize()` 失败，会先尝试终止进程再返回错误，
    /// 避免留下僵尸进程。
    pub async fn from_process(
        process: PluginProcess,
        local_peer: PeerDescriptor,
        local_initialize: Option<InitializeMessage>,
    ) -> Result<Self> {
        let mut process = process;
        let manifest_name = process.manifest.name.clone();
        let router = Arc::new(CapabilityRouter::default());
        let initialize = local_initialize.unwrap_or_else(|| {
            default_initialize_message(local_peer, Vec::new(), default_profiles())
        });
        let peer = Peer::new(process.transport(), initialize, router);
        let remote_initialize = match peer.initialize().await {
            Ok(remote_initialize) => remote_initialize,
            Err(error) => {
                if let Err(shutdown_error) = process.shutdown().await {
                    log::warn!(
                        "failed to terminate plugin '{}' after initialize error: {}",
                        manifest_name,
                        shutdown_error
                    );
                }
                return Err(error);
            }
        };
        Ok(Self {
            manifest_name,
            process: Mutex::new(process),
            peer,
            remote_initialize,
        })
    }

    /// 获取握手协商结果。
    ///
    /// 返回插件声明的能力列表、支持的 profiles 和元数据。
    pub fn remote_initialize(&self) -> &InitializeResultData {
        &self.remote_initialize
    }

    /// 获取 Peer 的克隆。
    ///
    /// 仅供内部使用（`pub(crate)`），外部应通过 `invoke()` 等方法间接使用。
    pub(crate) fn peer(&self) -> Peer {
        self.peer.clone()
    }

    /// 调用插件的某个能力（一元模式）。
    ///
    /// 自动生成 UUID 作为请求 ID，设置 `stream: false`。
    pub async fn invoke(
        &self,
        capability: impl Into<String>,
        input: Value,
        context: astrcode_protocol::plugin::InvocationContext,
    ) -> Result<ResultMessage> {
        self.peer
            .invoke(InvokeMessage {
                id: Uuid::new_v4().to_string(),
                capability: capability.into(),
                input,
                context,
                stream: false,
            })
            .await
    }

    /// 调用插件的某个能力（流式模式）。
    ///
    /// 自动生成 UUID 作为请求 ID，设置 `stream: true`。
    /// 返回 `StreamExecution` 用于接收增量事件。
    pub async fn invoke_stream(
        &self,
        capability: impl Into<String>,
        input: Value,
        context: astrcode_protocol::plugin::InvocationContext,
    ) -> Result<StreamExecution> {
        self.peer
            .invoke_stream(InvokeMessage {
                id: Uuid::new_v4().to_string(),
                capability: capability.into(),
                input,
                context,
                stream: true,
            })
            .await
    }

    /// 取消一个正在进行的请求。
    pub async fn cancel(
        &self,
        request_id: impl Into<String>,
        reason: Option<String>,
    ) -> Result<()> {
        self.peer.cancel(request_id, reason).await
    }

    /// 优雅关闭插件。
    ///
    /// # 关闭顺序
    ///
    /// 1. 中止 peer 的读循环和所有活跃的 invoke 处理器
    /// 2. 终止子进程
    ///
    /// 这个顺序很重要：如果先终止进程，peer 的后台任务可能因为
    /// 传输层管道断裂而产生不可预期的行为。
    pub async fn shutdown(&self) -> Result<()> {
        // Abort the read loop and any in-flight invoke handlers first, then
        // terminate the child process.  This order ensures the peer's background
        // tasks don't linger after the process exits (which could cause the
        // transport to hang if stdin/stdout pipes don't close promptly).
        self.peer.abort().await;
        self.process.lock().await.shutdown().await
    }

    /// 检查插件健康状态。
    ///
    /// # 检查顺序
    ///
    /// 1. 首先检查 peer 是否已关闭（协议层异常）
    /// 2. 然后检查进程是否仍在运行（进程层异常）
    ///
    /// # 返回
    ///
    /// - `Healthy` — 进程运行中且 peer 未关闭
    /// - `Unavailable` — peer 已关闭或进程已退出，`message` 包含具体原因
    pub async fn health_report(&self) -> Result<SupervisorHealthReport> {
        if let Some(reason) = self.peer.closed_reason().await {
            return Ok(SupervisorHealthReport {
                health: SupervisorHealth::Unavailable,
                message: Some(format!("protocol peer closed: {reason}")),
            });
        }

        let status = self.process.lock().await.status()?;
        if status.running {
            Ok(SupervisorHealthReport {
                health: SupervisorHealth::Healthy,
                message: None,
            })
        } else {
            Ok(SupervisorHealthReport {
                health: SupervisorHealth::Unavailable,
                message: Some(match status.exit_code {
                    Some(code) => format!("plugin process exited with code {code}"),
                    None => "plugin process exited".to_string(),
                }),
            })
        }
    }
}

#[async_trait]
impl ManagedRuntimeComponent for Supervisor {
    fn component_name(&self) -> String {
        format!("plugin supervisor '{}'", self.manifest_name)
    }

    async fn shutdown_component(&self) -> Result<()> {
        self.shutdown().await
    }
}

/// 构建默认的 `InitializeMessage`。
///
/// 用于宿主向插件发送初始化请求。
/// 包含本地 peer 信息、支持的能力、profiles 和传输元数据。
///
/// # 参数
///
/// * `local_peer` - 本地 peer 描述
/// * `capabilities` - 本地支持的能力列表（宿主→插件的反向调用）
/// * `profiles` - 支持的 profile 列表
pub fn default_initialize_message(
    local_peer: PeerDescriptor,
    capabilities: Vec<CapabilityDescriptor>,
    profiles: Vec<ProfileDescriptor>,
) -> InitializeMessage {
    InitializeMessage {
        id: Uuid::new_v4().to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        supported_protocol_versions: vec![PROTOCOL_VERSION.to_string()],
        peer: local_peer,
        capabilities,
        handlers: Vec::new(),
        profiles,
        metadata: json!({ "transport": "stdio" }),
    }
}

/// 构建默认的 profiles 列表。
///
/// 当前只支持 `coding` profile，包含编码工作流的上下文 schema：
/// - `workingDir` — 工作目录
/// - `repoRoot` — 仓库根目录
/// - `openFiles` — 已打开的文件列表
/// - `activeFile` — 当前活跃文件
/// - `selection` — 选区信息
/// - `approvalMode` — 审批模式
///
/// # 扩展
///
/// 未来可以添加更多 profile（如 `review`、`debug` 等），
/// 每个 profile 定义自己的上下文 schema。
pub fn default_profiles() -> Vec<ProfileDescriptor> {
    vec![ProfileDescriptor {
        name: "coding".to_string(),
        version: "1".to_string(),
        description: "Coding workflow profile".to_string(),
        context_schema: json!({
            "type": "object",
            "properties": {
                "workingDir": { "type": "string" },
                "repoRoot": { "type": "string" },
                "openFiles": { "type": "array", "items": { "type": "string" } },
                "activeFile": { "type": "string" },
                "selection": { "type": "object" },
                "approvalMode": { "type": "string" }
            }
        }),
        metadata: Value::Null,
    }]
}
