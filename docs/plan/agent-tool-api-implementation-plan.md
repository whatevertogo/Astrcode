# Agent as Tool + 开放 API 实施计划

## 总览

本计划分为多个阶段, 每个阶段有独立的交付物和验证标准。总预计工作量基于现有 Astrcode 架构的最小侵入性改造。

```
Phase 0 (设计完成) → Phase 1 (Agent Loader) → Phase 2 (Agent as Tool)
    → Phase 3 (扩展 API) → Phase 4 (WebSocket) → Phase 5 (前端适配)
```

---

## Phase 0: 基础设施准备

**目标**: 创建新 crate, 配置依赖, 建立开发环境

**预估时间**: 0.5 天

### 步骤

#### 0.1 创建新 Crate 骨架

```bash
cd crates

# Agent 定义加载器 (预置 Agent 配置)
cargo init --lib runtime-agent-loader

# Agent as Tool 实现
cargo init --lib runtime-agent-tool

# 扩展 API 层
cargo init --lib runtime-agent-api
```

#### 0.2 配置 Cargo.toml 依赖

**crates/runtime-agent-loader/Cargo.toml**:
```toml
[package]
name = "runtime-agent-loader"
version = "0.1.0"
edition = "2021"

[dependencies]
core = { path = "../core" }
protocol = { path = "../protocol" }
serde = { workspace = true }
serde_json = { workspace = true }
```

**crates/runtime-agent-tool/Cargo.toml**:
```toml
[package]
name = "runtime-agent-tool"
version = "0.1.0"
edition = "2021"

[dependencies]
runtime-agent-loop = { path = "../runtime-agent-loop" }
runtime-agent-loader = { path = "../runtime-agent-loader" }
core = { path = "../core" }
protocol = { path = "../protocol" }
tokio = { workspace = true }
tokio-util = { workspace = true, features = ["sync"] }
tracing = { workspace = true }
serde = { workspace = true }
```

**crates/runtime-agent-api/Cargo.toml**:
```toml
[package]
name = "runtime-agent-api"
version = "0.1.0"
edition = "2021"

[dependencies]
server = { path = "../server" }
runtime = { path = "../runtime" }
runtime-agent-tool = { path = "../runtime-agent-tool" }
runtime-agent-loader = { path = "../runtime-agent-loader" }
core = { path = "../core" }
protocol = { path = "../protocol" }
axum = { version = "0.7", features = ["ws", "json"] }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
tokio = { workspace = true }
tokio-stream = { workspace = true }
tower-http = { version = "0.5", features = ["cors", "limit"] }
utoipa = { version = "4", features = ["axum_extras"] }
utoipa-swagger-ui = { version = "7", features = ["axum"] }
```

#### 0.3 修改 workspace Cargo.toml

```toml
# d:\GitObjectsOwn\Astrcode\Cargo.toml
[workspace]
members = [
    # ... existing members
    "crates/runtime-agent-loader",
    "crates/runtime-agent-tool",
    "crates/runtime-agent-api",
]
```

#### 0.4 验证编译

```bash
cargo check --workspace
```

**验收标准**: `cargo check --workspace` 无错误

---

## Phase 1: Agent Loader 系统

**目标**: 实现 Agent 定义加载、合并和注册表

**预估时间**: 1 天

### 步骤

#### 1.1 定义 Agent Profile 数据模型

**文件**: `crates/runtime-agent-loader/src/lib.rs`

