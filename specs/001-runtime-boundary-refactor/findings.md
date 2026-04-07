# Findings: Runtime Boundary Refactor

本文只记录已确认的代码事实，不包含解决方案。

## 1. 当前 durable subrun 事件没有独立保存完整 lineage 和 trigger link

- `StorageEvent::SubRunStarted` / `SubRunFinished` 当前字段只有 `turn_id`、`agent`、resolved snapshots 或 result/step/token，没有 `tool_call_id`，也没有单独的 lineage descriptor。  
  参考：`crates/core/src/event/types.rs:170-199`
- 对应的 domain event 和 protocol DTO 也是同一组字段，没有额外的 durable lineage 结构。  
  参考：`crates/core/src/event/domain.rs:118-130`、`crates/protocol/src/http/event.rs:349-365`
- `AgentEventContext` 当前只包含 `agent_id`、`parent_turn_id`、`agent_profile`、`sub_run_id`、`invocation_kind`、`storage_mode`、`child_session_id`，没有 `depth` 与 `parent_agent_id`。  
  参考：`crates/core/src/agent/mod.rs:381-403`
- subrun 生命周期事件的实际写入点 `emit_child_started` / `emit_finished` 也没有写入 `tool_call_id` 或额外 lineage 结构。  
  参考：`crates/runtime/src/service/execution/subagent.rs:277-286`、`crates/runtime/src/service/execution/subagent.rs:436-445`

## 2. durable replay 目前无法完整重建 subrun handle

- `find_subrun_status_in_events` 只从 lifecycle event 中收集 `agent`、resolved snapshots、finish result。  
  参考：`crates/runtime-execution/src/subrun.rs:45-86`
- `build_replayed_handle` 在 replay 时把 `depth` 硬编码为 `1`，并把 `parent_agent_id` 固定为 `None`。  
  参考：`crates/runtime-execution/src/subrun.rs:89-110`
- 当前 status 查询会先读 `agent_control`，live 中找不到时再退回 durable replay。  
  参考：`crates/runtime/src/service/execution/status.rs:10-45`

## 3. `runtime-session` 仍然编译依赖 `runtime-agent-control` 和 `runtime-agent-loop`

- `crates/runtime-session/Cargo.toml` 直接依赖 `astrcode-runtime-agent-control` 与 `astrcode-runtime-agent-loop`。  
  参考：`crates/runtime-session/Cargo.toml`
- `turn_runtime.rs` 直接导入 `AgentControl`、`AgentLoop` 相关类型与工具函数。  
  参考：`crates/runtime-session/src/turn_runtime.rs:11-15`
- `complete_session_execution` 会在 session 完成时调用 `agent_control.cancel_for_parent_turn(turn_id)`。  
  参考：`crates/runtime-session/src/turn_runtime.rs:95-111`

## 4. `runtime` 中旧 façade 和新用例模块当前并存

- `service/mod.rs` 仍然同时声明 `execution_service`、`session_service`，以及新的 `execution`、`session`、`turn` 模块。  
  参考：`crates/runtime/src/service/mod.rs:34-44`
- `RuntimeService::load_session_history` 经过 `service/session/load.rs` 再委托到 `session_service()`。  
  参考：`crates/runtime/src/service/session/load.rs:9-27`
- `RuntimeService::submit_prompt` / `interrupt` 经过 `service/turn/submit.rs` 再委托到 `execution_service()`。  
  参考：`crates/runtime/src/service/turn/submit.rs:3-16`
- `SessionReplaySource for RuntimeService` 的 `replay()` 仍然委托到 `ExecutionService::replay()`。  
  参考：`crates/runtime/src/service/replay.rs:9-19`
- `SessionService::delete_session` 内部又调用 `self.runtime.interrupt(&normalized)`。  
  参考：`crates/runtime/src/service/session_service.rs:218-223`

## 5. server 过滤和 frontend subrun tree 仍在用 turn-owner 启发式推 parent/child

- server 的 `SessionEventFilter` 内部维护 `turn_owner` 和 `sub_run_parent` 两张表。  
  参考：`crates/server/src/http/routes/sessions/filter.rs:45-50`
- `matches()` 会先 `observe_turn_owner()` 再 `observe_sub_run_parent()`，最后通过 `turn_owner` 推导 scope。  
  参考：`crates/server/src/http/routes/sessions/filter.rs:61-85`
- `observe_sub_run_parent()` 通过 `event_parent_turn_id(event)` 再查 `turn_owner` 得到父 subrun。  
  参考：`crates/server/src/http/routes/sessions/filter.rs:117-129`
- frontend 的 `buildSubRunIndex()` 先建立 `turnOwnerMap`，再通过 `parentTurnId` 反推 `parentSubRunId`。  
  参考：`frontend/src/lib/subRunView.ts:137-177`

## 6. agent profile loader 已支持 working-dir 作用域，但 runtime 启动与 watch 流程仍然绑定进程 cwd

- `AgentProfileLoader::load()` 会读取 `std::env::current_dir()` 并委托到 `load_for_working_dir()`。  
  参考：`crates/runtime-agent-loader/src/lib.rs:121-148`
- runtime bootstrap 初始化 agent profile registry 时调用的是 `agent_loader.load()`。  
  参考：`crates/runtime/src/bootstrap.rs:145-148`
- agent watch loop 也是从 `std::env::current_dir()` 计算 `watch_paths`，后续 debounce 后仍使用同一个 `working_dir` 重新求 watch target。  
  参考：`crates/runtime/src/service/watch_ops.rs:85-127`
- 根执行路由虽然允许请求带 `working_dir`，但缺失时会静默退回 `std::env::current_dir()`。  
  参考：`crates/server/src/http/routes/agents.rs:45-50`

## 7. 当前 server / frontend 外部入口已经同时消费旧 façade和新 handle

- server 的 agent 相关路由使用 `agent_execution_service()`。  
  参考：`crates/server/src/http/routes/agents.rs:28-31`、`crates/server/src/http/routes/agents.rs:52-63`、`crates/server/src/http/routes/agents.rs:87-90`、`crates/server/src/http/routes/agents.rs:104-107`
- server 的 session 历史、stream、mutation 仍然调用 `RuntimeService::load_session_history()`、`replay()`、`submit_prompt()`、`interrupt()` 这些 façade 入口。  
  参考：`crates/server/src/http/routes/sessions/query.rs:43-58`、`crates/server/src/http/routes/sessions/stream.rs:93-196`、`crates/server/src/http/routes/sessions/mutation.rs:48-70`
- `DeferredSubAgentExecutor` 也通过 `runtime.agent_execution_service()` 间接发起 subagent。  
  参考：`crates/runtime/src/service/execution/mod.rs:55-79`

