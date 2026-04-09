# Implementation Status: Runtime Boundary Refactor

本文记录重构完成后的实施状态和关键变更点。

## 1. ✅ Durable subrun 事件现已包含完整 lineage 和 trigger link

- `StorageEvent::SubRunStarted` / `SubRunFinished` 现在包含 `descriptor: Option<SubRunDescriptor>` 和 `tool_call_id: Option<String>` 字段。  
  参考：`crates/core/src/event/types.rs:178-196`
- `SubRunDescriptor` 包含完整的 lineage 信息：`sub_run_id`、`parent_turn_id`、`parent_agent_id`、`depth`。  
  参考：`crates/core/src/agent/mod.rs:319-325`
- 对应的 domain event 和 protocol DTO 已同步更新，支持 descriptor 序列化。  
  参考：`crates/core/src/event/domain.rs`、`crates/protocol/src/http/event.rs`
- subrun 生命周期事件写入点 `emit_child_started` / `emit_finished` 现在写入完整的 descriptor 和 tool_call_id。  
  参考：`crates/runtime/src/service/execution/subagent.rs`

## 2. ✅ Durable replay 现已完整重建 subrun handle

- `ExecutionLineageIndex` 新增，用于从 durable events 中索引和查询 lineage 信息。  
  参考：`crates/runtime-execution/src/subrun.rs`
- `find_subrun_status_in_events` 现在从 lifecycle event 的 descriptor 中提取完整的 lineage 信息。  
  参考：`crates/runtime-execution/src/subrun.rs`
- `build_replayed_handle` 使用 descriptor 中的实际 `depth` 和 `parent_agent_id`，不再硬编码。  
  参考：`crates/runtime-execution/src/subrun.rs`
- Status 查询优先读取 live state，降级到 durable replay 时使用 `ExecutionLineageIndex`。  
  参考：`crates/runtime/src/service/execution/status.rs`
- Legacy 历史（缺少 descriptor）返回 `source=legacyDurable`，lineage 字段为 null。

## 3. ✅ Runtime boundary 已清晰分离

- `runtime-session` 不再编译依赖 `runtime-agent-control` 和 `runtime-agent-loop`。  
  参考：`crates/runtime-session/Cargo.toml`
- Session truth 通过 trait `SessionTruthSource` 定义，execution 通过 trait `ExecutionOrchestrator` 定义。  
  参考：`crates/core/src/runtime/traits.rs`
- Turn orchestration 逻辑移至 `crates/runtime/src/service/turn/orchestration.rs`，不再混合在 session 层。
- `runtime-execution` 作为独立 crate 管理 execution context、subrun lineage index 和 status 查询。  
  参考：`crates/runtime-execution/src/lib.rs`

## 4. ✅ 旧 façade 已删除，新边界已生效

- `service/execution_service.rs` 和 `service/session_service.rs` 已删除。
- `service/session/load.rs`、`service/turn/submit.rs`、`service/replay.rs` 已删除。
- `RuntimeService` 现在通过 trait 方法直接调用新的 execution 和 session 模块。  
  参考：`crates/runtime/src/service/mod.rs`
- Server 路由直接使用 `agent_execution_service()` 和新的 session/turn 入口。  
  参考：`crates/server/src/http/routes/`
- 所有调用点已迁移到新的边界接口，不再依赖旧 façade。

## 5. ✅ Server 过滤和 frontend 现已使用 descriptor-based lineage

- Server 的 `SessionEventFilter` 现在使用 `ExecutionLineageIndex` 构建 lineage 树。  
  参考：`crates/server/src/http/routes/sessions/filter.rs`
- `matches()` 通过 descriptor 的 `parent_agent_id` 直接判断 scope，不再依赖 turn-owner 启发式。  
  参考：`crates/server/src/http/routes/sessions/filter.rs`
- Frontend 的 `buildSubRunThreadTree()` 使用 `descriptorParentAgentId` 构建树。  
  参考：`frontend/src/lib/subRunView.ts:205-207`
- Legacy 记录（`!hasDescriptorLineage`）显式设置 `parentSubRunId = null`，不再伪造 ancestry。  
  参考：`frontend/src/lib/subRunView.ts:198-206`
- Frontend UI 显示 lineage 状态警告，提示用户 legacy 历史的父子关系不完整。  
  参考：`frontend/src/components/Chat/SubRunBlock.tsx`

## 6. ✅ Agent 解析和 watch 作用域已绑定到 execution context

- `AgentProfileLoader::load_for_working_dir()` 接受显式的 `working_dir` 参数，不再依赖 `std::env::current_dir()`。  
  参考：`crates/runtime-agent-loader/src/lib.rs`
- Runtime bootstrap 和 execution 初始化时调用 `load_for_working_dir(Some(&working_dir))`。  
  参考：`crates/runtime/src/service/execution/mod.rs:139`
- Agent watch loop 从活跃 sessions 收集 `working_dirs`，调用 `watch_paths_for_working_dirs()`。  
  参考：`crates/runtime/src/service/watch_ops.rs:243-247`
- 根执行路由要求 `working_dir` 必填，缺失时返回 400 错误，不再静默退回进程 cwd。  
  参考：`crates/server/src/http/routes/agents.rs`
- 每个 execution context 拥有独立的 agent 解析作用域，多项目隔离生效。

## 7. ✅ Server 和 frontend 入口已统一使用新边界

- Server 的 agent 路由使用 `agent_execution_service()`。  
  参考：`crates/server/src/http/routes/agents.rs`
- Server 的 session 历史、stream、mutation 使用新的 session/turn 入口。  
  参考：`crates/server/src/http/routes/sessions/`
- `DeferredSubAgentExecutor` 通过 `runtime.agent_execution_service()` 发起 subagent。  
  参考：`crates/runtime/src/service/execution/mod.rs`
- 所有外部入口已迁移完成，不再调用旧 façade。

## 总结

所有 7 个发现点已完成重构：
1. ✅ Durable events 包含完整 lineage 和 trigger link
2. ✅ Durable replay 完整重建 subrun handle
3. ✅ Runtime boundary 清晰分离
4. ✅ 旧 façade 已删除
5. ✅ Server 和 frontend 使用 descriptor-based lineage
6. ✅ Agent 解析绑定到 execution context
7. ✅ 所有入口统一使用新边界

重构目标已全部达成。

