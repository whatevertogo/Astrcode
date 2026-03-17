# Repository Guidelines

## Project Structure & Module Organization

```
AstrCode/
├── crates/
│   ├── core/        # 纯领域类型、Tool trait、ToolContext、AgentEvent
│   ├── contracts/   # HTTP / SSE DTO
│   ├── agent/       # 会话生命周期、JSONL 日志、配置、事件广播
│   ├── tools/       # Tool 实现，不依赖 agent
│   └── server/      # Axum 本地 server，唯一业务入口
├── src-tauri/       # Tauri 薄壳：sidecar 管理、窗口控制、宿主 GUI 桥接
└── frontend/        # React + TypeScript + Vite UI，共用桌面端和浏览器端
```

## Key Files Quick Reference

| 文件                                | 用途                                         |
| ----------------------------------- | -------------------------------------------- |
| `crates/server/src/main.rs`         | 本地 HTTP/SSE server、认证、路由与启动 token |
| `crates/agent/src/service/mod.rs`   | `AgentService` 门面与 service 子模块入口     |
| `crates/agent/src/tool_registry.rs` | 冻结后的只读 `ToolRegistry`                  |
| `frontend/src/hooks/useAgent.ts`    | 统一的 fetch + EventSource 客户端            |
| `src-tauri/src/main.rs`             | sidecar 启动、bootstrap 注入、退出清理       |

## Build, Test, and Development Commands

```bash
# Development (Tauri 壳 + 前端 dev server，桌面端会自行拉起本地 server)
cargo tauri dev

# Browser-only local server
cargo run -p astrcode-server

# Production build
cargo tauri build

# Run all Rust tests
cargo test --workspace

# Workspace check
cargo check --workspace

# Dependency boundary checks
cargo deny check bans

# Frontend development
cd frontend && npm run dev

# Frontend type checking
cd frontend && npm run typecheck
```

## Workflow Checklist

- Rust 代码改动：`cargo fmt --all -- --check && cargo test --workspace`
- 前端代码改动：`cd frontend && npm run typecheck && npm run lint && npm run format:check`
- 依赖边界改动（`Cargo.lock`/`deny.toml`）：补跑 `cargo deny check bans`
- 同时改 Rust 与前端：以上检查都要过

## Coding Style & Naming Conventions

### Rust

- Use `cargo fmt --all` before committing
- Follow standard Rust naming: `snake_case` for functions/variables, `PascalCase` for types
- Async functions should return `anyhow::Result<T>`

### TypeScript/React

- Components: `PascalCase.tsx` (e.g., `MessageList.tsx`)
- Hooks: `use*.ts` (e.g., `useAgent.ts`)
- Utilities: `camelCase.ts`
- Run `npm run typecheck`, `npm run lint`, and `npm run format:check` before committing

## Testing Guidelines

- Rust tests use built-in `#[test]` and `#[tokio::test]` attributes
- Test files are colocated with source files in `#[cfg(test)]` modules
- Run full suite: `cargo test --workspace`

## Architecture Notes

### Server Is The Truth

- 所有会话、配置、模型、事件流业务入口只通过 `crates/server` 暴露的 HTTP / SSE API。
- 前端和 Tauri 都不得直接调用 `agent`；Tauri 只保留窗口控制与宿主 GUI 能力。

### Session / Event Model

- 会话持久化在 `~/.astrcode/sessions/session-*.jsonl`。
- JSONL 采用 append-only `StoredEvent { storage_seq, event }`；`storage_seq` 由会话 writer 独占分配。
- `GET /api/sessions/:id/events` 先通过 `SessionReplaySource` 回放历史，再实时订阅广播；SSE 事件 id 形如 `{storage_seq}.{subindex}`。

### Tool System

- `Tool` trait 和 `ToolContext` 定义在 `crates/core`。
- `ToolRegistryBuilder` 在 server 启动时组装工具，`build()` 后冻结为只读 `ToolRegistry` 并转移给 `AgentService`。
- 所有工具必须基于 `ToolContext.working_dir` / `sandbox_root` 工作；禁止读取或修改进程级 cwd。

**Tool Error Semantics:**

- `Err(anyhow::Error)` → 系统级失败（IO 错误、参数解析失败、取消）
- `ToolExecutionResult { ok: false }` → 工具级拒绝（安全策略、需用户确认）

## Development Tips

### 环境与配置
- **配置文件**: `~/.astrcode/config.json`（API 密钥、Profile），`run.json`（port/token/pid）
- **ASTRCODE_HOME_DIR**: 用户 home 根目录，应用数据仍在 `.astrcode/...` 下
- **async_trait**: 默认要求 `Send`；非 `Send` 回调用 `#[async_trait(?Send)]`

### 前端/浏览器
- **首屏加载**: 先调 `/api/sessions/:id/messages`，再用 `x-session-cursor` 头连 SSE
- **会话列表**: `/api/sessions` 已按 `updated_at` 倒序，前端不二次排序
- **开发态 API**: 保持同源 `/api` 交给 Vite 代理，`/__astrcode__/run-info` 仅用于读取 token

### Tauri/桌面端
- **Sidecar**: 文件名需带 `-${TAURI_ENV_TARGET_TRIPLE}` 后缀，通过 `scripts/tauri-frontend.js` 构建
- **Bootstrap 时序**: Vite 先启动，sidecar 后启动；首个 API 请求须等 `window.__ASTRCODE_BOOTSTRAP__` 注入
- **HTTP 桥接**: 鉴权用 bootstrap token，调 origin/CSP 时同步更新 CORS 白名单
- **Windows 命令**: 不用 `npm.ps1`，用 `node` 脚本或 `npm.cmd` 启动前端

### 调试陷阱
- **Windows Home 目录测试**: `dirs::home_dir()` 不受临时环境变量影响；用 `test_support::test_home_dir()` / `TestEnvGuard`
- **PR 评论修复前**: 先查 `git status --short` 和 `git diff`，避免覆盖未提交改动
- **排查问题前**: 检查关键文件（`main.rs`、`Cargo.toml`）是否被误删，排除”运行旧版本”误判
- **AgentService 拆分后**: `crates/agent/src/service/` 已按 `types` / `config_ops` / `session_ops` / `turn_ops` / `replay` / `session_state` 拆分；新增逻辑先确认职责落点，避免把配置 façade、session runtime、事件翻译重新塞回 `mod.rs`
- **善意提醒已落实**: 这里曾出现过 `AgentService` 在单文件里同时承载配置 façade、会话运行态、事件翻译和测试的职责混杂；后续看到单文件再次快速膨胀时，要立刻提醒协作开发者先做边界整理，再继续堆逻辑
