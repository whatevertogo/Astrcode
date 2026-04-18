## Why

当前迁移已经把单 session 真相逐步收回 `session-runtime`，但 agent control 相关能力仍然存在两个风险：

1. `kernel` 的 `agent_tree`、事件流和查询能力还更像内部实现，而不是稳定合同。
2. `application/server` 若继续绕过稳定接口直接碰内部结构，就会把全局控制面重新耦合到实现细节。

旧项目里确实存在 subrun status、route、wake、observe、close 等真实产品需求，但它们必须以“稳定控制合同”形式暴露，而不是按旧 runtime 结构直接搬回来。

## What Changes

- 为 agent 控制面建立稳定的对外合同，统一承接：
  - subrun status 查询
  - child delivery / mailbox 路由
  - observe / wake / close
- 明确 `kernel` 只负责全局控制与路由，不接管 session 真相。
- 明确 `application`/`server` 只能依赖稳定控制合同，不直接触碰 `agent_tree` 内部结构。

## Capabilities

### New Capabilities

- `subrun-status-contracts`: 为 root agent 与 subrun 暴露统一、稳定、可查询的状态合同。
- `agent-delivery-contracts`: 为 route / wake / close / observe 提供稳定控制合同。

### Modified Capabilities

- `kernel`: 必须暴露稳定 agent control API，而不是要求上层依赖内部树结构。
- `application-use-cases`: 必须通过稳定控制合同编排 agent 控制请求。

## Impact

- `kernel` 将增加稳定的查询与控制接口。
- `application/server` 将收敛到合同层，避免继续依赖内部实现。
- 这是破坏性整理：不保留旧 runtime 风格的临时 façade。
