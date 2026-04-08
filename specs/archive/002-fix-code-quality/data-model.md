# Data Model: 修复项目代码质量问题

**Feature**: 002-fix-code-quality  
**Date**: 2026-04-08  
**Status**: Complete

本文档定义所有新增或修改的数据结构。

---

## 1. AstrError (扩展)

**Purpose**: 统一错误类型，支持各 crate 错误转换

**Location**: `crates/core/src/error.rs`

**Structure**:
```rust
#[derive(Debug, thiserror::Error)]
pub enum AstrError {
    // ... 现有 variants
    
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
    
    #[error("lock poisoned: {name}")]
    LockPoisoned {
        name: &'static str,
    },
}
```

**Fields**:
- `inner` - 原始错误（通过 `#[source]` 保留错误链）
- `name` - 锁名称（用于 LockPoisoned）

**Relationships**:
- 每个 crate 的自定义错误类型实现 `From<T> for AstrError`
- 通过 `#[source]` 保留错误链，支持 `error.source()` 追溯

**Validation Rules**:
- 转换 MUST 保留原始错误信息
- `map_err` MUST NOT 使用 `|_|` 丢弃原始错误

**State Transitions**: N/A（错误类型无状态转换）

---

## 2. LockRecoveryResult<T>

**Purpose**: 表示锁获取结果（成功、恢复、失败）

**Location**: `crates/core/src/support.rs`

**Structure**:
```rust
pub struct LockRecoveryResult<T> {
    pub value: T,
    pub was_poisoned: bool,
}
```

**Fields**:
- `value: T` - 锁守卫或恢复后的值
- `was_poisoned: bool` - 是否发生中毒恢复

**Relationships**:
- 由 `with_lock_recovery()` 返回（可选，当前实现直接返回 R）
- 用于统一锁恢复语义

**Validation Rules**:
- `was_poisoned = true` 时必须记录错误日志

**State Transitions**: N/A

**Note**: 当前实现中 `with_lock_recovery()` 直接返回闭包结果，不需要此结构体。保留此定义作为未来扩展的参考。

---

## 3. ExecutionTaskManager

**Purpose**: 管理 turn 执行任务的生命周期

**Location**: `crates/runtime/src/service/execution/task_manager.rs`

**Structure**:
```rust
pub struct ExecutionTaskManager {
    active_turns: DashMap<String, tokio::task::JoinHandle<()>>,
}
```

**Fields**:
- `active_turns` - 正在运行的 turn 任务（turn_id → JoinHandle）

**Methods**:
```rust
impl ExecutionTaskManager {
    pub fn new() -> Self;
    pub fn spawn_turn(&self, turn_id: String, task: impl Future<Output = ()> + Send + 'static);
    pub async fn cancel_turn(&self, turn_id: &str) -> bool;
    pub async fn shutdown(&self);
}
```

**Relationships**:
- 由 `AgentExecutionServiceHandle` 持有
- 管理 `root.rs` 和 `mod.rs` 中的 turn 执行任务

**Validation Rules**:
- `turn_id` MUST 唯一
- `shutdown()` MUST 在 RuntimeService 关闭时调用

**State Transitions**:
```
[创建] → spawn_turn() → [运行中] → cancel_turn() → [已取消]
                                  → 任务完成 → [已完成]
                                  → shutdown() → [已关闭]
```

---

## 4. SubagentTaskManager

**Purpose**: 管理子 Agent 执行任务的生命周期

**Location**: `crates/runtime/src/service/execution/subagent_task_manager.rs`

**Structure**:
```rust
pub struct SubagentTaskManager {
    active_children: DashMap<String, tokio::task::JoinHandle<()>>,
    child_cancel_tokens: DashMap<String, CancelToken>,
}
```

**Fields**:
- `active_children` - 正在运行的子 Agent 任务（sub_run_id → JoinHandle）
- `child_cancel_tokens` - 子 Agent 取消令牌（sub_run_id → CancelToken）

**Methods**:
```rust
impl SubagentTaskManager {
    pub fn new() -> Self;
    pub fn spawn_child(&self, sub_run_id: String, cancel: CancelToken, task: impl Future<Output = ()> + Send + 'static);
    pub async fn cancel_child(&self, sub_run_id: &str);
    pub async fn shutdown(&self);
}
```

**Relationships**:
- 由 `SubagentExecutionService` 持有
- 管理 `subagent.rs` 中的子 Agent 执行任务
- 使用 `CancelToken` 实现细粒度取消控制

**Validation Rules**:
- `sub_run_id` MUST 唯一
- `cancel_child()` MUST 先触发 CancelToken 再 abort JoinHandle

**State Transitions**:
```
[创建] → spawn_child() → [运行中] → cancel_child() → [已取消]
                                   → 任务完成 → [已完成]
                                   → shutdown() → [已关闭]
```

---

## 5. WatchManager (扩展)

**Purpose**: 管理配置和 Agent 热重载监听任务

**Location**: `crates/runtime/src/service/watch_manager.rs`

