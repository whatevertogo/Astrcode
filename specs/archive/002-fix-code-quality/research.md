# Research: 修复项目代码质量问题

**Feature**: 002-fix-code-quality  
**Date**: 2026-04-08  
**Status**: Complete

本文档记录所有技术研究结果和设计决策，解决 plan.md 中的 NEEDS CLARIFICATION。

---

## 1. Plugin 类型迁移策略 (US2)

### 问题陈述

当前 `core` 依赖 `protocol` 违反宪法 1.2.0 Architecture Constraints（core 和 protocol 必须双向独立）。根据 spec Clarification，Plugin 是核心扩展机制，应属于 core。

### 研究发现

#### 需要迁移的类型（从 protocol 移入 core）

**Descriptor 类型族**（11 个）：
- `CapabilityDescriptor` - 能力的完整描述
- `CapabilityDescriptorBuilder` - Builder 模式构造器
- `CapabilityKind` - 能力类型枚举（tool/agent/context_provider）
- `PeerDescriptor` - 通信对等方身份
- `PeerRole` - 对等方角色枚举
- `HandlerDescriptor` - 事件处理器描述
- `TriggerDescriptor` - 触发器描述符
- `FilterDescriptor` - 过滤器描述符
- `ProfileDescriptor` - Profile 描述符
- `SkillDescriptor` - Skill 声明描述符
- `SkillAssetDescriptor` - Skill 资产文件描述符

**元数据类型**（4 个）：
- `SideEffectLevel` - 副作用级别枚举
- `StabilityLevel` - 稳定性级别枚举
- `PermissionHint` - 权限提示
- `DescriptorBuildError` - 描述符构建错误

**调用上下文类型**（4 个）：
- `InvocationContext` - 调用上下文
- `CallerRef` - 调用方引用
- `WorkspaceRef` - 工作区引用
- `BudgetHint` - 预算提示

#### 保留在 protocol 的类型（传输 DTO）

- `InitializeMessage` - 握手消息
- `InitializeResultData` - 握手结果
- `InvokeMessage` - 调用消息
- `EventMessage` - 事件消息
- `ResultMessage` - 结果消息
- `CancelMessage` - 取消消息
- `PluginMessage` - 消息 enum

#### Caller Inventory（受影响的模块）

**Core 层**（3 个文件）：
- `core/src/capability.rs` - 当前从 protocol re-export，迁移后成为原生类型
- `core/src/plugin/manifest.rs` - PluginManifest.capabilities 字段类型变更
- `core/src/plugin/registry.rs` - PluginEntry.capabilities 字段类型变更

**SDK 层**（3 个文件）：
- `sdk/src/lib.rs` - re-export 来源从 protocol 变为 core
- `sdk/src/tool.rs` - ToolHandler trait 返回类型变更
- `sdk/src/hook.rs` - PolicyHook trait 参数类型变更

**Plugin 层**（7 个文件）：
- `plugin/src/capability_router.rs` - CapabilityHandler trait 参数类型变更
- `plugin/src/invoker.rs` - PluginCapabilityInvoker.descriptor 字段类型变更
- `plugin/src/loader.rs` - PeerDescriptor 保留在 protocol（传输 DTO）
- `plugin/src/supervisor.rs` - 同上
- `plugin/src/worker.rs` - 同上
- `plugin/src/bin/fixture_worker.rs` - 测试 fixture，需同步更新

**Runtime 层**（3 个文件）：
- `runtime/src/runtime_surface_assembler.rs` - host_peer_descriptor() 返回类型变更
- `runtime/src/plugin_hook_adapter.rs` - HandlerDescriptor 保留在 protocol
- `runtime/src/plugin_skill_materializer.rs` - SkillDescriptor 保留在 protocol

**Runtime-Agent-Loop 层**（6 个文件）：
- `runtime-agent-loop/src/agent_loop.rs` - capability_descriptors() 返回类型变更
- `runtime-agent-loop/src/approval_service.rs` - ApprovalRequest.capability 字段类型变更
- `runtime-agent-loop/src/context_pipeline.rs` - capability_descriptors 字段类型变更
- `runtime-agent-loop/src/context_window/prune_pass.rs` - descriptors 字段类型变更
- `runtime-agent-loop/src/prompt_runtime.rs` - capability_descriptors 字段类型变更
- `runtime-agent-loop/src/agent_loop/tests/plugin.rs` - 测试代码，PeerDescriptor 保留在 protocol

