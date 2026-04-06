# AstrCode Architecture

## Three-Layer Architecture

### Layer 1: Immutable Core Contracts

`crates/protocol` + `crates/core`。只放"平台事实"，不放"产品选择"。

| 模块 | 位置 | 核心类型 | 职责 |
|------|------|---------|------|
| DTO 协议 | `crates/protocol/src/` | `http::*`, `plugin::*`, `transport::*` | 跨模块通信协议 |
| Tool | `crates/core/src/tool.rs` | `Tool` trait, `ToolContext`, `ToolDefinition`, `ToolExecutionResult` | 工具抽象接口 |
| Capability | `crates/core/src/capability.rs` | `CapabilityDescriptor`, `CapabilityKind`, `CapabilityInvoker`, `CapabilityContext` | 一等动作模型 |
| Policy | `crates/core/src/policy/` | `PolicyEngine`, `PolicyVerdict<T>` | 同步决策面 |
| Event | `crates/core/src/event/` | `AgentEvent`(观测), `StorageEvent`(持久化), `Phase`, `EventTranslator` | 异步观测面 |
| Agent | `crates/core/src/agent/` | `AgentProfile`, `SpawnAgentParams`, `AgentMode`, `SubRunHandle`, `SubRunResult`, `SubagentContextOverrides` | Agent 生命周期 DTO |
| Session | `crates/core/src/store.rs` + `event/mod.rs` | `SessionManager`, `SessionTurnLease`, `EventLogWriter`, `SessionMeta` | 持久化接口 |
| Hook | `crates/core/src/hook.rs` | `HookHandler`, `HookEvent`, `HookInput`, `HookOutcome` | 生命周期钩子接口 |
| Cancel | `crates/core/src/cancel.rs` | `CancelToken` | 取消令牌 |
| Plugin | `crates/core/src/plugin/` | `PluginManifest`, `PluginRegistry`, `PluginHealth` | 插件契约 |
| Runtime | `crates/core/src/runtime/` | `RuntimeHandle`, `ManagedRuntimeComponent`, `RuntimeCoordinator` | 运行时契约 |

### Layer 2: Runtime Assembly

| Crate | 核心入口 | 职责 |
|-------|---------|------|
| `storage` | `FileSystemSessionRepository` | JSONL 会话持久化实现 |
| `runtime-tool-loader` | 内置工具集 | `ReadFile`, `WriteFile`, `EditFile`, `ListDir`, `FindFiles`, `Grep`, `Shell` |
| `runtime-config` | `ConfigManager` | 配置加载/保存/验证、API Key 解析、模型选择回退 |
| `runtime-llm` | `LlmProvider` trait | Anthropic + OpenAI 兼容 API、SSE 流式、指数退避重试 |
| `runtime-prompt` | `PromptComposer` | 贡献者模式、拓扑排序、条件渲染、skill 索引摘要 |
| `runtime-skill-loader` | `SkillCatalog` | 技能发现解析、内置 skill(仅 `git-commit`)、目录扫描 |
| `runtime-registry` | `CapabilityRouter`, `ToolRegistry` | 能力路由 + 工具注册表实现 |
| `plugin` | `Supervisor` | 子进程管理、stdio JSON-RPC 双向通信、生命周期 |
| `sdk` | `ToolHandler`, `PluginContext` | 插件开发者 SDK |
| `runtime-session` | `SessionState` | 会话状态、token 预算跟踪、turn 执行引擎 |
| `runtime-execution` | `prepare_scoped_agent_execution()` | 执行上下文快照、策略验证、作用域装配 |
| `runtime-agent-control` | `AgentControl` | spawn/list/cancel/wait、parent-child 关系、深度/并发/GC 限制 |
| `runtime-agent-loader` | `AgentProfileLoader` | builtin(4) + 用户 + 项目 agent 定义加载 (Markdown/YAML) |
| `runtime-agent-loop` | `AgentLoop` | LLM 调用 + 工具执行主循环、审批、prompt/context/compaction |
| `runtime-agent-tool` | `SpawnAgentTool` | 把 `spawnAgent` 能力暴露为 LLM 工具 |
| `runtime` | `RuntimeService` | 门面：组合所有 runtime crate 提供统一 API |

#### RuntimeService 公开 API