**Structure** (扩展现有结构):
```rust
pub struct WatchManager {
    runtime: Arc<RuntimeService>,
    config_watch_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    agent_watch_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
```

**New Fields**:
- `config_watch_handle` - 配置监听任务句柄
- `agent_watch_handle` - Agent 监听任务句柄

**New Methods**:
```rust
impl WatchManager {
    pub async fn shutdown(&self);
}
```

**Relationships**:
- 由 `RuntimeService` 持有
- 管理 `watch_manager.rs` 中的配置和 Agent 热重载任务

**Validation Rules**:
- `shutdown()` MUST 在 RuntimeService 关闭时调用
- 任务句柄 MUST 在 `start_*_auto_reload()` 时保存

**State Transitions**:
```
[创建] → start_config_auto_reload() → [监听中] → shutdown() → [已关闭]
       → start_agent_auto_reload() → [监听中] → shutdown() → [已关闭]
```

---

## 6. PluginLoadHandle

**Purpose**: 管理插件装配任务的生命周期

**Location**: `crates/runtime/src/bootstrap.rs`

**Structure**:
```rust
pub struct PluginLoadHandle {
    task_handle: Option<tokio::task::JoinHandle<()>>,
    state: Arc<RwLock<PluginLoadState>>,
    completed_notify: Arc<Notify>,
}

pub enum PluginLoadState {
    Loading,
    Completed,
    Failed(String),
}
```

**Fields**:
- `task_handle` - 插件装配任务句柄
- `state` - 装配状态（Loading/Completed/Failed）
- `completed_notify` - 完成通知（用于等待装配完成）

**Methods**:
```rust
impl PluginLoadHandle {
    pub async fn wait_completion(&self);
    pub async fn cancel(&mut self);
    pub fn state(&self) -> PluginLoadState;
}
```

**Relationships**:
- 由 `RuntimeBootstrap` 返回
- 管理 `bootstrap.rs:238` 中的插件装配任务

**Validation Rules**:
- `wait_completion()` MUST 阻塞直到装配完成或失败
- `cancel()` MUST 中止装配任务

**State Transitions**:
```
[创建] → Loading → Completed
                → Failed(reason)
                → cancel() → [已取消]
```

---

## 7. Plugin 类型迁移（从 protocol 移入 core）

**Purpose**: 将 Plugin 领域类型从 protocol 移入 core，实现双向独立

**Affected Types** (19 个类型迁移到 core):

### Descriptor 类型族
- `CapabilityDescriptor` - 能力的完整描述
- `CapabilityDescriptorBuilder` - Builder 模式构造器
- `CapabilityKind` - 能力类型枚举
- `PeerDescriptor` - 通信对等方身份
- `PeerRole` - 对等方角色枚举
- `HandlerDescriptor` - 事件处理器描述
- `TriggerDescriptor` - 触发器描述符
- `FilterDescriptor` - 过滤器描述符
- `ProfileDescriptor` - Profile 描述符
- `SkillDescriptor` - Skill 声明描述符
- `SkillAssetDescriptor` - Skill 资产文件描述符

### 元数据类型
- `SideEffectLevel` - 副作用级别枚举
- `StabilityLevel` - 稳定性级别枚举
- `PermissionHint` - 权限提示
- `DescriptorBuildError` - 描述符构建错误

### 调用上下文类型
- `InvocationContext` - 调用上下文
- `CallerRef` - 调用方引用
- `WorkspaceRef` - 工作区引用
- `BudgetHint` - 预算提示

**New Location**: `crates/core/src/plugin/`

**Structure**: 保持与 protocol 中完全相同的定义（包括 serde 注解）

**Relationships**:
- protocol 中保留传输 DTO（InitializeMessage、InvokeMessage 等）
- plugin crate 实现 mapper（protocol DTO ↔ core 领域类型）

**Validation Rules**:
- 序列化格式 MUST 与 protocol 中的完全相同
- 迁移后 core MUST NOT 依赖 protocol

**Migration Impact**:
- 受影响模块：7 个 crate，30+ 个文件
- 迁移顺序：5 个阶段（准备 → SDK/Plugin → Runtime → Server → 清理）

---

## Summary

**新增实体**：
- `AstrError` 扩展（6 个新 variants）
- `ExecutionTaskManager`（turn 任务管理）
- `SubagentTaskManager`（子 Agent 任务管理）
- `WatchManager` 扩展（任务句柄管理）
- `PluginLoadHandle`（插件装配任务管理）

**迁移实体**：
- 19 个 Plugin 类型从 protocol 移入 core

**关键关系**：
- 所有自定义错误类型 → `AstrError`（通过 `From` trait）
- 所有任务管理器 → `RuntimeService`（通过 shutdown 级联）
- Plugin 类型 → core（领域模型）vs protocol（传输 DTO）

**验证要点**：
- 错误转换保留上下文
- 任务管理器正确保存和清理 JoinHandle
- Plugin 类型迁移不破坏序列化格式