```rust
//! Agent Loader 定义和注册表
//!
//! Agent Loader 把子 Agent 定义收敛成统一注册表:
//! - 可用工具集
//! - 系统提示
//! - 最大步数和 Token 预算
//! - 模型偏好

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Agent Profile 唯一标识
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentProfileId(pub String);

/// Agent Profile 定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// 显示名称
    pub name: String,
    /// 描述 (用于 LLM 理解何时调用此 Agent)
    pub description: String,
    /// 系统提示 (可选, 覆盖默认)
    pub system_prompt: Option<String>,
    /// 允许使用的工具列表 (None = 无限制)
    pub allowed_tools: Option<Vec<String>>,
    /// 最大步骤数限制
    pub max_steps: usize,
    /// Token 预算 (可选)
    pub token_budget: Option<usize>,
    /// 模型偏好 ("fast" / "balanced" / "quality" 或具体模型名)
    pub model_preference: Option<String>,
    /// 是否允许请求用户审批 (子 Agent 通常为 false)
    pub can_request_approval: bool,
}

impl AgentProfile {
    /// 创建只读探索型 Agent
    pub fn explore() -> Self {
        Self {
            name: "代码探索".into(),
            description: "用于读取和理解代码库。仅执行只读操作，不会修改任何文件或执行命令。".into(),
            system_prompt: Some("你是一个代码分析助手。你的任务是阅读和理解代码，回答问题。你不应该修改任何文件或执行shell命令。分析完成后，返回清晰的摘要。".into()),
            allowed_tools: Some(vec![
                "readFile".into(),
                "listDir".into(),
                "findFiles".into(),
                "grep".into(),
            ]),
            max_steps: 5,
            token_budget: Some(8000),
            model_preference: Some("fast".into()),
            can_request_approval: false,
        }
    }

    /// 创建任务规划型 Agent
    pub fn plan() -> Self {
        Self {
            name: "任务规划".into(),
            description: "用于分析需求和制定执行计划。不修改代码，仅输出计划。".into(),
            system_prompt: Some("你是一个技术规划师。分析用户的需求，阅读相关代码，然后输出详细的分步执行计划。你不应该修改代码。".into()),
            allowed_tools: Some(vec![
                "readFile".into(),
                "grep".into(),
            ]),
            max_steps: 3,
            token_budget: Some(6000),
            model_preference: Some("balanced".into()),
            can_request_approval: false,
        }
    }

    /// 创建代码执行型 Agent
    pub fn execute() -> Self {
        Self {
            name: "代码执行".into(),
            description: "用于执行具体的代码变更任务。可以读写文件和执行shell命令。".into(),
            system_prompt: None,  // 使用默认
            allowed_tools: Some(vec![
                "readFile".into(),
                "writeFile".into(),
                "editFile".into(),
                "shell".into(),
            ]),
            max_steps: 10,
            token_budget: Some(16000),
            model_preference: Some("quality".into()),
            can_request_approval: true,
        }
    }

    /// 创建代码审查型 Agent
    pub fn review() -> Self {
        Self {
            name: "代码审查".into(),
            description: "用于审查代码的质量、安全性和最佳实践。只读操作。".into(),
            system_prompt: Some("你是一个资深的代码审查专家。检查代码的质量、安全性、性能和可维护性。提供具体的改进建议。".into()),
            allowed_tools: Some(vec![
                "readFile".into(),
                "grep".into(),
            ]),
            max_steps: 5,
            token_budget: Some(8000),
            model_preference: Some("quality".into()),
            can_request_approval: false,
        }
    }
}

/// Agent Profile 注册表
#[derive(Debug, Clone)]
pub struct AgentProfileRegistry {
    profiles: HashMap<AgentProfileId, AgentProfile>,
}

impl AgentProfileRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            profiles: HashMap::new(),
        };
        // 注册预置 Profile
        registry.register("explore", AgentProfile::explore());
        registry.register("plan", AgentProfile::plan());
        registry.register("execute", AgentProfile::execute());
        registry.register("review", AgentProfile::review());
        registry
    }

    pub fn register(&mut self, id: impl Into<String>, profile: AgentProfile) {
        self.profiles.insert(AgentProfileId(id.into()), profile);
    }

    pub fn get(&self, id: &AgentProfileId) -> Option<&AgentProfile> {
        self.profiles.get(id)
    }

    pub fn list(&self) -> Vec<(&AgentProfileId, &AgentProfile)> {
        self.profiles.iter().collect()
    }

    pub fn list_ids(&self) -> Vec<String> {
        self.profiles.keys().map(|k| k.0.clone()).collect()
    }
}

impl Default for AgentProfileRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

#### 1.2 从 Markdown 目录加载 Agent

**文件**: `crates/runtime-agent-loader/src/lib.rs`

```rust
//! 从 Markdown frontmatter 目录加载 Agent

use std::path::Path;

use crate::AgentProfileRegistry;

/// 先加载内置 agent，再按目录优先级逐层合并用户级和项目级定义。
pub fn load_from_dirs(paths: &[&Path], registry: &mut AgentProfileRegistry) -> std::io::Result<()> {
    for path in paths {
        // 保持默认能力存在，同时允许本地目录覆盖同名定义。
        load_from_dir(path, registry)?;
    }

    Ok(())
}
```

#### 1.3 单元测试

**文件**: `crates/runtime-agent-loader/src/lib.rs` (底部)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_profiles_have_sane_defaults() {
        let registry = AgentProfileRegistry::new();
        
        // explore 应该是只读的
        let explore = registry.get(&AgentProfileId("explore".into())).unwrap();
        assert!(!explore.allowed_tools.as_ref().unwrap().iter().any(|t| 
            matches!(t.as_str(), "writeFile" | "editFile" | "shell")
        ));
        assert!(!explore.can_request_approval);
        
        // execute 应该有写权限
        let execute = registry.get(&AgentProfileId("execute".into())).unwrap();
        assert!(execute.allowed_tools.as_ref().unwrap().contains(&"writeFile".into()));
    }

    #[test]
    fn test_custom_profile_override() {
        let mut registry = AgentProfileRegistry::new();
        registry.register(
            "custom-explore",
            AgentProfile {
                name: "Custom Explore".into(),
                description: "Custom".into(),
                system_prompt: None,
                allowed_tools: Some(vec!["readFile".into()]),
                max_steps: 3,
                token_budget: None,
                model_preference: None,
                can_request_approval: false,
            },
        );
        
        assert!(registry.get(&AgentProfileId("custom-explore".into())).is_some());
    }
}
```

#### 1.4 集成到 Runtime Bootstrap

**修改**: `crates/runtime/src/bootstrap.rs`