```rust
pub struct RuntimeService {
    sessions: DashMap<String, Arc<SessionState>>,
    loop_: RwLock<Arc<AgentLoop>>,
    surface: RwLock<RuntimeSurfaceState>,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalBroker>,
    config: Mutex<Config>,
    session_manager: Arc<dyn SessionManager>,
    observability: Arc<RuntimeObservability>,
    agent_control: AgentControl,
    agent_loader: Arc<AgentProfileLoader>,
    agent_profiles: Arc<StdRwLock<Arc<AgentProfileRegistry>>>,
    // ...
}

impl RuntimeService {
    // 构造
    pub fn from_capabilities(CapabilityRouter) -> ServiceResult<Self>
    pub fn from_capabilities_with_prompt_inputs(...) -> ServiceResult<Self>

    // Loop 访问
    pub async fn current_loop(&self) -> Arc<AgentLoop>
    pub async fn replace_capabilities_with_prompt_inputs_and_hooks(...) -> ServiceResult<()>

    // 会话 CRUD
    pub async fn create_session(working_dir: &str) -> ServiceResult<SessionMeta>
    pub async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>>
    pub async fn load_session_history(&self, session_id: &str) -> ServiceResult<SessionHistorySnapshot>
    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()>
    pub async fn delete_project(&self, working_dir: &str) -> ServiceResult<DeleteProjectResult>

    // Turn 执行
    pub async fn submit_prompt(session_id: &str, text: &str) -> ServiceResult<PromptAccepted>
    pub async fn interrupt(&self, session_id: &str) -> ServiceResult<()>
    pub async fn compact_session(&self, session_id: &str) -> ServiceResult<()>

    // 配置
    pub async fn get_config(&self) -> Config
    pub async fn reload_config_from_disk(&self) -> ServiceResult<Config>
    pub async fn reload_agent_profiles_from_disk(&self) -> ServiceResult<Arc<AgentProfileRegistry>>
    pub async fn save_active_selection(profile: &str, model: &str) -> ServiceResult<()>
    pub async fn test_connection(profile: &str, model: &str) -> ServiceResult<TestResult>
    pub async fn open_config_in_editor(&self) -> ServiceResult<()>

    // 服务句柄
    pub fn agent_execution_service(self: &Arc<Self>) -> AgentExecutionServiceHandle
    pub fn tool_execution_service(self: &Arc<Self>) -> ToolExecutionServiceHandle

    // Agent 控制
    pub fn agent_control(&self) -> AgentControl
    pub fn agent_loader(&self) -> Arc<AgentProfileLoader>
    pub fn agent_profiles(&self) -> Arc<AgentProfileRegistry>

    // Composer
    pub async fn list_composer_options(&self, session_id: &str, request: ...) -> ServiceResult<Vec<ComposerOption>>

    // 事件
    pub fn subscribe_session_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent>

    // 可观测性
    pub fn observability_snapshot(&self) -> RuntimeObservabilitySnapshot
    pub fn loaded_session_count(&self) -> usize
    pub fn running_session_ids(&self) -> Vec<String>

    // 自动重载
    pub fn start_config_auto_reload(self: &Arc<Self>)
    pub fn start_agent_auto_reload(self: &Arc<Self>)

    // 关闭
    pub async fn shutdown(&self, timeout_secs: u64)
}
```

#### RuntimeService service/ 模块分组

| 模块组 | 文件 | 职责 |
|--------|------|------|
| 核心 | `mod.rs` (15k+行) + `service_contract.rs` | RuntimeService 结构体、`RuntimeHandle` trait 实现 |
| 会话 | `session/create.rs` | 创建/列出会话 |
| 会话 | `session/load.rs` | 加载会话快照/历史 |
| 会话 | `session/delete.rs` | 删除会话/项目 |
| Turn | `turn/submit.rs` | 提交 prompt / 中断 |
| Turn | `turn/branch.rs` | 自动分支逻辑 (忙时自动分支) |
| Turn | `turn/compact.rs` | 手动压缩 |
| 执行 | `execution/mod.rs` | `AgentExecutionServiceHandle` + `ToolExecutionServiceHandle` |
| 执行 | `execution/root.rs` | 根执行入口 |
| 执行 | `execution/subagent.rs` | 子 agent 执行 |
| 执行 | `status.rs` | Sub-run 状态查询 |
| 配置 | `config_manager.rs` / `config_ops.rs` | 配置快照读写、磁盘重载 |
| 监听 | `watch_manager.rs` / `watch_ops.rs` | 文件系统自动重载 (防抖) |
| 能力面 | `capabaility_manager.rs` | capability surface 与 loop 热替换 |
| 装配 | `loop_factory.rs` | AgentLoop 组装器 |
| 观测 | `observability.rs` | 原子计数器指标收集 |
| 回放 | `replay.rs` | `SessionReplaySource` trait 实现 |
| Composer | `composer_ops.rs` | 输入候选查询 |
| 辅助 | `blocking_bridge.rs` | async/blocking 桥接 |

#### AgentLoop 内部模块

