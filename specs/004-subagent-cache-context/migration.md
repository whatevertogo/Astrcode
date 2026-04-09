# Migration: 子智能体会话与缓存边界优化

## Migration Principles

- 新写入立即切到独立子会话 durable 模型。
- 旧共享写入历史不再保留读取、回放或恢复兼容。
- 先切断旧路径，再稳定 replay、prompt cache 和父唤醒。
- 每个阶段都必须有可执行验证门槛，避免“一次性大爆炸”。

## Current Cutover State

- Phase 1 到 Phase 5 的目标都已落地到默认运行路径。
- 当前 steady state 只允许独立 child session 新写入；shared-history 只作为 legacy 输入被显式拒绝。
- 后续维护重点从“迁移设计”转为“守住边界”，避免把 runtime bridge、legacy DTO 或调试读模型重新扩成公共真相面。

## Caller Inventory

| Area | Current Assumption | Migration Action |
|------|--------------------|------------------|
| `runtime-execution` policy / spawn path | 独立子会话仍受 experimental 守卫 | 去掉默认 guard，统一新写入路径 |
| `runtime-session` / replay path | 仍可能背共享写入 legacy 读取语义 | 已删除共享写入读取/回放/恢复路径，统一稳定错误返回 |
| `runtime-execution` context assembly | 父背景主要拼接进任务消息 | 已拆成 task payload + inherited blocks |
| `runtime-prompt` | 已有 fingerprint，但 child 继承层未独立建模 | 已落地 inherited prompt layer，并接入共享 LayerCache |
| `runtime-agent-loop` / `runtime` execution service | 父唤醒依赖 durable `ReactivationPrompt` | 已切到运行时信号 + 一次性交付输入 |
| `runtime-agent-control` | 交付缓冲偏内存态、语义未收紧 | 已增加幂等、FIFO 和忙父重排；重启后仍只保留 durable 追溯 |
| `server` / `protocol` | 可能继续为 legacy 历史投影 | 已改为显式 `unsupported_legacy_shared_history` 错误契约 |
| `frontend` | 可能假设所有历史都能正常打开 | 已对齐父摘要视图与 legacy 显式拒绝展示 |

## Phase Order

### Phase 1: 默认独立子会话与旧路径切断

- 去掉新 child 的 experimental 阻塞
- 新写入统一创建独立 child session
- 删除共享写入历史的读取、回放与恢复路径
- 为 legacy 访问增加稳定错误码和推荐动作
- 同步规范、测试和文档

**Exit Criteria**

- 新建 child 100% 生成独立 `child_session_id`
- 旧共享写入历史访问返回稳定错误，而不是进入兼容逻辑
- 父历史不再混入子内部事件

### Phase 2: Replay-Based Resume 与 lineage mismatch 失败路径

- 为 resume 接入 child session durable replay / projector
- 创建新的 `ChildExecutionInstance(trigger_kind=resume)`
- 在 lineage 不一致或历史损坏时输出双通道错误

**Exit Criteria**

- resume 成功时原 `child_session_id` 不变
- 失败时不会隐式 spawn 并列 child

### Phase 3: Inherited Prompt Blocks 与共享缓存

- 拆分 `resolve_context_snapshot()` 输出
- 引入 inherited prompt blocks
- 复用 `runtime-prompt` fingerprint 与 LayerCache
- 增加 recent tail 确定性筛选和预算裁剪

**Exit Criteria**

- 首条任务消息不再混入父背景全文
- 支持缓存指标的 provider 上可观测到缓存收益

### Phase 4: 父唤醒与交付桥接迁移

- 移除 durable `ReactivationPrompt` 写入
- 引入运行时唤醒信号和一次性交付输入
- 明确多交付独立缓冲、逐个消费和幂等约束

**Exit Criteria**

- 父 durable 历史中不再出现机制性唤醒消息
- 多子交付不会丢失、合并或重复消费

### Phase 5: 投影、协议与前端收尾

- 更新 `/history`、`/events`、status DTO
- 前端对齐父摘要、子入口、legacy 拒绝和 lineage 错误展示
- 完成日志、指标和回归测试补齐

**Exit Criteria**

- `/history` 与 `/events` 对新边界事实语义一致
- 前端不再依赖混合历史猜测 child 内部过程
- legacy 共享写入历史明确显示为不受支持

## Cutover Checkpoints

### Checkpoint A: Legacy Read Paths Removed

- `runtime-session` 不再为共享写入历史提供读取、回放或恢复逻辑。
- `server` 与 `protocol` 返回稳定错误码 `unsupported_legacy_shared_history`。
- 回归测试覆盖 legacy 访问失败路径。

### Checkpoint B: Child Session Truth Stable

- 新 child spawn 100% 生成独立 `child_session_id`。
- 父历史只保留边界事实，不出现子内部事件。
- `SubRun` 与 `ChildSessionNode` 的 durable/transport 词汇保持一致。

### Checkpoint C: Prompt Split Landed

- child 首条任务消息只含 task payload。
- compact summary / recent tail 已进入 inherited prompt blocks。
- prompt 层与消息流的缓存边界已可测试。

### Checkpoint D: Parent Wake No Longer Pollutes Durable History

- durable 历史里不存在 `ReactivationPrompt`。
- 交付详情只通过一次性运行时输入桥接。
- 父忙碌时多子交付能够独立缓冲、逐个送达。

### Checkpoint E: Public Surface Consistent

- `/history`、`/events`、status DTO、前端 session API 对 `childSessionId`、`executionId`、`statusSource`、legacy rejection 的命名和语义一致。
- 文档、前端展示和错误文案都不再暗示共享写入历史仍受支持。

## Post-Cutover Guardrails

- 不要把 `SharedSession` / `legacyDurable` 从 DTO 中简单删除；它们仍承担 legacy 样本显式拒绝与测试夹具输入的读侧职责。
- 不要把 mixed-thread helper 当作默认父视图真相；它现在只用于子线程细节浏览和调试。
- 不要在新的父唤醒路径里重新引入 durable `UserMessage` 桥接；一次性交付材料必须继续停留在 runtime prompt declarations。

## Public-Surface Removal Notes

- 删除或收紧任何“共享写入历史仍可读取/回放/恢复”的公共语义；不保留隐藏式兼容分支。
- 删除 durable `ReactivationPrompt` 驱动路径；父唤醒只保留运行时语义。
- 删除把 inherited context 混入 child 首条 `UserMessage` 的公共行为。
- 删除把 mixed subrun thread tree 作为默认父视图主入口的假设，统一转向 child session summary + open-session link。

## Cutover Strategy

- 不保留旧共享写入历史的读取或回放能力
- 不提供本 feature 内置迁移器
- 若环境中仍存在 legacy 数据，由外部升级或清理流程处理

## Validation Matrix

1. `cargo fmt --all --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --exclude astrcode`
4. `cd frontend && npm run typecheck && npm run lint && npm run format:check && npm run test`
5. 手工验证：
   - 独立子会话
   - resume 沿用原会话
   - inherited context 进入 system blocks
   - `PromptMetrics.cache_creation_input_tokens` 缓存收益
   - 父历史无 durable `ReactivationPrompt`
   - 旧共享写入历史返回稳定错误

## Rollback Considerations

- 当前实现不建议回滚到“继续支持共享写入历史”的模式，因为那会重新引入双轨 durable 语义。
- 若未来只想调整 prompt cache 或父唤醒策略，应保持独立 child session 与 replay-based resume 这两条 durable 基线不动。
