## Why

当前项目已经具备较干净的 `server/bootstrap -> application -> kernel/session-runtime -> adapter-*` 分层，但 plugin 相关能力仍未完全进入这条主链路：

- plugin loader / supervisor 还未成为组合根中的真实输入
- plugin hook / skill / capability 还未稳定并入 capability surface
- `reload` 若没有真正带动插件能力刷新，就只是治理层的空转

旧项目里这些能力确实存在，因此需要迁入；但迁入方式必须是“把插件变成 capability surface 的一个真实来源”，而不是再造一个 runtime 中心层。

## What Changes

- 将 plugin discovery、loader、supervisor 生命周期接入 `server/bootstrap` 与 `application` 治理入口。
- 将 plugin 提供的 hooks、skills、capabilities 统一物化为 `kernel` 可替换的 capability surface 输入。
- 明确 builtin、MCP、plugin 三类能力统一并入同一 surface 替换链路。

## Capabilities

### New Capabilities

- `plugin-capability-surface`: plugin 能力、hook、skill 能通过统一物化与刷新链路并入 capability surface。
- `plugin-governance-lifecycle`: plugin 的发现、装载、刷新、失败可进入治理视图。

### Modified Capabilities

- `kernel`: capability surface 替换必须同时承接 builtin、MCP、plugin 三类来源。
- `application-use-cases`: reload 与治理快照必须反映 plugin 生命周期与 surface 结果。

## Impact

- `server/bootstrap` 会成为 plugin loader 和 capability materializer 的唯一接线点。
- `application` 会补齐真实 reload 编排与治理快照。
- `kernel` 会承担统一的 surface 原子替换，不再只接 builtin/MCP。