| 模块 | 职责 |
|------|------|
| `agent_loop.rs` | `AgentLoop` 结构体 + `TurnOutcome` 枚举 |
| `agent_loop/turn_runner.rs` | Turn 编排主循环 (LLM → 工具 → LLM step 循环) |
| `agent_loop/llm_cycle.rs` | LLM provider 构建与调用 |
| `agent_loop/tool_cycle.rs` | 工具执行 (安全工具并发、unsafe 串行)、Policy 三态检查、审批 |
| `agent_loop/token_budget.rs` | Token 预算决策 |
| `context_pipeline.rs` | Context 构建管道 (7 Stage: Baseline/RecentTail/Workset/CompactionView/Recovery/PrunePass/BudgetTrim) |
| `compaction_runtime.rs` | 压缩运行时 (Policy + Strategy + Rebuilder 三件套) |
| `request_assembler.rs` | PromptPlan + ContextBundle → ModelRequest |
| `prompt_runtime.rs` | 桥接 PromptComposer |
| `context_window/compaction.rs` | 自动压缩逻辑 (Auto/Reactive/Manual 触发) |
| `context_window/prune_pass.rs` | 轻量裁剪 (截断长工具结果、清除旧工具结果) |
| `context_window/token_usage.rs` | Token 启发式估算 (4 chars/token) |
| `context_window/file_access.rs` | 文件访问跟踪 (用于恢复) |
| `approval_service.rs` | 审批代理 |
| `provider_factory.rs` | LLM Provider 工厂 |
| `hook_runtime.rs` | Hook 运行时 |
| `subagent.rs` | `ChildExecutionTracker` + `SubAgentPolicyEngine` |

### Layer 3: Transports

`crates/server` + `src-tauri` + `frontend`。对外暴露 runtime。

## HTTP API 端点

### 认证

| 端点 | 方法 | 说明 |
|------|------|------|
| `/__astrcode__/run-info` | GET | Bootstrap 信息 (不认证) |
| `/api/auth/exchange` | POST | Bootstrap token → Session token |

### 会话

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/sessions` | POST | 创建会话 |
| `/api/sessions` | GET | 列出会话 |
| `/api/sessions/{id}` | DELETE | 删除会话 |
| `/api/sessions/{id}/prompts` | POST | 提交 prompt (202 异步执行) |
| `/api/sessions/{id}/interrupt` | POST | 中断执行 |
| `/api/sessions/{id}/compact` | POST | 手动压缩 |
| `/api/sessions/{id}/history` | GET | 会话历史完整事件 |
| `/api/sessions/{id}/events` | GET (SSE) | 事件流，支持 `?afterEventId=` 与 `?subRunId=&scope=` 过滤 |
| `/api/sessions/{id}/composer/options` | GET | Composer 输入候选 |
| `/api/session-events` | GET (SSE) | 全局目录事件 (创建/删除/分支) |
| `/api/projects` | DELETE | 删除整个项目 (?workingDir=) |

### 配置

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/config` | GET | 查看配置 |
| `/api/config/reload` | POST | 从磁盘重载配置 |
| `/api/config/active-selection` | POST | 保存 active profile/model |

### 模型

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/models` | GET | 列出可用模型 |
| `/api/models/current` | GET | 当前模型信息 |
| `/api/models/test` | POST | 测试模型连接 |

### 运行时

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/runtime/plugins` | GET | 插件状态 |
| `/api/runtime/plugins/reload` | POST | 重载插件 |

### v1 API

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/v1/agents` | GET | 列出 Agent Profiles |
| `/api/v1/agents/{id}/execute` | POST | 创建 root execution |
| `/api/v1/sessions/{id}/subruns/{sub_run_id}` | GET | 查询子 run 状态 |
| `/api/v1/tools` | GET | 列出工具 |
| `/api/v1/tools/{id}/execute` | POST | 501 Not Implemented (未启用) |

## 配置模型

```rust
pub struct Config {
    pub version: String,                 // "1"
    pub active_profile: String,          // 默认 "deepseek"
    pub active_model: String,            // 默认 "deepseek-chat"
    pub runtime: RuntimeConfig,
    pub profiles: Vec<Profile>,
}

pub struct RuntimeConfig {
    // 工具并发 (默认 10, 可被 ASTRCODE_MAX_TOOL_CONCURRENCY 覆盖)
    pub max_tool_concurrency: Option<usize>,
    // 自动压缩 (默认 true)
    pub auto_compact_enabled: Option<bool>,
    pub compact_threshold_percent: Option<u8>,    // 默认 90
    pub compact_keep_recent_turns: Option<u8>,   // 默认 4
    pub tool_result_max_bytes: Option<usize>,      // 默认 100_000
    // Token 预算 (默认 0 = 禁用)
    pub default_token_budget: Option<u64>,
    pub continuation_min_delta_tokens: Option<usize>, // 默认 500
    pub max_continuations: Option<u8>,                // 默认 3
    // Agent 控制
    pub agent: Option<AgentConfig>,
    // LLM 客户端
    pub llm_connect_timeout_secs: Option<u64>,     // 默认 10
    pub llm_read_timeout_secs: Option<u64>,        // 默认 90
    // ... 更多高级配置选项
}