在 `RuntimeBootstrap` 中同时保留 loader 和 registry:

```rust
use runtime_agent_loader::{AgentProfileLoader, AgentProfileRegistry};

pub struct RuntimeBootstrap {
    pub service: Arc<RuntimeService>,
    pub coordinator: Arc<RuntimeCoordinator>,
    pub governance: Arc<RuntimeGovernance>,
    pub plugin_load_handle: PluginLoadHandle,
    // 新增
    pub agent_loader: Arc<AgentProfileLoader>,
    pub agent_profiles: Arc<AgentProfileRegistry>,
}
```

**验收标准**: 
- `cargo test --package runtime-agent-loader` 全部通过
- `cargo check --workspace` 无错误

---

## Phase 2: Agent as Tool 实现

**目标**: 实现 `RunAgentTool`, 将 Agent Loop 作为 Tool 暴露给 LLM

**预估时间**: 2 天

### 步骤

#### 2.1 扩展 StorageEvent (新增子 Agent 事件)

**文件**: `crates/core/src/event/mod.rs` (或 `storage_event.rs`)

在现有 `StorageEvent` enum 中添加:

```rust
/// 子 Agent Turn 开始
SubAgentTurnStart {
    storage_seq: u64,
    turn_id: String,                  // 父 turn_id
    sub_turn_id: String,              // 子 turn_id
    agent_profile: String,            // Profile ID
    task: String,                     // 任务描述
    max_steps: usize,
    token_budget: Option<usize>,
    model: Option<String>,
    timestamp: DateTime<Utc>,
},

/// 子 Agent Turn 完成
SubAgentTurnEnd {
    storage_seq: u64,
    turn_id: String,
    sub_turn_id: String,
    agent_profile: String,
    outcome: String,                  // "completed" / "failed" / "aborted" / "token_exceeded"
    summary: String,                  // LLM可读摘要
    token_usage: Option<TokenUsage>,
    timestamp: DateTime<Utc>,
},
```

#### 2.2 定义 RunAgent Tool 参数

**文件**: `crates/runtime-agent-tool/src/lib.rs`

```rust
use serde::{Deserialize, Serialize};

/// runAgent 工具调用参数
#[derive(Debug, Serialize, Deserialize)]
pub struct RunAgentParams {
    /// Agent Profile 名称 (如 "explore", "plan")
    pub name: String,
    /// 任务描述 (会作为子 Agent 的用户消息)
    pub task: String,
    /// 额外上下文 (可选)
    pub context: Option<String>,
    /// 覆盖最大步数 (可选)
    pub max_steps: Option<usize>,
}
```

#### 2.3 实现 SubAgentLoop (子 Agent 执行引擎)

**文件**: `crates/runtime-agent-tool/src/sub_agent_loop.rs`

