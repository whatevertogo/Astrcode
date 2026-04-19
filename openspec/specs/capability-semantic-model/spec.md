## Purpose

定义 AstrCode 运行时唯一的 capability semantic model，约束能力语义、执行语义与边界 DTO 的职责分离，并确保 child capability surface 仍然建立在同一模型之上。

## Requirements

### Requirement: `core` 定义唯一能力语义模型 `CapabilitySpec`

`astrcode-core` SHALL 定义 `CapabilitySpec`，作为运行时内部唯一能力语义模型。`CapabilitySpec` SHALL 不依赖 `astrcode-protocol`。

`CapabilitySpec` SHALL 包含以下语义字段：

- `name: CapabilityName` — 能力名称（newtype，非裸字符串）
- `kind: CapabilityKind` — 能力类型枚举
- `description: String` — 能力描述
- `input_schema: Value` — 输入 JSON Schema
- `output_schema: Value` — 输出 JSON Schema
- `invocation_mode: InvocationMode` — 调用模式枚举（`Unary` | `Streaming`）
- `concurrency_safe: bool` — 是否并发安全
- `compact_clearable: bool` — 压缩时是否可清除
- `profiles: Vec<String>` — 能力适用的 profile 列表
- `tags: Vec<String>` — 标签列表
- `permissions: Vec<PermissionSpec>` — 权限声明列表
- `side_effect: SideEffect` — 副作用级别（`None` | `Local` | `Workspace` | `External`）
- `stability: Stability` — 稳定性级别（`Experimental` | `Stable` | `Deprecated`）
- `metadata: Value` — 自定义元数据
- `max_result_inline_size: Option<usize>` — 最大内联结果大小

`CapabilitySpec` 通过 `CapabilitySpecBuilder` 构建器模式创建，构建时自动校验所有必填字段和格式约束。

#### Scenario: core 可以独立于 protocol 编译

- **WHEN** 执行 `cargo check -p astrcode-core`
- **THEN** 编译成功
- **AND** `crates/core/Cargo.toml` 不包含 `astrcode-protocol`

#### Scenario: 运行时内部统一消费 CapabilitySpec

- **WHEN** 检查 `core`、`kernel`、`session-runtime`、`application`、`adapter-*`
- **THEN** 内部能力语义类型为 `core::CapabilitySpec`
- **AND** 不再以 `protocol::CapabilityWireDescriptor` 作为内部主模型

---

### Requirement: 能力语义使用强类型

能力语义 SHALL 使用枚举或 newtype 表达，不依赖裸字符串约定。

#### Scenario: `CapabilityKind` 为枚举

- **WHEN** 检查能力语义定义
- **THEN** `CapabilityKind` 为枚举，包含 `Tool`, `Agent`, `ContextProvider`, `MemoryProvider`, `PolicyHook`, `Renderer`, `Resource`, `Prompt`, `Custom(String)` 变体
- **AND** 支持通过 `From<&str>` / `From<String>` 从字符串解析
- **AND** `is_tool()` 方法判断是否为 Tool 类型

#### Scenario: `InvocationMode` 为枚举

- **WHEN** 检查调用模式定义
- **THEN** `InvocationMode` 为枚举，包含 `Unary` 和 `Streaming` 变体
- **AND** 流式语义不再用 `streaming: bool`

#### Scenario: `CapabilityName` 为 newtype

- **WHEN** 检查能力名称定义
- **THEN** `CapabilityName` 为 newtype 包装 `String`
- **AND** 提供 `as_str()` 和 `into_string()` 方法

#### Scenario: `SideEffect` 和 `Stability` 为枚举

- **WHEN** 检查副作用和稳定性定义
- **THEN** `SideEffect` 包含 `None`, `Local`, `Workspace`, `External` 变体
- **AND** `Stability` 包含 `Experimental`, `Stable`, `Deprecated` 变体

#### Scenario: `PermissionSpec` 包含名称和理由

- **WHEN** 检查权限声明定义
- **THEN** `PermissionSpec` 包含 `name: String` 和 `rationale: Option<String>`

---

### Requirement: 执行提示字段归 `core` 所有

