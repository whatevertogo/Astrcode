# Runtime 迁移基线（阶段 1）

本文档用于支撑 `tasks.md` 的 1.2 / 1.3，明确当前 `server -> runtime -> runtime-*` 的真实依赖链，以及 HTTP handler 对 `RuntimeService` 的直接调用面。

## 1. crate 依赖链基线（1.2）

## 1.1 `server` 直接依赖

来自 [crates/server/Cargo.toml](/D:/GitObjectsOwn/Astrcode/crates/server/Cargo.toml)：

- `astrcode-runtime`
- `astrcode-runtime-execution`
- `astrcode-runtime-registry`

来自 [crates/server/src/main.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/main.rs) 与 [crates/server/src/bootstrap/runtime.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/bootstrap/runtime.rs)：

- `AppState` 仍直接持有 `Arc<RuntimeService>`
- 运行时组合根 `bootstrap_server_runtime()` 仍委托 `astrcode_runtime::bootstrap_runtime()`
- server 仍直接使用 `RuntimeCoordinator` 与旧 runtime 桥接快照

## 1.2 `runtime` 向下依赖

来自 [crates/runtime/Cargo.toml](/D:/GitObjectsOwn/Astrcode/crates/runtime/Cargo.toml)：

- `astrcode-runtime-session`
- `astrcode-runtime-execution`
- `astrcode-runtime-agent-control`
- `astrcode-runtime-agent-loop`
- `astrcode-runtime-config`
- `astrcode-runtime-registry`
- `astrcode-adapter-*`（llm/prompt/mcp/storage/skills/agents/tools）

来自 [crates/runtime/src/service/mod.rs](/D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/mod.rs)：

- `RuntimeService` 直接持有 `SessionState`、`AgentControl`、`AgentLoop`、`CapabilityRouter`、`Config`、`MCP manager` 等跨层状态
- 这说明当前仍是“runtime 超级门面”，尚未完成“application 唯一用例边界”

## 2. handler 调用面基线（1.3）

统计范围：`crates/server/src/http/routes/**` 的真实 HTTP handler（不含测试代码）。

## 2.1 ServiceHandle 级别调用面

- `SessionServiceHandle`（8）
  - `list`
  - `create`
  - `history_filtered`
  - `view`
  - `compact`
  - `subscribe_catalog`
  - `history`
  - `replay`
- `AgentExecutionServiceHandle`（8）
  - `list_profiles`
  - `execute_root_agent`
  - `get_subrun_status`
  - `close_agent_subtree`
  - `submit_prompt`
  - `interrupt_session`
  - `delete_session`
  - `delete_project`
- `ConfigServiceHandle`（5）
  - `get_config`
  - `current_config_path`
  - `save_active_selection`
  - `reload_config_from_disk`
  - `test_connection`
- `ComposerServiceHandle`（1）
  - `list_composer_options`
- `McpServiceHandle`（8）
  - `list_status`
  - `approve_server`
  - `reject_server`
  - `reconnect_server`
  - `reset_project_choices`
  - `upsert_config`
  - `remove_config`
  - `set_enabled`

合计：`5` 个服务句柄、`30` 个直接调用方法。

## 2.2 路由文件定位

- [agents.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/agents.rs)
- [composer.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/composer.rs)
- [config.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/config.rs)
- [mcp.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/mcp.rs)
- [model.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/model.rs)
- [sessions/query.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/sessions/query.rs)
- [sessions/mutation.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/sessions/mutation.rs)
- [sessions/stream.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/sessions/stream.rs)

## 3. 迁移含义

- 在 8.5（handler 解耦）之前，`server` 无法删除 `RuntimeService`。
- 在 9.x 删除旧 runtime 体系前，必须先让 `application` 提供等价服务接口并完成 handler 全替换。