```rust
//! 子 Agent 执行引擎
//!
//! 子 Agent Loop 是主 Agent Loop 的受限版本:
//! - 使用独立的 Prompt (仅包含任务描述)
//! - 受限的工具集 (根据 Profile)
//! - 共享父的 CancelToken (父取消 → 子也取消)
//! - 事件写入同一个 EventLog (标记 parent_turn_id)

use std::sync::Arc;

use runtime_agent_loop::AgentLoop;
use runtime_agent_profiles::{AgentProfile, AgentProfileRegistry};
use core::event::EventLogWriter;
use core::policy::{PolicyEngine, PolicyContext, PolicyVerdict};
use core::cancel::CancelToken;
use protocol::http::PromptRequest;
use crate::RunAgentParams;
use tokio::sync::mpsc;
use tracing;

/// 子 Agent 执行结果
#[derive(Debug)]
pub struct SubAgentResult {
    /// 执行结果状态
    pub outcome: SubAgentOutcome,
    /// LLM 可读的摘要
    pub summary: String,
    /// 产生的 artifacts (文件修改等)
    pub artifacts: Vec<ArtifactRef>,
    /// Token 使用量
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug)]
pub enum SubAgentOutcome {
    Completed,
    Failed { error: String },
    Aborted,
    TokenExceeded,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub kind: String,  // "file", "tool_call", etc.
    pub reference: String,
}

/// 自定义政策引擎: 限制子 Agent 的工具访问
pub struct SubAgentPolicyEngine {
    parent: Arc<dyn PolicyEngine>,
    allowed_tools: Vec<String>,
    can_request_approval: bool,
}

impl SubAgentPolicyEngine {
    pub fn new(
        parent: Arc<dyn PolicyEngine>,
        profile: &AgentProfile,
    ) -> Self {
        Self {
            parent,
            allowed_tools: profile.allowed_tools.clone().unwrap_or_default(),
            can_request_approval: profile.can_request_approval,
        }
    }
}

#[async_trait::async_trait]
impl PolicyEngine for SubAgentPolicyEngine {
    async fn check(&self, call: &CapabilityCall, ctx: &PolicyContext) -> PolicyVerdict {
        // 检查工具是否在 allowed_tools 中
        if !self.allowed_tools.is_empty() && !self.allowed_tools.contains(&call.tool_name) {
            return PolicyVerdict::deny(format!(
                "工具 '{}' 不在 Agent 允许列表中 (允许: {:?})",
                call.tool_name, self.allowed_tools
            ));
        }
        
        // 子 Agent 不能请求用户审批
        if !self.can_request_approval {
            // ... 检查是否需要审批，如果是则拒绝
        }
        
        // 继承父策略的判断
        self.parent.check(call, ctx).await
    }
}

/// 子 Agent 执行器
pub struct SubAgentExecutor {
    profiles: Arc<AgentProfileRegistry>,
    parent_turn_id: String,
    event_writer: Arc<dyn EventLogWriter>,
}

impl SubAgentExecutor {
    pub fn new(
        profiles: Arc<AgentProfileRegistry>,
        parent_turn_id: String,
        event_writer: Arc<dyn EventLogWriter>,
    ) -> Self {
        Self {
            profiles,
            parent_turn_id,
            event_writer,
        }
    }

    /// 执行子 Agent
    pub async fn execute(
        &self,
        params: RunAgentParams,
        cancel_token: CancelToken,
        // ... 其他必要参数 (factory, capabilities 等从 RuntimeService 获取)
    ) -> Result<SubAgentResult, Box<dyn std::error::Error + Send + Sync>> {
        let profile = self.profiles
            .get(&runtime_agent_profiles::AgentProfileId(params.name.clone()))
            .ok_or_else(|| format!("Agent Profile '{}' 不存在", params.name))?;
        
        // 1. 生成子 turn_id
        let sub_turn_id = generate_sub_turn_id(&self.parent_turn_id);
        
        // 2. 记录 SubAgentTurnStart
        self.event_writer.append(StorageEvent::SubAgentTurnStart {
            storage_seq: 0,  // 会由 writer 自动填充
            turn_id: self.parent_turn_id.clone(),
            sub_turn_id: sub_turn_id.clone(),
            agent_profile: params.name.clone(),
            task: params.task.clone(),
            max_steps: params.max_steps.unwrap_or(profile.max_steps),
            token_budget: params.context.as_ref().map(|_| profile.token_budget.unwrap_or(0)),
            model: profile.model_preference.clone(),
            timestamp: Utc::now(),
        }).await?;
        
        let start = Instant::now();
        
        // 3. 构建子 Agent 的 Prompt (仅包含任务描述)
        let system_prompt = profile.system_prompt
            .clone()
            .unwrap_or_default();
        let user_message = if let Some(context) = &params.context {
            format!("# 任务\n\n{}\n\n# 额外上下文\n\n{}", params.task, context)
        } else {
            params.task.clone()
        };
        
        // 4. 构建子 Agent Loop
        //    - 使用受限的 policy engine
        //    - 使用受限的工具集 (根据 Profile)
        //    - 共享 cancel_token
        let sub_policy = Arc::new(SubAgentPolicyEngine::new(
            /* parent policy from runtime */,
            profile,
        ));
        
        // 5. 执行
        let result = match self.run_sub_loop(
            &sub_turn_id,
            system_prompt,
            user_message,
            sub_policy,
            cancel_token.clone(),
            // ... 
        ).await {
            Ok(output) => SubAgentResult {
                outcome: SubAgentOutcome::Completed,
                summary: Self::summarize_output(&output, profile.max_steps),
                artifacts: vec![],  // 可以从 output 中提取
                token_usage: output.token_usage,
            },
            Err(e) if cancel_token.is_cancelled() => SubAgentResult {
                outcome: SubAgentOutcome::Aborted,
                summary: "Agent 执行被中止".into(),
                artifacts: vec![],
                token_usage: None,
            },
            Err(e) => SubAgentResult {
                outcome: SubAgentOutcome::Failed { error: e.to_string() },
                summary: format!("Agent 执行失败: {}", e),
                artifacts: vec![],
                token_usage: None,
            },
        };
        
        // 6. 记录 SubAgentTurnEnd
        self.event_writer.append(StorageEvent::SubAgentTurnEnd {
            storage_seq: 0,
            turn_id: self.parent_turn_id.clone(),
            sub_turn_id,
            agent_profile: params.name.clone(),
            outcome: match result.outcome {
                SubAgentOutcome::Completed => "completed".into(),
                SubAgentOutcome::Failed { .. } => "failed".into(),
                SubAgentOutcome::Aborted => "aborted".into(),
                SubAgentOutcome::TokenExceeded => "token_exceeded".into(),
            },
            summary: result.summary.clone(),
            token_usage: result.token_usage.clone(),
            timestamp: Utc::now(),
        }).await?;
        
        tracing::info!(
            "SubAgent '{}' completed in {:?}, outcome: {:?}",
            params.name,
            start.elapsed(),
            result.outcome
        );
        
        Ok(result)
    }
    
    /// 结果摘要: 截断过长的输出, 使其适合 LLM 消费
    fn summarize_output(output: &str, max_summary_chars: usize) -> String {
        if output.len() <= max_summary_chars {
            output.to_string()
        } else {
            let truncated = &output[..max_summary_chars];
            format!("{}\n\n[输出已截断, 完整内容可通过事件查询]", truncated)
        }
    }
}

fn generate_sub_turn_id(parent_turn_id: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    format!("{}-sub-{}", parent_turn_id, ts)
}
```

