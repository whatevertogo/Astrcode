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

| 文件 | 用途 |
|------|------|
| `crates/server/src/main.rs` | 本地 HTTP/SSE server、认证、路由与启动 token |
| `crates/agent/src/service.rs` | `AgentService`、会话状态、广播与回放 |
| `crates/agent/src/tool_registry.rs` | 冻结后的只读 `ToolRegistry` |
| `frontend/src/hooks/useAgent.ts` | 统一的 fetch + EventSource 客户端 |
| `src-tauri/src/main.rs` | sidecar 启动、bootstrap 注入、退出清理 |

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

## Coding Style & Naming Conventions

### Rust
- Use `cargo fmt --all` before committing
- Follow standard Rust naming: `snake_case` for functions/variables, `PascalCase` for types
- Async functions should return `anyhow::Result<T>`

### TypeScript/React
- Components: `PascalCase.tsx` (e.g., `MessageList.tsx`)
- Hooks: `use*.ts` (e.g., `useAgent.ts`)
- Utilities: `camelCase.ts`
- Run `npm run typecheck` and `npm run lint` before committing

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

- **async_trait**: 默认 `#[async_trait]` 要求 `Send`；若需非 `Send` 回调，用 `#[async_trait(?Send)]`
- **配置文件**: 首次运行生成 `~/.astrcode/config.json`，含 API 密钥和 Profile 配置
- **run.json**: 本地 server 启动后写 `~/.astrcode/run.json`，包含 `port`、`token`、`pid`
- **前端首屏加载**: 先调 `GET /api/sessions/:id/messages`，再用响应头 `x-session-cursor` 连接 SSE，避免首连重复回放
- **会话列表排序**: `GET /api/sessions` 已按 `updated_at` 倒序返回，前端不得二次排序
- **Home 目录测试陷阱**: 在 Windows 测试环境里，`dirs::home_dir()` 不一定受临时 `HOME/USERPROFILE` 影响；需要可控 home 路径的测试或模块，优先复用 `crate::test_support::test_home_dir()` / `TestEnvGuard`
- **ASTRCODE_HOME_DIR 语义**: 该环境变量表示用户 home 根目录，不是应用数据目录；用户级文件路径都应继续拼接到 `.astrcode/...` 下，例如 `.astrcode/AGENTS.md`
- **Tauri 前端命令路径**: 当前环境里 `tauri.conf.json` 的 `beforeDevCommand` / `beforeBuildCommand` 按仓库根目录解析；在 Windows 上不要依赖 `npm.ps1`，优先通过 `node` 脚本或 `cmd.exe -> npm.cmd` 间接启动前端命令
- **Tauri sidecar 约束**: `bundle.externalBin` 的源文件名必须带 `-${TAURI_ENV_TARGET_TRIPLE}` 后缀；仓库里统一通过 `scripts/tauri-frontend.js` 先构建/复制 `astrcode-server` sidecar，再启动前端或打包
- **桌面端 HTTP 桥接**: 前端对本地 server 的鉴权依赖 bootstrap token 头/查询参数，而不是跨站 cookie；若调整 UI origin、serverOrigin 或 CSP，必须同步更新 `crates/server/src/main.rs` 的 CORS 白名单和 `src-tauri/tauri.conf.json` 的 `connect-src`
