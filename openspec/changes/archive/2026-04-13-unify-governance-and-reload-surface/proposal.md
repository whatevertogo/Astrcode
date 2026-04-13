## Why

当前仓库已经完成了主要运行时拆分，但治理与重载链路仍停留在“结构已迁、行为未闭环”的状态：`AppGovernance` 只有基础模型，`/api/config/reload` 只重读配置而不会统一重建 capability surface，MCP 刷新又走独立路径。继续在这种状态下推进功能，会让新架构重新长出分叉治理入口和半刷新语义。

## What Changes

- 建立统一的治理与重载入口，让 `application` 负责编排 builtin、MCP、plugin 三类能力来源的完整 reload，而不是分别由路由层或局部服务各自刷新。
- 明确治理快照、reload 原子替换、失败回退、运行中会话限制等行为语义，避免“配置已更新但 surface 未一致替换”的半生效状态。
- 收紧 server 侧治理调用路径，使配置重载、状态快照和后续治理接口统一走 `AppGovernance`，不再保留分散的 ad-hoc reload 逻辑。
- 调整 plugin 治理与 capability surface 参与语义，要求治理结果能够稳定反映 plugin 发现、装载、失败与参与 surface 的最终状态。
- **BREAKING** `POST /api/config/reload` 的语义将从“仅重读配置文件”提升为“执行完整治理级 reload 或显式失败”；依赖旧行为的隐式调用方需要同步更新认知与测试。

## Non-Goals

- 不在本次 change 中补做新的前端治理页面或运行时诊断 UI。
- 不在本次 change 中重写 `kernel` 或 `session-runtime` 的主执行模型。
- 不为旧 `runtime` 命名或旧 reload 路径保留长期兼容层。

## Capabilities

### New Capabilities

- `governance-reload-surface`: 定义治理入口如何统一编排 reload、原子替换 capability surface、暴露失败结果并保持旧状态可回退。

### Modified Capabilities

- `application-use-cases`: 调整 `application` 对治理、reload 和 server 治理入口的职责要求。
- `plugin-governance-lifecycle`: 扩展 plugin 生命周期在治理快照与 reload 结果中的表达要求。
- `plugin-capability-surface`: 强化 plugin 参与统一 capability surface 替换时的一致性要求。

## Impact

- 受影响代码主要位于 `crates/application/src/lifecycle/*`、`crates/server/src/bootstrap/*`、`crates/server/src/http/routes/config.rs`、`crates/server/src/http/mapper.rs`、`crates/server/src/main.rs`、`crates/adapter-mcp/src/manager/*` 与相关协议 DTO。
- 用户可见影响：配置重载和后续治理接口会变得更可靠，失败会显式暴露，不再出现“看起来 reload 成功但部分能力没刷新”的状态。
- 开发者可见影响：reload 入口会收口为单一路径，测试需要覆盖运行中会话限制、原子替换、失败回退和多来源 capability 同步。
- 迁移与回滚思路：迁移阶段先把旧局部 reload 调用重定向到新治理入口，再删除分叉路径；若新治理链路在落地过程中发现阻塞问题，回滚策略应是保留旧 capability surface 并返回显式失败，而不是接受半刷新结果。