#### 2.4 实现 RunAgentTool (Tool Trait)

**文件**: `crates/runtime-agent-tool/src/tool.rs`

```rust
//! RunAgent Tool 实现

use std::sync::Arc;
use async_trait::async_trait;
use core::tool::{Tool, ToolContext, ToolExecutionResult};
use runtime_agent_profiles::AgentProfileRegistry;
use crate::sub_agent_loop::SubAgentExecutor;
use crate::RunAgentParams;

pub struct RunAgentTool {
    profiles: Arc<AgentProfileRegistry>,
    // 从 RuntimeService 获取的执行器所需组件
    executor_factory: Arc<dyn SubAgentExecutorFactory>,
}

impl RunAgentTool {
    pub fn new(
        profiles: Arc<AgentProfileRegistry>,
        executor_factory: Arc<dyn SubAgentExecutorFactory>,
    ) -> Self {
        Self { profiles, executor_factory }
    }

    /// 生成对 LLM 友好的描述
    pub fn description_for_llm(registry: &AgentProfileRegistry) -> String {
        let profiles = registry.list();
        let profile_list: String = profiles
            .iter()
            .map(|(id, p)| format!(
                "- `{}`: {}",
                id.0, p.description
            ))
            .collect::<Vec<_>>()
            .join("\n");
        
        format!(
            r##"调用子 Agent 执行特定任务。

可用的 Agent Profile:
{}

每个 Profile 都有不同的工具权限和专精领域。选择合适的 Profile 可以提高效率和准确性。

子 Agent 会独立运行, 完成后返回结果摘要。

参数:
- name: Agent Profile 名称 (必须是上述列表中的一个)
- task: 任务描述, 应详细说明子 Agent 需要做什么
- context: (可选) 额外上下文信息
- max_steps: (可选) 覆盖最大步数"##,
            profile_list
        )
    }

    /// 生成 JSON Schema 供 LLM 理解
    pub fn parameters_json_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Agent Profile 名称",
                    "enum": ["explore", "plan", "execute", "review"]
                },
                "task": {
                    "type": "string",
                    "description": "任务详细描述"
                },
                "context": {
                    "type": "string",
                    "description": "可选的额外上下文"
                },
                "max_steps": {
                    "type": "integer",
                    "description": "可选的最大步数覆盖"
                }
            },
            "required": ["name", "task"]
        })
    }
}

#[async_trait]
impl Tool for RunAgentTool {
    async fn invoke(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult, Box<dyn std::error::Error + Send + Sync>> {
        // 1. 解析参数
        let params: RunAgentParams = serde_json::from_value(args)
            .map_err(|e| format!("参数解析失败: {}", e))?;
        
        // 2. 验证 Profile 存在
        if self.profiles.get(&runtime_agent_profiles::AgentProfileId(params.name.clone())).is_none() {
            return Ok(ToolExecutionResult::error(
                format!("未知的 Agent Profile: '{}'. 可用: {:?}", 
                    params.name, self.profiles.list_ids())
            ));
        }
        
        // 3. 创建子执行器
        let executor = self.executor_factory.create(
            self.profiles.clone(),
            ctx.turn_id.clone(),
            ctx.event_writer.clone(),
        );
        
        // 4. 执行
        tracing::info!(
            "RunAgent: name='{}', task='{}', turn_id={}",
            params.name, params.task, ctx.turn_id
        );
        
        match executor.execute(params, ctx.cancel_token.clone()).await {
            Ok(result) => {
                // 5. 返回摘要 (对 LLM 友好)
                Ok(ToolExecutionResult::ok(&result.summary))
            }
            Err(e) => {
                Ok(ToolExecutionResult::error(&format!("子 Agent 执行失败: {}", e)))
            }
        }
    }

    fn name(&self) -> &str {
        "runAgent"
    }
}

/// 子执行器工厂 trait (用于解耦)
#[async_trait]
pub trait SubAgentExecutorFactory: Send + Sync {
    fn create(
        &self,
        profiles: Arc<AgentProfileRegistry>,
        parent_turn_id: String,
        event_writer: Arc<dyn EventLogWriter>,
    ) -> SubAgentExecutor;
}
```

#### 2.5 注册 RunAgentTool 到 Capability Router

**修改**: `crates/runtime/src/bootstrap.rs` (或 tool-loader 相关文件)

在内置工具注册处添加:

```rust
use runtime_agent_tool::RunAgentTool;

// 在 create_builtin_router 或类似函数中
let run_agent_tool = RunAgentTool::new(
    bootstrap.agent_profiles.clone(),
    executor_factory,
);
router.register_tool("runAgent", Arc::new(run_agent_tool));
```

#### 2.6 前端事件投影适配

**修改**: `frontend/` 中的事件消费逻辑

