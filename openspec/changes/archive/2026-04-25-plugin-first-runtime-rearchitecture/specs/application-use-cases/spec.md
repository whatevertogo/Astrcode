## REMOVED Requirements

### Requirement: `application` 提供唯一业务入口 `App`

**Reason**: 本次重构明确删除 `application` crate，不保留 `App` 作为兼容 façade。旧 use-case 语义将按新边界分配到 `host-session`、`plugin-host` 与 `server`。

**Migration**: 将 session/conversation/observe/branch/fork 等入口迁入 `host-session`；将 plugin discovery/reload/resource aggregation 迁入 `plugin-host`；`server` 改为直接装配并消费新 host surfaces。

### Requirement: `application` 只依赖核心运行时层

**Reason**: `application` crate 将被删除，这条 crate 依赖边界约束不再成立。

**Migration**: 以新的 `agent-runtime`、`host-session`、`plugin-host` crate 边界替代旧的 `application -> kernel/session-runtime` 依赖结构。

### Requirement: `application` 负责用例编排、参数校验和权限前置

**Reason**: 这些职责会被拆分给新的 host 层 owner，而不是继续堆叠在单独的 `application` crate 中。

**Migration**: session/turn 相关用例下沉到 `host-session`；plugin/resource/reload 相关用例归属 `plugin-host`；协议映射与 transport concern 保持在 `server`。
