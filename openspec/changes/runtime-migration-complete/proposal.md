## Why

旧 `runtime-*` 架构中的剩余能力仍未完全迁入当前仓库，但当前项目已经有一套更干净的目标架构。  
继续迁移时，最大的风险已经不是“功能没搬完”，而是**为了赶功能，把已经建立好的边界重新污染掉**。

这份 change 现在的使命是：

1. 让 change 文档与当前实现重新对齐
2. 把已完成的基础迁移明确沉淀下来
3. 只保留那些仍值得继续迁、且不破坏架构的剩余任务

## What Changes

- 明确 `session-runtime` 是唯一单 session 真相面，不再把 session 持久化或 history/replay 放在 `application`
- 明确 `application` 只保留用例编排、参数校验、权限/策略入口、错误映射与治理编排
- 明确 `kernel` 只保留 agent tree、gateway、capability surface 等全局控制职责
- 重写 `runtime-migration-complete` 的 `design.md` 与 `tasks.md`，去掉已被当前架构否决或已经过时的任务描述
- 保留真正未完成的迁移项：root/subagent 执行、token budget auto-continue、plugin 集成、部分 server 合同补齐

## Non-goals

- 不按旧 runtime 文件路径做 1:1 复刻
- 不重新引入 `RuntimeService` 一类的大门面
- 不把 session shadow state 放回 `application`
- 不把 turn 构造细节塞回 `kernel`
- 不做向后兼容处理

## Capabilities

### Completed Foundations

- `session-persistence`：基础 durable session 真相已迁入 `session-runtime`
- `agent-lifecycle`：kernel agent tree 的 inbox / observe / cancel propagation 基础已存在
- `auxiliary-features`：配置连接测试已具备可用实现

### Remaining Migration Work

- `turn-orchestration`：补齐 token budget 驱动的 auto-continue 与更稳定的 observability 汇总
- `agent-execution`：补齐 execute_root_agent / launch_subagent 等业务入口
- `plugin-integration`：补齐 plugin loader / hook / skill / capability surface 接线
- `auxiliary-features`：评估并补齐工具搜索、Skill Tool 等仍有产品价值的能力

## Impact

**对实现的影响**

- OpenSpec change 将不再误导后续实现继续沿着旧边界扩张
- 后续开发会围绕真实剩余缺口推进，而不是重复做已经完成或已被否决的工作

**对架构的影响**

- 当前项目“架构第一，迁移第二”的原则被正式写入这份 change
- 后续迁移若与现边界冲突，优先修改迁移方案，而不是修改架构