子 Agent 事件需要渲染为嵌套结构:

```typescript
// 伪代码
if (event.type === 'SubAgentTurnStart') {
  // 显示一个折叠区域
  renderCollapsibleBlock({
    icon: '📦',
    title: `调用 Agent: ${event.agent_profile}`,
    loading: true,
  });
}

if (event.type === 'SubAgentTurnEnd') {
  // 更新折叠区域
  updateCollapsibleBlock({
    title: `✅ Agent ${event.agent_profile} (${event.outcome})`,
    content: event.summary,
    loading: false,
  });
}
```

**验收标准**:
- `cargo test --package runtime-agent-tool` 全部通过
- LLM 可以成功调用 `runAgent` 工具
- 子 Agent 的执行结果出现在 EventLog 中
- 前端可正确渲染嵌套 Agent 事件

---

## Phase 3: 扩展 REST API

**目标**: 添加完整的 REST API 端点, 支持 Agent 直接调用

**预估时间**: 2 天

### 步骤

#### 3.1 设置 Axum Router

**文件**: `crates/runtime-agent-api/src/lib.rs`

```rust
use axum::{Router, routing::{get, post}};
use tokio::net::TcpListener;
use std::sync::Arc;
use runtime::RuntimeService;
use runtime_agent_profiles::AgentProfileRegistry;

pub struct AgentApiConfig {
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub cors_origins: Vec<String>,
}

pub async fn start_server(
    config: AgentApiConfig,
    runtime: Arc<RuntimeService>,
    profiles: Arc<AgentProfileRegistry>,
) -> anyhow::Result<()> {
    let app = create_router(runtime, profiles, &config);
    
    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Agent API server starting on {}", addr);
    
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}

fn create_router(
    runtime: Arc<RuntimeService>,
    profiles: Arc<AgentProfileRegistry>,
    config: &AgentApiConfig,
) -> Router {
    let state = Arc::new(ApiState {
        runtime,
        profiles,
        api_key: config.api_key.clone(),
    });

    Router::new()
        // 健康检查
        .route("/health", get(health_check))
        // 会话 API
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", get(get_session).delete(delete_session))
        // 消息 API
        .route("/sessions/{id}/message", post(send_message_streaming))
        .route("/sessions/{id}/message/async", post(send_message_async))
        .route("/sessions/{id}/messages", get(get_messages))
        // Agent API
        .route("/agents", get(list_agents))
        .route("/agents/{id}/execute", post(execute_agent))
        // 工具 API
        .route("/tools", get(list_tools))
        // Swagger / OpenAPI
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        // Middleware
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(
                    config.cors_origins.iter()
                        .map(|o| o.parse().unwrap())
                        .collect::<Vec<_>>()
                )
                .allow_methods([http::Method::GET, http::Method::POST, http::Method::DELETE])
                .allow_headers([http::header::CONTENT_TYPE, http::header::AUTHORIZATION])
        )
        .layer(
            tower_http::limit::RequestBodyLimitLayer::new(10 * 1024 * 1024)  // 10MB
        )
        .with_state(state)
}

#[derive(Clone)]
struct ApiState {
    runtime: Arc<RuntimeService>,
    profiles: Arc<AgentProfileRegistry>,
    api_key: Option<String>,
}
```

#### 3.2 实现核心端点

**文件**: `crates/runtime-agent-api/src/routes/sessions.rs`

```rust
pub async fn send_message_streaming(
    State(state): State<Arc<ApiState>>,
    Path(session_id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> Result<impl IntoResponse> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    
    // 启动异步 prompt 执行
    let rt = state.runtime.clone();
    let sid = session_id.clone();
    tokio::spawn(async move {
        match rt.prompt(&sid, req.into(), tx).await {
            Ok(_) => {},
            Err(e) => {
                tx.send(AgentEvent::Error { message: e.to_string() }).ok();
            }
        }
    });
    
    // 返回 SSE 流
    let stream = event_source(rx);
    Ok(Sse::new(stream))
}
```

**文件**: `crates/runtime-agent-api/src/routes/agents.rs`

```rust
/// GET /agents - 列出可用 Agent Profile
pub async fn list_agents(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<Vec<AgentInfoResponse>>> {
    let agents = state.profiles.list().iter().map(|(id, p)| {
        AgentInfoResponse {
            id: id.0.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            allowed_tools: p.allowed_tools.clone().unwrap_or_default(),
            max_steps: p.max_steps,
        }
    }).collect();
    
    Ok(Json(agents))
}

/// POST /agents/{id}/execute - 直接执行 Agent
pub async fn execute_agent(
    State(state): State<Arc<ApiState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<AgentExecuteRequest>,
) -> Result<impl IntoResponse> {
    let profile = state.profiles.get(&AgentProfileId(agent_id))
        .ok_or_else(|| not_found("Agent Profile not found"))?;
    
    // 创建独立 session 或使用现有 session
    let session_id = state.runtime.create_session(&req.working_dir).await?;
    
    // 执行
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let rt = state.runtime.clone();
    let sid = session_id.clone();
    
    // 构建仅包含任务的消息
    let prompt = PromptRequest {
        message: req.task,
        ..Default::default()
    };
    
    tokio::spawn(async move {
        rt.prompt(&sid, prompt, tx).await.ok();
    });
    
    let stream = event_source(rx);
    Ok(Sse::new(stream))
}
```

