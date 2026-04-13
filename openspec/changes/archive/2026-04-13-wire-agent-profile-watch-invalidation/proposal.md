## Why

当前仓库已经具备 agent profile watch path 计算、通用 watch abstraction 与 profile cache invalidation 接口，但这些能力没有进入真实组合根与运行路径，导致 profile 文件变化不会稳定影响后续执行。为了让 profile 解析真正成为长期事实源，必须把 watch、失效与后续执行可见性串成正式治理链路。

## What Changes

- 把 agent profile 文件监听与 cache invalidation 正式接入 server/bootstrap 和 `application` 运行路径，使 profile 文件变化能够驱动后续 root/subagent 执行读取新结果。
- 建立 `WatchService`、`WatchSource::AgentDefinitions` 与 `ProfileResolutionService.invalidate*` 之间的稳定映射，避免 watch 能力继续停留在悬空服务状态。
- 明确 global profile 与 working-dir scoped profile 的失效粒度、去重策略与动态 source 管理方式。
- 为运行中的 session 规定边界：watch 失效只影响后续解析与后续 turn，不要求强行改写已启动的执行。
- **BREAKING** 收紧 profile 更新语义：profile 文件变化后，后续执行结果必须以服务端新解析结果为准，不再允许不同入口继续持有长期漂移的旧快照。

## Capabilities

### New Capabilities

- `agent-profile-watch-invalidation`: 定义 agent profile 文件监听、缓存失效与后续执行可见性的正式行为合同。

### Modified Capabilities

- `agent-profile-resolution`: 从“解析与缓存服务存在”扩展到“缓存失效后后续执行必须读到新 profile”的正式行为。

## Impact

- 影响代码：
  - `crates/application/src/watch/mod.rs`
  - `crates/application/src/execution/profiles.rs`
  - `crates/adapter-agents/src/lib.rs`
  - `crates/server/src/bootstrap/*`
- 影响系统：
  - server 组合根中的 watch 装配
  - working-dir 级 agent profile cache 生命周期
  - 后续 root/subagent 执行读取的 profile 新鲜度

## Non-Goals

- 本次不新增 profile 编辑 UI 或手动刷新按钮。
- 本次不要求正在运行中的 turn 或已创建的 child session 立即重新绑定到新 profile。
- 本次不处理 child delivery / parent wake 回流问题，也不处理 root/subagent 执行入口收口问题。

## Migration And Rollback

- 迁移方式为“先接监听与失效，再观察后续执行可见性”：优先让文件变化进入统一 watch 事件流，再把 invalidate 接到真实 profile resolver。
- 如果 watch 接线在特定平台上带来噪声或误触发，可以临时降级为仅保留显式失效入口与重启可见性，但必须保留清晰日志并同步调整 spec 说明。