pub struct AgentConfig {
    pub max_subrun_depth: Option<usize>,           // 默认 1
    pub max_depth: Option<usize>,                  // 兼容旧值，默认 3
    pub max_concurrent: Option<usize>,             // 默认 10
    pub finalized_retain_limit: Option<usize>,     // 默认 256
    pub experimental_independent_session: Option<bool>, // 默认 false
}

pub struct Profile {
    pub name: String,
    pub provider_kind: String,         // "openai-compatible" 或 "anthropic"
    pub base_url: String,
    pub api_key: Option<String>,       // 支持 env:/literal: 前缀
    pub models: Vec<ModelConfig>,
}
```

**配置路径**：
- 用户级：`~/.astrcode/config.json`
- 项目级 overlay：`<project>/.astrcode/config.json` (runtime 配置不被 overlay)

## 认证模型

两层 Token：

1. **Bootstrap Token** (24h) — server 启动时生成 `random_hex_token()` (64字符)，写入 `~/.astrcode/run.json`，用于首次握手
2. **Session Token** (8h) — `POST /api/auth/exchange` 交换后签发，后续所有 API 请求通过 `x-astrcode-token` header 或 SSE 场景下 `?token=` query param 注入

## Bootstrap 时序

```
main() → bootstrap_runtime() → RuntimeBootstrap { service, coordinator, governance }
  → 绑定 127.0.0.1:0 (随机端口)
  → 生成 bootstrap token (24h TTL, 64字符)
  → 写入 ~/.astrcode/run.json (port, token, pid, startedAt, expiresAtMs)
  → 加载 frontend/dist/ (如存在) → 注入 window.__ASTRCODE_BOOTSTRAP__ 到 index.html
  → 构建 Axum Router + CORS (允许 localhost:5173)
  → 启动 HTTP server + graceful shutdown (SIGTERM/Ctrl+C/stdin关闭)
```

## 存储模型

- **事件存储**：`~/.astrcode/projects/<hash>/sessions/<session-id>/session-*.jsonl`
- **JSONL 格式**：`StoredEvent { storage_seq, event: StorageEvent }` (append-only)
- **会话 turn 锁**：跨进程文件锁 (`fs2`)
- **SSE 事件 id**：`{storage_seq}.{subindex}`
- **广播**：`broadcast::Sender<SessionEventRecord>` 容量 2048
- **LRU 缓存**：`recent_records` / `recent_stored` 各 4096 条

## 四大核心契约

### 1. AgentLoop Contract

Turn 是基本调度单位。AgentLoop 按 turn 调度 (每 turn 包含多个 step 循环: LLM → 工具执行 → LLM ...)。

```
step 循环 {
    1. 取消检查
    2. ContextPipeline.build_bundle()     → ContextBundle (7-stage pipeline)
    3. PromptRuntime.build_plan()          → PromptPlan
    4. RequestAssembler.build_step_request()  → ModelRequest + TokenSnapshot
    5. Policy.decide_context_strategy() → maybe compact
    6. Policy.check_model_request()     → call LLM
    7. if 413 → reactive compact (最多 3 次)
    8. if max_tokens → auto-continue nudge (最多 3 次)
    9. if no tool_calls → Complete
    10. if tool_calls → execute (安全工具并发, unsafe 串行) → step_index++
}
```

```rust
pub enum TurnOutcome {
    Completed,   // LLM 返回纯文本，自然结束
    Cancelled,   // 用户取消或 CancelToken 触发
    Error { message: String },
}
```

### 2. Capability Contract

Capability 是唯一一等动作模型。`CapabilityRouter` 按名称路由到 `CapabilityInvoker`。

```rust
pub enum CapabilityKind {
    tool(), agent(), context_provider(), memory_provider(),
    policy_hook(), renderer(), resource(), prompt(),
}
```

### 3. Policy Contract

```rust
pub enum PolicyVerdict<T> {
    Allow(T),
    Deny { reason: String },
    Ask(Box<ApprovalPending<T>>),
}
```

三个决策点：`check_model_request`、`check_capability_call`、`decide_context_strategy`。
子 Agent 的 `Ask` 被 `SubAgentPolicyEngine` 强制转为 `Deny`。

### 4. Event Contract

两类事件通过 `EventTranslator` 互相投影：
- `AgentEvent` — SSE 推送前端
- `StorageEvent` — JSONL 持久化
