## Why

当前前端 composer 和执行合同之间还存在明显断层：`/compact` 在 busy 状态只能直接拒绝，`maxSteps` 与 `tokenBudget` 只停留在 TODO 和零散字段层面，输入侧无法稳定表达“这次执行该怎么跑”。如果不把这层控制面补齐，后续前端能力只会继续堆在零散条件分支上。

## What Changes

- 建立稳定的 composer 执行控制合同，让前端能够表达并持久处理手动 compact、执行预算和后续扩展控制，而不是把这些能力散落在临时判断里。
- 为根代理执行和子代理执行补齐显式的 `maxSteps` / `tokenBudget` 输入合同，并让后端按新架构把这些参数下传到正确的业务边界。
- 调整 `turn-budget-governance` 的输入来源，使用户显式指定的 budget 能进入 `session-runtime` 的 turn 决策，而不是只依赖默认配置或 TODO 字段。
- 收口 composer 在运行中状态下的控制行为，优先把“拒绝”升级为“有语义的执行控制处理”，为后续排队或受控延迟执行留下稳定边界。

## Non-Goals

- 不在本次 change 中实现附件上传或文件选择能力；附件入口仍属于独立产品能力。
- 不在本次 change 中重做聊天输入框的视觉设计或大规模交互改版。
- 不在本次 change 中引入任意命令脚本队列系统，只聚焦执行控制相关能力。

## Capabilities

### New Capabilities

- `composer-execution-controls`: 定义 composer 如何表达执行控制、运行中状态下如何处理控制请求，以及这些控制如何映射到稳定 API 合同。

### Modified Capabilities

- `root-agent-execution`: 为根代理执行补充显式执行控制参数的契约。
- `subagent-execution`: 为子代理执行补充显式执行控制参数的契约。
- `turn-budget-governance`: 让用户指定的 token budget 成为正式输入来源，而不是仅由默认配置驱动。

## Impact

- 受影响代码主要位于 `frontend/src/components/Chat/InputBar.tsx`、`frontend/src/hooks/app/useComposerActions.ts`、`frontend/src/types.ts`、`frontend/src/lib/api/*`、`crates/protocol/src/http/*`、`crates/application/src/execution/*`、`crates/session-runtime/src/turn/*` 与对应测试。
- 用户可见影响：输入侧会获得更稳定的执行控制体验，手动 compact 和预算类控制不再只是 TODO 或硬拒绝。
- 开发者可见影响：前后端需要围绕统一 DTO 和业务合同更新测试，不再依赖“某个字段以后再加”的临时约定。
- 迁移与回滚思路：优先以可选参数方式引入新控制合同，确保旧调用方仍可走默认行为；若新控制路径在落地中出现问题，回滚应退回默认执行语义，同时移除前端对新控制的显式暴露。