**Server 层**（2 个文件）：
- `server/src/http/mapper.rs` - to_runtime_capability_dto() 参数类型变更
- `server/src/tests/runtime_routes_tests.rs` - 测试代码，需同步更新

### 决策：完全迁移策略

**Rationale**：
- Plugin 类型是核心扩展机制，属于"稳定契约与共享领域类型"（宪法定义）
- protocol 应只包含传输 DTO，不应定义领域模型
- 迁移后 core 和 protocol 实现真正的双向独立

**Mapper 设计**：
- 在 `plugin` crate 中实现 `mapper.rs`（protocol DTO ↔ core 领域类型）
- 握手阶段：protocol DTO → core 类型
- 调用阶段：core 类型 → protocol DTO

**迁移顺序**（5 个阶段）：
1. **准备阶段**：在 core 中创建 Plugin 模块，复制类型定义，在 protocol 中创建 re-export 层（向后兼容）
2. **迁移 SDK 和 Plugin 层**：更新导入，创建 Plugin Mapper
3. **迁移 Runtime 层**：更新 Runtime 和 Runtime-Agent-Loop 导入
4. **迁移 Server 层**：更新 Server 导入
5. **清理和验证**：移除 protocol re-export，更新 Cargo.toml 依赖，运行测试

**Alternatives considered**：
- 保留在 protocol：违反宪法，protocol 不应定义领域模型
- 部分迁移：会导致类型分散，增加维护成本

---

## 2. 锁恢复机制推广 (US3)

### 问题陈述

`runtime-session` 已有 `with_lock_recovery` 实现，需推广到其他 crate。宪法 VI 要求所有锁获取必须使用恢复机制，不得 panic。

### 研究发现

#### with_lock_recovery 实现分析

**位置**：`crates/runtime-session/src/support.rs`

**核心逻辑**：
```rust
pub fn with_lock_recovery<T, R>(
    mutex: &StdMutex<T>,
    name: &'static str,
    update: impl FnOnce(&mut T) -> R,
) -> R {
    match mutex.lock() {
        Ok(mut guard) => update(&mut guard),
        Err(poisoned) => {
            log::error!("mutex '{name}' was poisoned; recovering inner state");
            let mut guard = poisoned.into_inner();  // 恢复内部状态
            let result = update(&mut guard);
            mutex.clear_poison();                    // 清除毒化标记
            result
        },
    }
}
```

**优点**：
- 使用 `poisoned.into_inner()` 恢复数据，不会 panic
- 记录错误日志便于诊断
- 通过闭包避免 guard 生命周期问题
- 设计优雅，易于使用

**配套函数**：
- `lock_anyhow()` - 返回 `Result<StdMutexGuard>`，将毒化错误转换为 `AstrError::LockPoisoned`

#### tokio Mutex 处理策略

**结论**：tokio Mutex 不需要恢复机制

**Rationale**：
- tokio Mutex 设计上不会 poison（即使任务 panic 也不会毒化）
- 失败模式主要是任务被 cancel 或 runtime 关闭
- 这些情况下无法恢复，应该让错误传播
- 保持现状，使用 `.lock().await` 直接获取

#### 需要修复的位置清单

**生产代码**（优先级 P1）：
- `plugin/src/peer.rs:300` - `.lock().unwrap()` → 改用 `with_lock_recovery()`
- `server/src/http/auth.rs:99` - `.lock().expect("auth token lock poisoned")` → 改用 `lock_anyhow()?`

**测试代码**（优先级 P3，共 47 处）：
- `runtime-agent-loop/src/agent_loop/tests/` - 26 处
- `runtime-agent-tool/src/lib.rs` - 2 处
- `runtime-llm/src/` - 5 处
- `runtime/src/` - 8 处
- `sdk/src/tests.rs` - 2 处
- `core/src/runtime/coordinator.rs` - 1 处
- `runtime-agent-loop/src/context_pipeline.rs` - 2 处

### 决策：统一 API 设计

**Rationale**：
- 不需要 trait 抽象，直接使用函数更简单
- 在 `astrcode-core` 中创建 `support` 模块，导出 `lock_anyhow` 和 `with_lock_recovery`
- 各 crate 通过 `use astrcode_core::support::*` 导入

