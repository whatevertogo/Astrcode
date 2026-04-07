# Compact 系统设计

## 要解决的问题

Compact 系统要在不破坏会话真相的前提下，持续把长对话压缩成可恢复、可继续执行、可审计的上下文。

Astrcode 当前已经有一条可用主线，但原来的计划和 TODO 分散在多个文档里，缺少统一的设计入口。

## 设计目标

1. 压缩结果必须服务于**继续执行**，而不是只做历史摘要。
2. 压缩过程必须与 session / replay / tail 恢复兼容。
3. 已压缩内容要能以事件和摘要形式被 UI 理解，而不是隐藏成黑盒。
4. 新能力优先走增量增强，不推翻现有 compaction policy 与 runtime pipeline。

## 当前主线

- durable 事件仍是真相，compact 只是建立新的上下文边界。
- 摘要必须保留任务目标、约束、关键决策、当前进度与下一步。
- 压缩后恢复优先依赖已有机制：compact summary、recent tail、附件恢复。
- 先增强 hook、cache 与 prompt 工程，再考虑更激进的 mid-turn compact 或 session memory。

## 核心设计判断

### 1. Compact 是上下文维护系统，不只是 prompt 模板

真正需要设计的是：

- 何时压缩
- 压缩前保留什么
- 压缩后恢复什么
- UI 与 replay 如何感知压缩边界

而不是只调一句“帮我总结一下”。

### 2. 优先做可插拔和可审计

短期最值得做的是：

- compact hook
- prune 标记而不是直接隐式删除
- 更稳定的 prompt 结构
- 更精确的 token 计量

### 3. 缓存优化属于收益增强，不应改变语义面

cache-sharing fork、context edit、时间触发微压缩都应该被视为 runtime 优化，而不是新的用户语义。

## 当前不主张的方向

- 在边界未收口前引入专用 compact agent
- 把 compact 做成前端快照协议的一部分
- 为了缓存命中而改写 session 真相

## 对应规范

- [../spec/compact-system-spec.md](../spec/compact-system-spec.md)
- [../spec/open-items.md](../spec/open-items.md)
