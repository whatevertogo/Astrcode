# Repository Guidelines

## Project Structure & Module Organization

```
AstrCode/
├── crates/
│   ├── core/        # Agent logic: AgentLoop, tools, LLM providers
│   └── ipc/         # IPC event/command types (NDJSON protocol)
├── src-tauri/       # Tauri desktop application entry point
└── frontend/        # React + TypeScript + Vite UI
```

## Key Files Quick Reference

| 文件 | 用途 |
|------|------|
| `crates/core/src/agent_loop.rs` | 核心执行循环：LLM 调用 + 工具执行 |
| `crates/core/src/tools/mod.rs` | Tool trait 定义和错误语义 |
| `crates/ipc/src/lib.rs` | IPC 事件/命令类型定义 |
| `frontend/src/hooks/useAgent.ts` | 前端 Agent 事件处理 |

## Build, Test, and Development Commands

```bash
# Development (frontend dev server + Tauri)
cargo tauri dev

# Production build
cargo tauri build

# Run all Rust tests
cargo test --workspace

# Run tests for specific crate
cargo test -p astrcode-core

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
- Run `npm run typecheck` before committing

## Testing Guidelines

- Rust tests use built-in `#[test]` and `#[tokio::test]` attributes
- Test files are colocated with source files in `#[cfg(test)]` modules
- Run full suite: `cargo test --workspace`

## Architecture Notes

### Agent Loop Pattern
Turn-based execution in `crates/core/src/agent_loop.rs`:
1. Receive prompt
2. Call LLM
3. Execute tools
4. Emit events
5. Repeat (max 8 steps)

### IPC Protocol
Backend → Frontend events: `SessionStarted`, `PhaseChanged`, `ModelDelta`, `ToolCallStart`, `ToolCallResult`, `TurnDone`, `Error`

Frontend → Backend commands: `SubmitPrompt`, `Interrupt`, `Exit`

### Tool System
Trait-based design in `crates/core/src/tools/`:
- `Tool` trait with `definition()` and `execute()` methods
- `ToolRegistry` manages registration and execution

**Tool Error Semantics:**
- `Err(anyhow::Error)` → 系统级失败（IO 错误、参数解析失败、取消）
- `ToolExecutionResult { ok: false }` → 工具级拒绝（安全策略、需用户确认）

## Development Tips

- **async_trait**: 默认 `#[async_trait]` 要求 `Send`；若需非 `Send` 回调，用 `#[async_trait(?Send)]`
- **配置文件**: 首次运行生成 `~/.astrcode/config.json`，含 API 密钥和 Profile 配置
- **IPC 协议**: 后端→前端事件通过 NDJSON 流传输，见 `AgentEventKind` 枚举
- **Home 目录测试陷阱**: 在 Windows 测试环境里，`dirs::home_dir()` 不一定受临时 `HOME/USERPROFILE` 影响；需要可控 home 路径的测试或模块，优先复用 `crate::test_support::test_home_dir()` / `TestEnvGuard`