**函数签名**：
```rust
// 用于需要返回 Result 的场景
pub fn lock_anyhow<'a, T>(
    mutex: &'a StdMutex<T>,
    name: &'static str,
) -> Result<StdMutexGuard<'a, T>>

// 用于不需要返回 Result 的场景（自动恢复）
pub fn with_lock_recovery<T, R>(
    mutex: &StdMutex<T>,
    name: &'static str,
    update: impl FnOnce(&mut T) -> R,
) -> R
```

**使用示例**：
```rust
// 场景 1：需要错误处理
let guard = lock_anyhow(&self.tokens, "auth tokens")?;

// 场景 2：自动恢复（不 panic）
with_lock_recovery(&self.phase, "session phase", |phase| {
    *phase = Phase::Idle;
})
```

**推广策略**（4 步）：
1. 在 `astrcode-core/src/support.rs` 中定义函数（或从 runtime-session re-export）
2. 修复生产代码（plugin、server）
3. 逐步修复其他 crate（runtime-agent-tool、runtime-agent-loop、runtime-llm）
4. 测试代码优化（可选）

**Alternatives considered**：
- Trait 抽象：增加复杂度，不如直接函数简单
- 全局管理器：不需要，各模块自行使用即可

---

## 3. JoinHandle 管理模式 (US4)

### 问题陈述

4 处 fire-and-forget `tokio::spawn` 需要保存 handle 和取消机制。宪法 VI 要求所有异步任务必须有生命周期管理。

### 研究发现

#### Fire-and-Forget Spawn 清单

| 文件:行号 | Spawn 的任务 | 生命周期归属 | 当前状态 |
|---------|-----------|----------|---------|
| `runtime/src/bootstrap.rs:238` | 后台插件装配与能力更新 | RuntimeBootstrap | ❌ 未保存 |
| `runtime/src/service/execution/mod.rs:197` | 会话 turn 执行（非根执行） | ExecutionService | ❌ 未保存 |
| `runtime/src/service/execution/root.rs:168` | 根执行 turn 运行 | AgentExecutionServiceHandle | ❌ 未保存 |
| `runtime/src/service/execution/subagent.rs:128` | 子 Agent 循环执行 | SubagentExecutionService | ❌ 未保存 |
| `runtime/src/service/watch_manager.rs:28` | 配置文件热重载监听 | WatchManager | ❌ 未保存（仅标记 flag） |
| `runtime/src/service/watch_manager.rs:46` | Agent 配置热重载监听 | WatchManager | ❌ 未保存（仅标记 flag） |

**已正确管理的 spawn**（参考实现）：
- `plugin/src/peer.rs:293` - 读循环 JoinHandle 保存在 `Mutex<Option<JoinHandle<()>>>`
- `plugin/src/peer.rs:577` - invoke 处理 JoinHandle 保存在 `Mutex<HashMap<String, JoinHandle<()>>>`

#### 取消机制分析

**项目中已有的取消机制**：

1. **CancelToken**（core crate）
   - 实现：`Arc<AtomicBool>` + `SeqCst` 排序
   - 特点：轻量级、跨线程共享、不依赖 tokio
   - 用途：工具执行、LLM 请求、长时间运行操作的取消

2. **CancellationToken**（tokio_util）
   - 用途：运行时全局关闭信号（`shutdown_token`）
   - 特点：支持 `cancel()` 和 `cancelled()` 等高级操作

**推荐方案**：
- 工具/执行层：继续使用 `CancelToken`（已在 core 中定义）
- 运行时关闭：继续使用 `CancellationToken`（已在 RuntimeService 中使用）
- 后台任务：使用 `CancelToken` 或通过 JoinHandle 的 `abort()` 方法

#### 持锁 await 分析

**结论**：当前持锁 await 都是安全的

**Rationale**：
- 大多数使用 `std::sync::Mutex`（同步）
- 异步 Mutex 的持锁时间都很短，无嵌套 await
- 无需修复

### 决策：模块自管理策略

**Rationale**：
- 不需要 TaskHandle 包装类，直接使用 `Mutex<Option<JoinHandle<()>>>` 或 `DashMap<String, JoinHandle<()>>`
- 不需要全局管理器，各模块自行管理其 spawn 的任务
- 关闭流程清晰：RuntimeService::shutdown() → 各模块的 stop/cancel 方法

**管理模式**：