#### 3.3 OpenAPI 文档

**文件**: `crates/runtime-agent-api/src/openapi.rs`

```rust
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    paths(
        sessions::list_sessions,
        sessions::create_session,
        sessions::send_message_streaming,
        sessions::send_message_async,
        agents::list_agents,
        agents::execute_agent,
    ),
    components(schemas(
        MessageRequest,
        MessagePart,
        AgentInfoResponse,
        AgentExecuteRequest,
        SessionResponse,
    )),
    tags(
        (name = "sessions", description = "会话管理"),
        (name = "agents", description = "Agent 执行"),
    )
)]
pub struct ApiDoc;
```

**验收标准**:
- `cargo run --package runtime-agent-api` 启动服务
- `GET /health` → 200 OK
- `GET /swagger-ui` → 打开 Swagger
- `POST /sessions/{id}/message` → SSE 流式响应
- `POST /agents/{id}/execute` → SSE 流式响应

---

## Phase 4: WebSocket 实时通信

**目标**: 实现 WebSocket 双向通信, 支持实时交互

**预估时间**: 1 天

### 步骤

#### 4.1 WebSocket Handler

**文件**: `crates/runtime-agent-api/src/ws/handler.rs`

```rust
use axum::{
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
};
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<ApiState>) {
    let (mut sender, mut receiver) = socket.split();
    
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WsServerEvent>();
    
    // 客户端消息处理 (receive loop)
    let send_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                let client_msg: Result<ClientMessage, _> = serde_json::from_str(&text);
                match client_msg {
                    Ok(msg) => handle_client_message(msg, &state, &send_tx).await,
                    Err(e) => {
                        send_tx.send(WsServerEvent::Error { 
                            message: format!("JSON parse error: {}", e) 
                        }).ok();
                    }
                }
            }
        }
    });
    
    // 服务端消息发送 (send loop)
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let json = serde_json::to_string(&event).unwrap();
            if sender.send(Message::Text(json)).await.is_err() {
                break;  // 连接已关闭
            }
        }
    });
}

async fn handle_client_message(
    msg: ClientMessage,
    state: &Arc<ApiState>,
    tx: &tokio::sync::mpsc::UnboundedSender<WsServerEvent>,
) {
    match msg {
        ClientMessage::Subscribe { session_id } => {
            // 订阅会话的事件流
            let events = state.runtime.get_events(&session_id);
            for evt in events {
                tx.send(WsServerEvent::Event(evt)).ok();
            }
        }
        ClientMessage::SendMessage { session_id, content } => {
            // 发送消息
            let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
            
            let rt = state.runtime.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                let prompt = PromptRequest { message: content, ..Default::default() };
                rt.prompt(&sid, prompt, event_tx).await.ok();
            });
            
            while let Some(evt) = event_rx.recv().await {
                if tx.send(WsServerEvent::Event(evt)).is_err() {
                    break;
                }
            }
            tx.send(WsServerEvent::TurnComplete { session_id }).ok();
        }
        ClientMessage::Abort { session_id } => {
            state.runtime.abort(&session_id);
            tx.send(WsServerEvent::Aborted { session_id }).ok();
        }
    }
}
```

#### 4.2 WS 消息协议

**文件**: `crates/runtime-agent-api/src/ws/protocol.rs`

```rust
use serde::{Deserialize, Serialize};

/// 客户端发送到服务端的消息
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// 订阅会话事件
    Subscribe { session_id: String },
    /// 发送消息
    SendMessage { session_id: String, content: String },
    /// 中止执行
    Abort { session_id: String },
}

/// 服务端发送到客户端的消息
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerEvent {
    /// 事件流 (StorageEvent 的投影)
    Event(StorageEvent),
    /// Turn 完成
    TurnComplete { session_id: String },
    /// 执行被中止
    Aborted { session_id: String },
    /// 错误
    Error { message: String },
}
```

#### 4.3 路由集成

**修改**: `crates/runtime-agent-api/src/lib.rs`

```rust
use crate::ws::handler::ws_handler;

Router::new()
    // ... existing routes
    .route("/ws", get(ws_handler))
```

**验收标准**:
- 可使用 `wscat -c ws://localhost:6543/ws` 连接
- 发送 `{"type": "subscribe", "session_id": "xxx"}` 接收事件
- 发送 `{"type": "send_message", ...}` 触发 Agent 执行
- 发送 `{"type": "abort", ...}` 中止执行

---

## Phase 5: 前端集成 (可选)

**目标**: 前端适配新的 Agent as Tool 功能

**预估时间**: 1 天

### 步骤

#### 5.1 事件类型扩展

修改 `frontend/src/types.ts`:

