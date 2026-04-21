## ADDED Requirements

### Requirement: 环境副作用能力由 `adapter-*` 或受限 support crate 实现

凡是依赖文件系统、shell、进程探测或 durable 持久化的基础设施能力，SHALL 由 `adapter-*` 或职责受限的 support crate 提供实现，并通过稳定契约暴露给上层。

这至少包括：

- project dir 解析、working dir 归一化所需的文件系统能力
- home 目录解析
- shell / process 探测与命令执行
- tool result 与等价执行产物的 durable persist
- plugin manifest 解析

#### Scenario: side effects are implemented by adapters

- **WHEN** 检查上述能力的最终实现位置
- **THEN** 真实实现 SHALL 位于某个 `adapter-*` 或 `astrcode-support` 这类职责受限的 support crate
- **AND** `core` / `application` / `session-runtime` 只通过契约消费这些能力

#### Scenario: adapter choice may vary without moving ownership back upward

- **WHEN** 团队判断某项副作用更适合 `adapter-storage` 还是其他现有 adapter
- **THEN** 可以在 adapter 层内部调整 owner
- **AND** 该实现 ownership SHALL NOT 回流到 `core`

---

### Requirement: `astrcode-support` 或等价 durable adapter 承接工具结果持久化

tool result、压缩产物或其他需要 durable 保存的执行结果，SHALL 由 `astrcode-support`、`adapter-storage` 或等价的 durable adapter 负责最终持久化实现。

#### Scenario: tool result persistence is no longer implemented in core

- **WHEN** 检查工具结果落盘与恢复相关实现
- **THEN** durable persist 逻辑 SHALL 位于 `astrcode-support`、`adapter-storage` 或等价 durable adapter
- **AND** `core` 不再直接实现这些落盘细节

---

### Requirement: shell、home 与 manifest 解析由 adapter、support crate 或组合根 owner 提供

shell 检测、home 目录解析、plugin manifest 解析等宿主相关能力，SHALL 由 `adapter-*`、`astrcode-support` 这类职责受限的 support crate，或组合根附近的 owner 提供；`core` 最多只保留共享数据结构和契约。

#### Scenario: shell detection is not implemented in core

- **WHEN** 检查 shell family 检测、默认 shell 选择、命令存在性检查
- **THEN** 这些实现 SHALL 位于 `astrcode-support::shell`、`adapter-tools` 或等价宿主 adapter
- **AND** `core` 只保留 `ShellFamily`、`ResolvedShell` 等共享数据结构

#### Scenario: plugin manifest parsing is not implemented in core

- **WHEN** 检查 `PluginManifest` 的 TOML 解析 owner
- **THEN** 实际解析实现 SHALL 位于 adapter、application 或组合根
- **AND** `core` 只保留 manifest 数据结构定义

#### Scenario: shared host path resolution is centralized outside core

- **WHEN** 多个 crate 需要共享 Astrcode home / projects / project bucket 解析
- **THEN** 这些宿主路径 helper SHALL 位于 `astrcode-support::hostpaths` 或等价受限 support crate
- **AND** `core` 不再拥有 `dirs::home_dir()`、Astrcode 根目录拼装或 `project_dir()` 这类 owner
