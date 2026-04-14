## Why

当前 Astrcode 的 agent-tool 调试能力仍然挤在主界面右下角，以一个临时浮层的形式存在。这个方案虽然验证了治理指标和实时工具流的价值，但也暴露出三个明显问题：

- 调试信息与主聊天界面争抢空间，阅读与筛选成本高。
- 最近 5 分钟趋势目前主要依赖前端本地采样，窗口刷新或重开后会丢失。
- debug 读取逻辑散落在 `server`、`application` 与前端组件之间，还没有形成清晰的后端读模型边界。

Astrcode 现在需要把这套调试能力升级为一个正式的 **Debug Workbench**：server 继续作为唯一组合根，独立的 debug 后端能力负责查询与聚合，前端则以单独窗口承载只读观测台。这样既能保持现有 runtime 真相不分叉，也能为后续更完整的治理/诊断工作台打下干净边界。

## What Changes

- 新增 `debug-workbench-read-model` capability，正式定义 Debug Workbench 的后端读模型、时间窗口趋势和会话级 trace 查询。
- 修改 `runtime-observability-pipeline` capability，要求最近时间窗口趋势由服务端维护，不再以仅前端内存采样作为主真相。
- 修改 `server-http-debug-surface` capability，要求 `server` 在 debug 构建中统一暴露 `/api/debug/*` 只读接口，并通过独立 debug crate 装配读模型。
- 将当前前端右下角调试浮层替换为独立的 Debug Workbench 窗口，保留主应用中的轻量入口，但不再在主聊天界面内渲染浮层。

## Capabilities

### New Capabilities

- `debug-workbench-read-model`: 定义 runtime overview、timeline、session trace 与 session agent tree 的调试读模型，以及它们的只读查询边界。

### Modified Capabilities

- `runtime-observability-pipeline`: runtime 调试趋势必须有服务端维护的时间窗口快照，支持 Debug Workbench 在重开窗口后继续读取最近样本。
- `server-http-debug-surface`: `server` 必须在 debug 构建中暴露 Debug Workbench 所需的只读 `/api/debug/*` 接口，并保持 DTO 映射和认证边界集中在 server。

## Impact

- 影响代码：
  - `crates/application/src/observability/*`
  - `crates/server/src/http/{routes,mapper}.rs`
  - `crates/protocol/src/http/*`
  - 新增 `crates/debug-workbench/*`
  - `frontend/src/*` 中的 debug UI 入口和独立工作台
  - `src-tauri/src/*` 中的窗口创建与宿主命令
- 用户可见影响：
  - 调试界面不再挤在主窗口右下角，而是以独立窗口出现
  - 最近 5 分钟趋势在关闭/重开调试窗口后仍然可读
  - 可以同时查看全局治理指标、当前会话 trace 和 child agent tree
- 开发者可见影响：
  - debug 查询与聚合逻辑从 `server`/前端临时实现中抽离，形成稳定的读模型边界
  - 后续可以在不污染主业务的前提下扩展更多调试图表和导出能力

## Non-Goals

- 不在本次 change 中实现可写调试命令或控制台能力。
- 不拆分成独立仓库或独立部署的 debug 前端项目。
- 不将所有 observability 逻辑迁入新 crate，只抽出 Debug Workbench 所需的读模型与查询能力。