```typescript
export interface SubAgentTurnStartEvent {
  type: 'SubAgentTurnStart';
  sub_turn_id: string;
  parent_turn_id: string;
  agent_profile: string;
  task: string;
  max_steps: number;
}

export interface SubAgentTurnEndEvent {
  type: 'SubAgentTurnEnd';
  sub_turn_id: string;
  parent_turn_id: string;
  agent_profile: string;
  outcome: 'completed' | 'failed' | 'aborted' | 'token_exceeded';
  summary: string;
  token_usage?: TokenUsage;
}
```

#### 5.2 渲染组件

创建新的 `SubAgentBlock.tsx` 组件用于渲染嵌套 Agent 执行:

```tsx
// 伪代码
export function SubAgentBlock({ event }: SubAgentBlockProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const isLoading = event.type === 'SubAgentTurnStart';
  
  return (
    <div className="sub-agent-block">
      <button onClick={() => setIsExpanded(!isExpanded)}>
        {isLoading ? '⏳' : event.outcome === 'completed' ? '✅' : '❌'}
        调用 Agent: {event.agent_profile}
      </button>
      {isExpanded && (
        <pre>{event.summary}</pre>
      )}
    </div>
  );
}
```

#### 5.3 事件处理器

修改 `frontend/src/hooks/useAgent.ts`:

```typescript
// 在事件分发逻辑中
if (event.type === 'SubAgentTurnStart' || event.type === 'SubAgentTurnEnd') {
  // 添加到消息列表, 按 parent_turn_id 分组
  dispatch({ type: 'ADD_SUB_AGENT_EVENT', event });
}
```

**验收标准**:
- 前端可以正确渲染嵌套 Agent 事件
- 折叠/展开子 Agent 执行详情
- 显示进度指示器 (loading state)

---

## Phase 6: 测试与验证

**目标**: 完整测试所有功能

**预估时间**: 1 天

### 6.1 单元测试

```bash
# Agent Loader 测试
cargo test --package runtime-agent-loader

# Agent Tool 测试
cargo test --package runtime-agent-tool

# API 端点测试 (集成)
cargo test --package runtime-agent-api
```

### 6.2 集成测试

```bash
# 完整 E2E 测试
# 1. 创建 session
# 2. 发送消息触发 Agent 执行
# 3. Agent 调用 runAgent 子 Agent
# 4. 子 Agent 执行完毕
# 5. 验证事件序列正确
# 6. 验证摘要截断正确
```

### 6.3 API 测试

使用 `curl` 验证所有端点:

```bash
# 健康检查
curl http://localhost:6543/health

# 列出 Agent
curl http://localhost:6543/agents

# 发送消息 (SSE)
curl -N http://localhost:6543/sessions/session-123/message \
  -H "Content-Type: application/json" \
  -d '{"content": "分析 src/ 目录下的代码结构"}'

# 执行 Agent 任务 (SSE)
curl -N http://localhost:6543/agents/explore/execute \
  -H "Content-Type: application/json" \
  -d '{"task": "查找所有使用 X 的地方", "working_dir": "/path/to/project"}'
```

---

## 总工作量评估

| Phase | 内容 | 预估时间 |
|-------|------|----------|
| Phase 0 | 基础设施 | 0.5 天 |
| Phase 1 | Agent Profile 系统 | 1 天 |
| Phase 2 | Agent as Tool | 2 天 |
| Phase 3 | 扩展 REST API | 2 天 |
| Phase 4 | WebSocket | 1 天 |
| Phase 5 | 前端集成 | 1 天 |
| Phase 6 | 测试验证 | 1 天 |
| **总计** | | **8.5 天** |

---

## 风险评估与缓解

| 风险 | 影响 | 概率 | 缓解措施 |
|------|------|------|----------|
| AgentLoop 重构影响现有功能 | 高 | 中 | 子 Agent 使用独立代码路径, 不修改现有 turn_runner |
| Token 预算控制失效 | 高 | 低 | 在 SubAgentExecutor 中强制检查 |
| SSE 流泄漏 (连接断开) | 中 | 中 | 使用 mpsc 的 `try_send`, 断开时自动清理 |
| 策略引擎绕过 | 高 | 低 | SubAgentPolicyEngine 在 tool_cycle 前拦截 |
| API Key 管理不善 | 高 | 低 | 通过环境变量, 不硬编码 |
| WebSocket 并发冲突 | 中 | 低 | Axum 的 ws 实现已处理并发 |

---

## 后续扩展 (Phase 7+)

- [ ] D-Mail 时间旅行 (参考 Kimi-CLI)
- [ ] 安全沙箱 (参考 Codex)
- [ ] Auto-configure Agent (LLM 自行创建子 Agent Profile)
- [ ] 多工作空间路由
- [ ] MCP Server 集成
- [ ] 分布式 Agent 编排

---

## 验证命令速查

```bash
# 全量检查
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode

# 单个 package
cargo test --package runtime-agent-loader
cargo test --package runtime-agent-tool

# 运行 API 服务
cargo run --package runtime-agent-api

# 前端检查
cd frontend && npm run typecheck && npm run lint
```
