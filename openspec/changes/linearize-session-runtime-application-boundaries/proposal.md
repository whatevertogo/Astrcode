## Why

`session-runtime` 目前同时暴露了过宽的根门面、重复的 turn/query 投影逻辑，以及多处可被外层直接拿来拼装内部事实的 helper；`application` 又把 `ProjectedTurnOutcome`、`TurnTerminalSnapshot`、`AgentObserveSnapshot` 等 `session-runtime` / `kernel` 具体类型继续向上透传。这让单 session 真相、应用用例编排和跨层合同缠在一起，代码越来越像一张网，而不是几条可理解的主线。

现在需要先做一个收敛性的第一阶段，把 `session-runtime -> application` 这条主线重新拉直：先消除重复真相、收口运行时公开表面、补上应用层 anti-corruption contracts，再为后续 `server` 隔离、`core` 瘦身和 hooks 平台演进建立干净基础。

## What Changes

- 统一 `session-runtime` 内部重复的 turn 终态投影、assistant summary 提取和 `session_id` 规范化逻辑，明确单一 canonical owner。
- 收口 `SessionRuntime` 根门面，使其更像组合入口而不是万能对象；公开 API 按 query / command / orchestration 责任分组。
- 为 `application` 补一层稳定的 session 合同摘要，移除对 `session-runtime` / `kernel` 内部快照类型的直接暴露与 re-export。
- 约束 `application` 只消费 `session-runtime` 的稳定 façade，不再依赖低层 helper、投影器或路径工具函数。
- 明确本次触及的外部合同与扩展点一律以纯数据快照 / 纯数据决策交互，不向 runtime 外泄取消、锁、原子状态等运行时控制细节。
- 同步更新相关 OpenSpec 与 `PROJECT_ARCHITECTURE.md` 的表述，使代码边界与仓库级架构约束重新对齐。

## Non-Goals

- 本次不引入 hooks 平台，也不把 hooks 相关改造并入本 change。
- 本次不完成 `server` 对 `session-runtime` 的全面隔离，只为后续隔离建立稳定 application 合同。
- 本次不做 `core` 的全面瘦身搬迁；`core` 中运行时算法 / I/O 归位留给后续 change。
- 本次不拆 crate，不调整 `kernel` 的总体职责；`kernel` 仅允许做极小的 surface 收口配合。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `session-runtime`: 收敛重复真相与过宽 façade，明确 turn/query helper 的唯一所有者，并把公开能力组织为更线性的 query / command / orchestration 表面。
- `session-runtime-subdomain-boundaries`: 明确 `turn`、`query`、`state`、`command` 之间的 canonical helper 所有权与单向依赖，禁止继续跨子域重建同类投影语义。
- `application-use-cases`: 约束 `application` 通过稳定 anti-corruption contracts 消费 `session-runtime` / `kernel` 能力，不再把底层内部快照与实现类型作为公共合同继续向上传递。

## Impact

- 主要影响 `crates/session-runtime`、`crates/application`，以及少量与公共合同相关的 `crates/server` 编译适配与测试。
- 会调整若干公开类型导出与 port trait 签名，属于开发者可见的 API 收口；仓库不追求向后兼容，本次优先以边界清晰和长期可维护性为准。
- 需要同步更新 `PROJECT_ARCHITECTURE.md` 与相关 OpenSpec，确保“application 只依赖稳定 runtime 合同、server 不持有业务真相”的原则落到代码结构上。
