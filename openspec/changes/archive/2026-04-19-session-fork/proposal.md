## Why

AstrCode 的 `branch_session_from_busy_turn` 已经实现了"复制源 session 事件到新 session"的核心逻辑（replay → 稳定前缀截断 → 创建新 session → 逐条 append），但它被硬编码为"session 正忙时自动分叉"这一个场景。我们需要将这个能力泛化为通用的 session fork——从源 session 的**稳定前缀**创建独立新 session，继承该点之前的完整事件历史。这不仅是用户功能（回溯探索到某个已完成 turn 重新来过），更是一个底层能力：后台 compact 隔离、多智能体并行分支、未来模式切换的 KV cache 友好派生，都建立在此之上。

## What Changes

- 在 `SessionRuntime`（`session-runtime` crate）上新增 `fork_session` 方法，复用现有 `branch_session_from_busy_turn` 的核心逻辑，泛化为支持从任意已完成 turn 的末尾 fork
- 在 `session-runtime` 中新增 `ForkPoint` 枚举和 `ForkResult` 结构体，不污染 `core` crate
- Fork 点**只接受稳定前缀**（已完成的 turn 序列），不复制活跃 turn 的半截事件，确保新 session 投影后 `phase = Idle`
- 复用 `SessionStart` 已有的 `parent_session_id` / `parent_storage_seq` 谱系字段，不改事件结构
- 复用现有 `SessionCatalogEvent::SessionBranched` catalog event，不新增事件类型
- 新增 HTTP API `POST /api/sessions/:id/fork`，`turnId` 与 `storageSeq` 互斥，同时传入返回 `Validation` 错误
- 前端支持在已完成 turn 和可映射到该 turn 的历史消息上触发 fork，但这只是客户端便捷入口，提交到后端时必须先归一化为 `turnId`
- fork 成功后前端立即切换到新 session，保持类似 Claude 的“从此处分叉并进入分支”体验，但底层仍以 AstrCode 的稳定前缀事件模型为准
- 在 `protocol` 层新增 `ForkSessionRequest` DTO，响应复用现有 `SessionListItem`（通过 `parentSessionId` 字段表达谱系）

## Capabilities

### New Capabilities

- `session-fork`: 定义从稳定前缀 fork session 的核心语义、稳定前缀校验规则、fork 点解析规则和谱系记录契约

### Modified Capabilities

- `session-runtime`: 新增 `fork_session` 方法、`ForkPoint` 枚举和 `ForkResult` 结构体
- `server-session-routes`: 新增 `POST /api/sessions/:id/fork` 端点

## Impact

- 受影响 crate：`session-runtime`（fork 逻辑、ForkPoint/ForkResult 类型）、`application`（fork use case）、`protocol`（ForkSessionRequest DTO）、`server`（HTTP 端点）
- 用户可见影响：前端可在已完成 turn 及其历史消息处展示"从此处 fork"操作，fork 后立即切换到新 session，新 session 的 `parentSessionId` 指向源 session
- 开发者可见影响：后台流程可通过 `SessionRuntime::fork_session()` 进行隔离操作
- 迁移与回滚：纯增量，不改现有 branch 逻辑和 EventStore trait，现有行为不受影响
