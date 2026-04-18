## Context

当前项目同时拥有三类与 profile 更新相关的能力：

- `adapter-agents` 中的 watch path 推导
- `application::watch` 中的通用 watch abstraction
- `application::execution::profiles` 中的 invalidate 接口

但这些能力没有进入 server 组合根，也没有驱动真实执行链路失效。结果是 profile 文件变化之后，prompt facts、执行入口与缓存生命周期之间可能继续出现不一致。

## Goals / Non-Goals

**Goals:**

- 把 agent profile 文件监听接入真实组合根
- 把 watch 事件稳定映射到 profile cache invalidation
- 定义 global 与 scoped profile 的失效粒度与后续执行可见性
- 保持运行中 turn 不被强行热切换，只影响后续解析

**Non-Goals:**

- 不新增 profile 编辑界面或手动刷新 API
- 不在本 change 中处理 root/subagent 执行入口如何使用 resolver
- 不要求对已启动 child session 做即时 profile 重绑定

## Decisions

### 决策 1：监听只进入 `application::watch` 抽象，平台细节仍留在适配器层

server 组合根负责装配 `WatchService`，但底层文件系统监听仍由 `WatchPort` 的适配器实现承担。

原因：

- 避免 server 或 application 直接绑定平台文件系统库
- 保持 watch source 与业务失效逻辑可以单测

### 决策 2：watch 事件只触发 cache invalidation，不直接重建执行中的 profile 快照

agent profile 文件变化后的正式语义是：

- 失效对应 working-dir 或 global cache
- 后续 root/subagent 解析重新读取磁盘
- 当前正在运行的 turn 不被中途改写

原因：

- 这符合事件日志优先与 session 真相边界，不会把执行中的上下文切成两半
- 也能把风险限制在“后续执行行为变化”，而不是“当前执行中途漂移”

### 决策 3：global 与 scoped 失效分开处理

- 命中全局 agent 定义目录变化：调用 `invalidate_global`
- 命中项目目录下 `.astrcode/agents/`：调用对应 working-dir 的 `invalidate`
- 无法精确定位目录归属时，退化为 `invalidate_all`

原因：

- 这样可以在正确性与实现复杂度之间取得平衡
- 先保证不会错用旧值，再逐步优化失效粒度

### 决策 4：watch source 的动态维护由组合根按已知工作目录管理

server 组合根根据当前会话 working-dir 集合维护 `WatchSource::AgentDefinitions`，会话集合变化时增删 watch source。

原因：

- agent profile 的有效作用域天然与 working-dir 绑定
- 这与旧项目的 watch target 动态维护方式相近，但新的实现不再依赖旧 runtime façade

## Risks / Trade-offs

- [Risk] working-dir 变化频繁时，动态增删 watch source 可能带来管理复杂度  
  → Mitigation：先允许在无法精确维护时采用较粗粒度监听，再逐步优化 source 集合

- [Risk] 文件系统事件噪声导致频繁失效  
  → Mitigation：在适配器层保持防抖，并在 application 层只做无副作用 invalidate

- [Risk] prompt facts 与 execution resolver 若仍然使用不同加载路径，watch 接线后仍会有漂移  
  → Mitigation：本 change 明确将后续执行新鲜度定义为目标，同时在设计里记录与执行入口收口 change 的依赖关系

## Migration Plan

1. 在组合根装配 `WatchService`
2. 将 agent definition watch source 与当前工作目录集合连接
3. 将 watch event 映射到 `ProfileResolutionService.invalidate*`
4. 补充 cache 失效与后续执行读取新 profile 的测试

回滚策略：

- 若平台监听稳定性不足，可临时保留 invalidate 接口和粗粒度重启可见性，关闭自动监听

## Open Questions

- prompt facts 是否也要切换到同一 resolver；本 change 先聚焦执行侧可见性，不强制统一 prompt facts 实现
