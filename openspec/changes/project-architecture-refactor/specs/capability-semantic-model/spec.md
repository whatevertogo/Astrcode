## ADDED Requirements

### Requirement: `core` 定义唯一能力语义模型 `CapabilitySpec`

`astrcode-core` SHALL 定义 `CapabilitySpec`，作为运行时内部唯一能力语义模型。`CapabilitySpec` SHALL 不依赖 `astrcode-protocol`。

`CapabilitySpec` SHALL 至少包含以下语义字段：

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

#### Scenario: core 可以独立于 protocol 编译

- **WHEN** 执行 `cargo check -p astrcode-core`
- **THEN** 编译成功
- **AND** `crates/core/Cargo.toml` 不包含 `astrcode-protocol`

#### Scenario: 运行时内部统一消费 CapabilitySpec

- **WHEN** 检查 `core`、`kernel`、`session-runtime`、`application`、`adapter-*`
- **THEN** 内部能力语义类型为 `core::CapabilitySpec`
- **AND** 不再以 `protocol::CapabilityDescriptor` 作为内部主模型

---

### Requirement: 能力语义使用强类型

能力语义 SHALL 使用枚举或 newtype 表达，不依赖裸字符串约定。

#### Scenario: `CapabilityKind` 与 `InvocationMode` 为枚举

- **WHEN** 检查能力语义定义
- **THEN** `CapabilityKind` 与 `InvocationMode` 均为 `enum`
- **AND** 运行时内部不通过字符串比较决定能力行为

#### Scenario: 流式语义不再用 `streaming: bool`

- **WHEN** 一个能力支持流式返回
- **THEN** `CapabilitySpec.invocation_mode == InvocationMode::Streaming`
- **AND** 不再以传输层布尔字段表达运行时调用语义

---

### Requirement: 执行提示字段归 `core` 所有

以下字段 SHALL 归 `core` 所有，因为它们决定运行时行为，而不是传输形状：

- `profiles`
- `compact_clearable`
- `max_result_inline_size`
- `invocation_mode`

#### Scenario: prompt / loop / plugin 基于 CapabilitySpec 决策

- **WHEN** prompt、turn loop 或 plugin 需要判断 profile、compaction、streaming 语义
- **THEN** 从 `CapabilitySpec` 读取
- **AND** 不再从 `CapabilityDescriptor` 读取执行语义

---

### Requirement: `CapabilityDescriptor` 降级为边界 DTO

`astrcode-protocol::CapabilityDescriptor` SHALL 仅承担 wire DTO 职责，不承担运行时内部语义职责。

#### Scenario: 协议边界统一映射

- **WHEN** server 响应、插件握手或其他协议边界输出能力描述
- **THEN** 通过显式 mapper 将 `CapabilitySpec` 映射为 `CapabilityDescriptor`

#### Scenario: 非边界层不依赖 DTO 语义

- **WHEN** 检查 `core`、`kernel`、`session-runtime`、`application`
- **THEN** 不存在围绕 `CapabilityDescriptor` 的业务判断逻辑

---

### Requirement: Tool 与 Registry 接口返回 `CapabilitySpec`

`ToolCapabilityMetadata`、`Tool` trait 的能力描述接口，以及 `CapabilityInvoker` trait SHALL 返回 `CapabilitySpec`。

#### Scenario: Tool 能力描述返回 CapabilitySpec

- **WHEN** 检查 `core/tool.rs`
- **THEN** 默认能力描述构建逻辑返回 `CapabilitySpec`

#### Scenario: CapabilityInvoker 主接口返回 CapabilitySpec

- **WHEN** 检查 `core/registry/router.rs`
- **THEN** 公共接口返回 `CapabilitySpec`
- **AND** 不再以 `descriptor()` 作为主语义接口