1. **ExecutionTaskManager**（管理 turn 执行）：
```rust
pub struct ExecutionTaskManager {
    active_turns: DashMap<String, tokio::task::JoinHandle<()>>,
}

impl ExecutionTaskManager {
    pub fn spawn_turn(&self, turn_id: String, task: impl Future<Output = ()> + Send + 'static) {
        let handle = tokio::spawn(task);
        self.active_turns.insert(turn_id, handle);
    }
    
    pub async fn cancel_turn(&self, turn_id: &str) -> bool {
        self.active_turns
            .remove(turn_id)
            .map(|(_, handle)| handle.abort())
            .is_some()
    }
    
    pub async fn shutdown(&self) {
        for (_, handle) in self.active_turns.iter() {
            handle.abort();
        }
        self.active_turns.clear();
    }
}
```

2. **SubagentTaskManager**（管理子 Agent 执行）：
```rust
pub struct SubagentTaskManager {
    active_children: DashMap<String, tokio::task::JoinHandle<()>>,
    child_cancel_tokens: DashMap<String, CancelToken>,
}

impl SubagentTaskManager {
    pub fn spawn_child(&self, sub_run_id: String, cancel: CancelToken, task: impl Future<Output = ()> + Send + 'static) {
        let handle = tokio::spawn(task);
        self.active_children.insert(sub_run_id.clone(), handle);
        self.child_cancel_tokens.insert(sub_run_id, cancel);
    }
    
    pub async fn cancel_child(&self, sub_run_id: &str) {
        if let Some(cancel) = self.child_cancel_tokens.get(sub_run_id) {
            cancel.cancel();
        }
        if let Some((_, handle)) = self.active_children.remove(sub_run_id) {
            handle.abort();
        }
    }
}
```

3. **WatchManager**（管理配置监听）：
```rust
pub struct WatchManager {
    runtime: Arc<RuntimeService>,
    config_watch_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    agent_watch_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl WatchManager {
    pub async fn shutdown(&self) {
        if let Some(handle) = self.config_watch_handle.lock().await.take() {
            handle.abort();
        }
        if let Some(handle) = self.agent_watch_handle.lock().await.take() {
            handle.abort();
        }
    }
}
```

4. **PluginLoadHandle**（管理插件装配）：
```rust
pub struct PluginLoadHandle {
    task_handle: Option<tokio::task::JoinHandle<()>>,
    state: Arc<RwLock<PluginLoadState>>,
    completed_notify: Arc<Notify>,
}

impl PluginLoadHandle {
    pub async fn wait_completion(&self) {
        self.completed_notify.notified().await;
    }
    
    pub async fn cancel(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
    }
}
```

**实施优先级**：
1. **高优先级**（影响系统稳定性）：
   - `bootstrap.rs:238` - 插件装配（需要等待完成）
   - `root.rs:168` & `mod.rs:197` - Turn 执行（需要追踪和取消）

2. **中优先级**（影响子系统管理）：
   - `subagent.rs:128` - 子 Agent 执行（需要级联取消）

3. **低优先级**（优雅关闭）：
   - `watch_manager.rs:28 & 46` - 配置监听（可以通过 flag 控制）

**Alternatives considered**：
- 全局管理器：增加复杂度，不如模块自管理清晰
- TaskHandle 包装类：不需要，直接使用 tokio JoinHandle 更透明

---

## 4. 错误统一策略 (US5)

### 问题陈述

6 个 crate 各自定义错误类型，需与 `AstrError` 兼容。

### 研究发现

#### 自定义错误类型清单

**当前状态**：
- `ProtocolError` - protocol crate
- `StorageError` - storage crate
- `PluginError` - plugin crate
- `ConfigError` - runtime-config crate
- `RegistryError` - runtime-registry crate
- `AgentLoopError` - runtime-agent-loop crate

**问题**：
- 各 crate 错误类型不兼容
- 上层调用者需要处理多种错误类型
- `map_err(|_| ...)` 丢弃原始错误上下文（至少 1 处：`src-tauri/src/main.rs:424`）

### 决策：AstrError 扩展方案

**Rationale**：
- 保留各 crate 自有错误类型（领域特定）
- 在 `AstrError` 中新增对应 variant + `#[source]` 保留原始错误
- 实现 `From<T>` for `AstrError` 自动转换

