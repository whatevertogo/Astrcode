# Design: Subrun Protocol Slice

## Goal

把 subrun 的 durable 真相收敛成一个可以独立评审的“第一刀”：

- 补齐 durable lifecycle event 缺失的 lineage 与 trigger 信息
- 让 `/history`、`/events`、subrun status 都能消费同一份 durable 事实
- 明确旧历史如何降级

本文只讨论协议与 durable/query 读写影响，不讨论五个边界的 owner 划分；那部分单独放在 `design-execution-boundary.md`。

## Change Set

### 1. 新增 `SubRunDescriptor`

在 `crates/core/src/agent/mod.rs` 新增 durable lineage 结构：

```rust
pub struct SubRunDescriptor {
    pub sub_run_id: String,
    pub parent_turn_id: String,
    pub parent_agent_id: Option<String>,
    pub depth: usize,
}
```

**职责**

- 只表达 ownership / lineage
- 不表达运行态 status
- 不表达 resolved overrides / limits
- 不携带 session mode 推断逻辑

### 2. 调整 durable event 结构

为 `StorageEvent::SubRunStarted` 和 `StorageEvent::SubRunFinished` 同时增加：

- `descriptor: Option<SubRunDescriptor>`
- `tool_call_id: Option<String>`

`Option` 只用于读旧历史时兼容反序列化；新代码写入时这两个字段都必须存在。

### 3. 调整 domain event 与 protocol DTO

同步修改：

- `crates/core/src/event/domain.rs`
- `crates/protocol/src/http/event.rs`

使 `SubRunStarted` / `SubRunFinished` 的 domain event、HTTP event payload 与 durable event 保持同一字段集合。

### 4. 调整 subrun status 输出结构

将 `SubRunStatusDto` 改成“durable descriptor + live/durable status overlay”的显式结构：

```rust
pub struct SubRunStatusDto {
    pub sub_run_id: String,
    pub descriptor: Option<SubRunDescriptorDto>,
    pub tool_call_id: Option<String>,
    pub source: SubRunStatusSourceDto,
    pub agent: SubRunAgentBindingDto,
    pub status: String,
    pub result: Option<SubRunResultDto>,
    pub step_count: Option<u32>,
    pub estimated_tokens: Option<u64>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverridesDto>,
    pub resolved_limits: Option<ResolvedExecutionLimitsDto>,
}
```

其中：

- `descriptor` / `tool_call_id` 是 durable facts
- `source` 区分 `live`、`durable`、`legacyDurable`
- `agent` 只描述 agent/session/storage 绑定，不和 lineage 混在一起

## Affected Crates And Files

| Crate | Files | Required change |
|------|-------|-----------------|
| `astrcode-core` | `src/agent/mod.rs` | 新增 `SubRunDescriptor` |
| `astrcode-core` | `src/event/types.rs` | `SubRunStarted` / `SubRunFinished` 增加 `descriptor`、`tool_call_id` |
| `astrcode-core` | `src/event/domain.rs` | 同步 domain event 结构 |
| `astrcode-core` | `src/event/translate.rs` | durable -> domain 映射新字段 |
| `astrcode-runtime` | `src/service/execution/subagent.rs` | 生命周期事件写入新字段 |
| `astrcode-runtime-execution` | `src/subrun.rs` | replay 改为优先解析 `descriptor`，并回传 `source` |
| `astrcode-protocol` | `src/http/event.rs` | 增加 `SubRunDescriptorDto`、事件 payload 新字段 |
| `astrcode-protocol` | `src/http/agent.rs` | 重塑 `SubRunStatusDto` 与 `SubRunStatusSourceDto` |
| `astrcode-server` | `src/http/mapper.rs` | 新字段映射；status source 映射 |
| `frontend` | `src/types.ts`、`src/lib/agentEvent.ts`、`src/lib/subRunView.ts` | 使用新 payload，不再依赖 `parentTurnId -> turnOwner` 推导 |

## Serialization And Compatibility Strategy

### Durable logs

- 新 runtime 只写新 schema，不做 dual-write。
- 新 runtime 读取旧 schema 时，`descriptor` 与 `tool_call_id` 允许缺省。
- 不提供批量回填脚本，不尝试把旧日志推断回新 descriptor。

### Protocol compatibility

- 旧前端读取新 status DTO 不在本次兼容目标内。
- 前后端同分支同步升级。
- `subRunStarted` / `subRunFinished` payload 的新字段对新前端是强依赖，对旧前端不保兼容。

### Legacy read behavior

- 若 lifecycle event 缺少 `descriptor`，解析结果返回 `descriptor = None`。
- 若 lifecycle event 缺少 `tool_call_id`，解析结果返回 `tool_call_id = None`。
- status API 的 `source` 取值为 `legacyDurable`。
- lineage 依赖型 scope 过滤直接失败，不伪造 ancestry。

## Replay And Query Impact

### `runtime-execution::find_subrun_status_in_events`

当前实现从 `agent` 上读取 `sub_run_id`，并在 `build_replayed_handle()` 中硬编码 `depth=1`、`parent_agent_id=None`。改造后：

1. 若 `descriptor` 存在，则用 `descriptor` 构建 lineage。
2. 若 `descriptor` 缺失，则返回 `source=legacyDurable` 且 `descriptor=None`。
3. `tool_call_id` 作为单独 durable 字段参与 status 输出。

### `server` 的 `/history` 与 `/events`

协议切完之后，server 可以只依赖 lifecycle event 构建 `ExecutionLineageIndex`，不再需要从普通事件顺序推父子关系。

### `frontend` 的 subrun tree

frontend 仍然从 `/history + /events` 生成 read model，但 parent/child 关系改为：

- child lifecycle event 的 `descriptor.parentAgentId`
- lifecycle event 的 `agent.agentId`
- 普通消息只用 `subRunId` 归类，不再承担 ancestry 推断职责

## Write Path Impact

`crates/runtime/src/service/execution/subagent.rs` 的写入点必须同时提供：

- child agent context
- `SubRunDescriptor`
- `tool_call_id`

其中 `tool_call_id` 来源于触发 `spawnAgent` 的 tool invocation，上游已有该值沿 tool execution chain 传递，只是还没有落到 subrun lifecycle event。

## Tests That Must Change

- `crates/protocol/tests/subrun_event_serialization.rs`
- `crates/runtime-execution/src/subrun.rs` 的解析测试
- `crates/runtime/src/service/execution/tests.rs`
- `crates/server/src/tests/runtime_routes_tests.rs`
- `frontend/src/lib/agentEvent.test.ts`
- `frontend/src/lib/subRunView.test.ts`

## Implementation Checklist

- [x] `SubRunDescriptor` 与 `SubRunStarted`/`SubRunFinished` 的字段设计已与 `data-model.md` 对齐。
- [x] durable -> domain -> protocol 映射链路已在文档中给出一一对应的 crate/file 清单。
- [x] `legacyDurable` 降级语义与 contracts 中的 scope/status 规则保持一致。
- [x] 本文涉及的实现范围已与 `tasks.md`（T004-T019）映射一致。

## Explicitly Out Of Scope For This Slice

- 不在这一刀里删除 `session_service.rs` / `execution_service.rs`
- 不在这一刀里重写五个边界的 compile-time 依赖
- 不在这一刀里引入新的持久化文件格式或迁移脚本