以下字段 SHALL 归 `core` 所有，因为它们决定运行时行为，而不是传输形状：

- `profiles`
- `compact_clearable`
- `max_result_inline_size`
- `invocation_mode`
- `side_effect`
- `stability`

#### Scenario: prompt / loop / plugin 基于 CapabilitySpec 决策

- **WHEN** prompt、turn loop 或 plugin 需要判断 profile、compaction、streaming 语义
- **THEN** 从 `CapabilitySpec` 读取
- **AND** 不再从 `CapabilityWireDescriptor` 读取执行语义

---

### Requirement: `CapabilityWireDescriptor` 降级为边界 DTO

`astrcode-protocol::CapabilityWireDescriptor` SHALL 仅承担 wire DTO 职责，不承担运行时内部语义职责。

#### Scenario: 协议边界统一映射

- **WHEN** server 响应、插件握手或其他协议边界输出能力描述
- **THEN** 通过显式 mapper 将 `CapabilitySpec` 映射为 `CapabilityWireDescriptor`

#### Scenario: 非边界层不依赖 DTO 语义

- **WHEN** 检查 `core`、`kernel`、`session-runtime`、`application`
- **THEN** 不存在围绕 `CapabilityWireDescriptor` 的业务判断逻辑

---

### Requirement: Tool 与 Registry 接口返回 `CapabilitySpec`

`Tool` trait 的能力描述接口，以及 `CapabilityInvoker` trait SHALL 返回 `CapabilitySpec`。

#### Scenario: Tool 能力描述返回 CapabilitySpec

- **WHEN** 检查 `core/tool.rs`
- **THEN** `Tool::capability_spec()` 方法返回 `Result<CapabilitySpec>`

#### Scenario: CapabilityInvoker 主接口返回 CapabilitySpec

- **WHEN** 检查 `kernel/registry/router.rs` 或 `core/registry/router.rs`
- **THEN** `CapabilityInvoker::capability_spec()` 返回 `CapabilitySpec`

---

### Requirement: Capability Semantic Model Supports Discovery Without Parallel Registries

能力语义模型 MUST 成为 discovery 能力的扩展点，而不是允许平行注册表再次出现。

#### Scenario: Discovery needs extra semantic metadata

- **WHEN** 工具发现需要标签、可见性、语义描述、搜索字段或排序字段
- **THEN** 系统 SHALL 在现有 capability semantic model 上扩展这些字段
- **AND** SHALL NOT 新建平行 discovery 模型作为第二事实源

### Requirement: child capability surfaces SHALL remain projections of the current capability semantic model

子 agent 的可见能力面 MUST 从当前 `CapabilitySpec` / capability router 派生，而不是通过 `AgentProfile` 或其他平行配置对象重新定义一套能力真相。

#### Scenario: child launch resolves capability surface

- **WHEN** 系统启动一个 child agent
- **THEN** child 的能力面 MUST 基于当前 capability router 求得
- **AND** MUST NOT 仅凭 `AgentProfile` 直接决定 child 可调用哪些工具

#### Scenario: status and replay explain child capabilities

- **WHEN** 系统查询或回放某个 child 的运行状态
- **THEN** 用于解释该 child capability 的数据 MUST 对应到启动时的 capability semantic projection
- **AND** MUST NOT 退回到读取最新 profile 文件重新推断

### Requirement: task-scoped capability grants SHALL constrain runtime capability projections

面向 child agent 的任务级授权 MUST 作为 capability projection 的输入之一，而不是扩展 `AgentProfile` 成为权限对象。

#### Scenario: spawn requests a restricted capability grant

- **WHEN** 调用方在 `spawn` 时提供任务级 capability grant
- **THEN** 系统 MUST 用该 grant 约束 child 的 capability projection
- **AND** grant 的语义 MUST 建立在现有 `CapabilitySpec` 命名与分类之上

#### Scenario: no task-scoped capability grant is provided

- **WHEN** 调用方未提供 capability grant
- **THEN** child 的 capability projection MUST 回退到父级可继承上界与 runtime availability 的交集
- **AND** MUST NOT 因为缺少 grant 就去读取 profile 中的工具白名单作为替代真相