## Why

当前项目已经有干净的 `application -> session-runtime -> kernel` 分层，但“根代理执行”和“子代理执行”仍然没有作为正式业务入口落地，导致旧 runtime 中最关键的执行能力还没真正迁入。现在需要把这些入口补齐到 `application`，让迁移继续沿着正确边界推进，而不是把执行逻辑再次塞回 `App` 根文件或 `kernel`。

## What Changes

- 在 `application` 中新增独立执行子域，承接 `execute_root_agent` 和 `launch_subagent`
- 把根代理执行、子代理执行、profile 解析与 working-dir 级缓存收敛为明确的业务用例
- 明确 `application` 只做执行入口编排，`session-runtime` 负责单 session turn，`kernel` 负责全局 agent control
- 让根执行、spawn/send/observe/close 四工具模型能通过稳定边界接回现有 server 合同
- **BREAKING**：旧 runtime 中隐式耦合的执行路径将不再保留；新的执行入口只通过 `application` 暴露

## Capabilities

### New Capabilities
- `root-agent-execution`: 定义根代理执行的业务入口、参数校验和执行回执契约
- `subagent-execution`: 定义子代理创建、执行、结果回流与关闭的业务入口契约
- `agent-profile-resolution`: 定义 working-dir 级 agent profile 解析与缓存契约

### Modified Capabilities
- `application-use-cases`: 扩展 `application` 的用例边界，使其正式承接执行入口而不长回 runtime façade
- `session-runtime`: 明确它只消费已解析执行输入，不负责 profile 解析、权限和业务策略判断
- `kernel`: 明确 agent control 由 kernel 提供全局能力，但不直接实现业务入口

## Impact

- 受影响 crate：`crates/application`、`crates/kernel`、`crates/session-runtime`、`crates/server`
- 受影响模块：`application/execution/*`、`server/http/routes/agents.rs`、agent profile 解析与缓存路径
- 用户可见影响：agent execute / spawn / close / observe 将拥有更稳定的一致行为
- 开发者可见影响：执行入口不再散落在 `App` 或路由层，而是有明确子域和边界
- 迁移与回滚：先增加新执行子域和调用路径，再切换 server 调用；回滚时可暂时保留旧路由编排，但不恢复旧 runtime 门面