**AstrError 扩展设计**：
```rust
// crates/core/src/error.rs

#[derive(Debug, thiserror::Error)]
pub enum AstrError {
    // 现有 variants...
    
    #[error("protocol error")]
    Protocol {
        #[source]
        inner: ProtocolError,
    },
    
    #[error("storage error")]
    Storage {
        #[source]
        inner: StorageError,
    },
    
    #[error("plugin error")]
    Plugin {
        #[source]
        inner: PluginError,
    },
    
    #[error("config error")]
    Config {
        #[source]
        inner: ConfigError,
    },
    
    #[error("registry error")]
    Registry {
        #[source]
        inner: RegistryError,
    },
    
    #[error("agent loop error")]
    AgentLoop {
        #[source]
        inner: AgentLoopError,
    },
}

// 自动转换
impl From<ProtocolError> for AstrError {
    fn from(err: ProtocolError) -> Self {
        AstrError::Protocol { inner: err }
    }
}

// ... 其他 From 实现
```

**错误转换契约**：
- 每个 crate 的自定义错误类型 MUST 实现 `Into<AstrError>`
- 转换 MUST 保留原始错误信息（通过 `#[source]` 或 `error.to_string()`）
- `map_err` MUST NOT 使用 `|_|` 丢弃原始错误

**Alternatives considered**：
- 全部合并到 AstrError：会导致 AstrError 过于庞大，失去领域特定性
- 不统一：上层调用者难以统一处理

---

## 5. 模块拆分策略 (US7)

### 问题陈述

`service/mod.rs`（17,000 行）和 `execution/mod.rs`（14,000 行）需拆分到 ≤800 行/文件。

### 研究发现

#### 当前模块结构

**service/mod.rs**（约 17,000 行）：
- session 创建、加载、删除
- turn 提交、中断
- 历史回放
- compaction
- 工具执行
- 子 Agent 执行
- 配置热重载
- Agent 热重载

**execution/mod.rs**（约 14,000 行）：
- 根执行编排
- 子执行编排
- 状态查询
- 取消控制

### 决策：两阶段拆分策略

**Rationale**：
- 先完成 US1-US4（编译、架构、panic、并发），稳定后再拆分
- 避免同时做边界重构和模块拆分两件大事
- 拆分时按职责边界，避免循环依赖

**拆分方案**（Phase 1 - service/mod.rs）：
```
service/
├── mod.rs              # 门面和装配（≤800 行）
├── session/
│   ├── create.rs       # session 创建
│   ├── load.rs         # session 加载
│   ├── delete.rs       # session 删除
│   └── catalog.rs      # session 目录
├── execution/
│   ├── root.rs         # 根执行
│   ├── subagent.rs     # 子 Agent 执行
│   ├── status.rs       # 状态查询
│   └── cancel.rs       # 取消控制
├── turn/
│   ├── submit.rs       # turn 提交
│   ├── interrupt.rs    # turn 中断
│   └── replay.rs       # 历史回放
├── watch_ops.rs        # 配置和 Agent 热重载
└── compaction.rs       # compaction
```

**拆分方案**（Phase 2 - execution/mod.rs）：
```
execution/
├── mod.rs              # 门面和装配（≤800 行）
├── root.rs             # 根执行编排
├── subagent.rs         # 子执行编排
├── status.rs           # 状态查询
├── cancel.rs           # 取消控制
└── context.rs          # 执行上下文
```

**拆分顺序**：
1. 先拆分 `service/mod.rs`（影响面更大）
2. 再拆分 `execution/mod.rs`（相对独立）
3. 每次拆分后运行完整测试验证

**Alternatives considered**：
- 立即拆分：风险高，可能与 US1-US4 冲突
- 不拆分：违反宪法 800 行限制

---

## 总结

所有 NEEDS CLARIFICATION 已解决：

1. ✅ **Plugin 类型迁移**：完全迁移到 core，5 阶段迁移计划
2. ✅ **锁恢复机制**：推广 `with_lock_recovery` 和 `lock_anyhow`，tokio Mutex 保持现状
3. ✅ **JoinHandle 管理**：模块自管理策略，4 种管理模式
4. ✅ **错误统一**：AstrError 扩展方案，保留各 crate 自有错误类型
5. ✅ **模块拆分**：两阶段拆分策略，先稳定 US1-US4 再拆分

**下一步**：进入 Phase 1，生成 data-model.md、contracts/、quickstart.md
