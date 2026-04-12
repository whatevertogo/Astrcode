## ADDED Requirements

### Requirement: `core` 定义唯一能力语义模型 `CapabilitySpec`

`astrcode-core` SHALL 定义 `CapabilitySpec`，作为 runtime 内部唯一能力语义模型。`CapabilitySpec` SHALL 不依赖 `astrcode-protocol`。

`CapabilitySpec` SHALL 包含以下字段：

- `name`
- `kind`
- `description`
- `input_schema`
- `output_schema`
- `invocation_mode`
- `concurrency_safe`
- `compact_clearable`
- `profiles`
- `tags`
- `permissions`
- `side_effect`
- `stability`
- `max_result_inline_size`

#### Scenario: core 独立编译

- **WHEN** 执行 `cargo check -p astrcode-core`
- **THEN** 编译成功
- **AND** `crates/core/Cargo.toml` 中不包含 `astrcode-protocol`

#### Scenario: CapabilitySpec 是 runtime 内部统一模型

- **WHEN** 检查 `core`、`kernel`、`session-runtime`、`application`、`adapter-*`
- **THEN** 它们内部消费的能力语义类型是 `core::CapabilitySpec`
- **AND** 不再以 `protocol::CapabilityDescriptor` 作为内部能力模型

---

### Requirement: 能力模型使用强类型而不是字符串拼装

`CapabilitySpec` 相关语义 SHALL 使用强类型枚举或 newtype 表达，而不是在内部依赖裸字符串约定。

#### Scenario: kind 与 invocation mode 是枚举

- **WHEN** 检查 `CapabilityKind` 与 `InvocationMode`
- **THEN** 它们是 `enum`
- **AND** 运行时内部不通过字符串比较决定能力语义

---

### Requirement: `InvocationMode` 进入 `core`

`streaming: bool` SHALL 被 `core::InvocationMode` 替代，避免用传输层布尔字段表达运行时调用语义。

#### Scenario: Streaming tool 通过 InvocationMode 表达

- **WHEN** 一个能力支持流式返回
- **THEN** 它的 `CapabilitySpec.invocation_mode` 为 `InvocationMode::Streaming`

#### Scenario: Unary tool 通过 InvocationMode 表达

- **WHEN** 一个能力只支持普通请求-响应
- **THEN** 它的 `CapabilitySpec.invocation_mode` 为 `InvocationMode::Unary`

---

### Requirement: 运行时真正使用的执行提示字段留在 `core`

以下字段 SHALL 归 `core` 所有，因为它们影响运行时行为，而不是单纯影响传输形状：

- `profiles`
- `compact_clearable`
- `max_result_inline_size`
- `invocation_mode`

#### Scenario: prompt/runtime/plugin 不再从 protocol 读取这些字段

- **WHEN** `runtime-prompt`、`runtime-agent-loop`、`plugin` 需要判断 profile、compaction 或 streaming 语义
- **THEN** 它们从 `CapabilitySpec` 读取
- **AND** 不再把 `CapabilityDescriptor` 当作运行时主模型

---

### Requirement: `protocol::CapabilityDescriptor` 只保留传输职责

`astrcode-protocol` SHALL 保留 `CapabilityDescriptor` 作为 wire DTO，但它不再是 runtime 内部的主语义模型。

#### Scenario: 边界处进行映射

- **WHEN** server 响应、插件握手或其他协议边界需要能力描述
- **THEN** 在边界处将 `CapabilitySpec` 映射为 `CapabilityDescriptor`

#### Scenario: runtime 内部不直接依赖 DTO

- **WHEN** 检查 `core`、`kernel`、`session-runtime`、`application`
- **THEN** 不存在直接围绕 `CapabilityDescriptor` 进行业务判断的实现

---

### Requirement: Tool 和 CapabilityInvoker 改为返回 `CapabilitySpec`

`ToolCapabilityMetadata`、`Tool` trait 的能力描述接口，以及 `CapabilityInvoker` trait SHALL 改为构建并返回 `CapabilitySpec`。

#### Scenario: Tool 不再构建 CapabilityDescriptor

- **WHEN** 检查 `core/tool.rs`
- **THEN** 默认能力描述构建逻辑返回 `CapabilitySpec`

#### Scenario: CapabilityInvoker 不再暴露 descriptor

- **WHEN** 检查 `core/registry/router.rs`
- **THEN** 公开接口返回 `CapabilitySpec`
- **AND** 不再以 `descriptor()` 作为主接口
