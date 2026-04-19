## Why

当前 `Astrcode` 的工具调用仍以同步 turn 执行为主：长时间运行的 shell / tool 会直接阻塞 turn loop，而现有运行时又没有一套像 Claude Code 那样的后台任务注册、输出落盘与完成通知机制。同时，现有 `shell` 只覆盖一次性非交互命令，无法提供持久终端会话、stdin 回写与有限等待窗口输出语义，导致 LLM 无法像现代 coding agent 那样稳定操控终端。

现在引入该能力的时机已经成熟：`session-runtime` 已经是单会话真相面，`ToolContext` 也已有 stdout / stderr 流式输出通道，继续沿同步一次性工具模型堆补丁，只会让事件模型、conversation 投影与前端展示继续分裂。

## What Changes

- 引入 Claude Code 风格的后台任务执行语义：长耗时工具不再阻塞当前 turn，而是立即返回 `backgroundTaskId`、输出路径和状态摘要，后续通过完成通知与显式读取获取结果。
- 为长任务建立统一的进程监管面，区分“一次性后台命令”和“持久终端会话”，统一处理生命周期、取消、失败、输出落盘与通知。
- 保留现有 `shell` 作为一次性命令工具，并增加 `background` 执行模式；同时新增 Codex 风格的持久执行工具族，使 LLM 可以启动带 `process_id` 的终端会话、写入 stdin、在有限等待窗口内拿到新输出并终止或关闭会话。
- 扩展事件模型、conversation authoritative read model 与前端展示，使“后台任务已启动/已完成/已失败”和“终端会话输出/状态变化”成为正式合同，而不是前端本地猜测。
- 明确后台任务和终端会话的恢复/失败语义：Astrcode 重启后不得静默丢失状态，必须向用户和模型暴露明确的 lost / failed 结果。

## Capabilities

### New Capabilities
- `async-tool-execution`: 定义工具如何立即返回后台任务句柄、持续产出输出、发送完成通知并暴露稳定输出引用。
- `terminal-tool-sessions`: 定义持久终端会话的创建、输入输出、生命周期控制与失败恢复语义。

### Modified Capabilities
- `session-runtime`: 增加后台任务通知、终端会话状态投影、内部完成唤醒输入等会话真相要求。
- `terminal-chat-read-model`: 增加后台任务通知块、终端会话块及其 hydration / delta 合同。

## Impact

- 受影响模块：
  - `crates/core`：工具结果类型、后台任务/终端事件与端口契约
  - `crates/session-runtime`：完成通知输入、conversation/query 投影、终端会话读取路径
  - `crates/application`：后台任务/终端会话监管与用例编排
  - `crates/adapter-tools`：`shell` 扩展与新终端工具族
  - `crates/protocol` 与 `frontend`：waiting / terminal session DTO 与渲染
- 用户可见影响：
  - 长工具调用不再卡死会话，而是变成后台任务并在完成时收到通知
  - LLM 可以通过正式工具直接操控持久终端会话，并基于 `process_id` 持续交互
- 开发者可见影响：
  - 工具执行从“同步完成即返回”演进为“foreground / background / persistent-exec-session”多模式合同
  - 事件模型需要新增后台任务通知与终端会话语义，避免继续把跨多次交互的输出硬塞进单个 `tool_call_id`
- 依赖与系统影响：
  - 可能需要引入跨平台 PTY 支撑（例如 `portable-pty` 或等价方案）
  - 需要同步更新 `PROJECT_ARCHITECTURE.md`，明确后台进程监管与终端会话在 `application` / `session-runtime` / read model 之间的职责边界
