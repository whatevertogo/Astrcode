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
