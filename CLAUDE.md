# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**AstrCode** is a local terminal agent application (AI-powered coding agent) built with:
- **Core Runtime**: Rust (crates/core)
- **Desktop Framework**: Tauri v2 (src-tauri)
- **Frontend**: React + TypeScript + Vite (frontend)
- **IPC Protocol**: NDJSON over stdin/stdout (crates/ipc)

## Build Commands

```bash
# Development (runs frontend dev server + Tauri)
cargo tauri dev

# Production build
cargo tauri build

# Run all tests
cargo test --workspace

# Run tests for specific crate
cargo test -p astrcode-core

# Frontend only
cd frontend && npm run dev
```

## Architecture

### Workspace Structure
- `crates/core` - Main agent logic (AgentLoop, tools, LLM providers)
- `crates/ipc` - IPC event/command types for frontend-backend communication
- `src-tauri` - Tauri desktop application entry point
- `frontend` - React/TypeScript UI

### Key Patterns

**Agent Loop** (`crates/core/src/agent_loop.rs`):
- Turn-based execution: prompt → LLM call → tool execution → repeat
- Max 8 steps per turn
- Cancellation via `CancellationToken`
- Emits events: `PhaseChanged`, `ModelDelta`, `ToolCallStart`, `ToolCallResult`, `TurnDone`

**Tool System** (`crates/core/src/tools/`):
- Trait-based: `Tool` trait with `definition()` and `execute()` methods
- Registry pattern: `ToolRegistry` manages registration and execution
- Built-in tools: `shell`, `readFile`, `listDir`

**LLM Provider** (`crates/core/src/llm/`):
- Trait: `LlmProvider` with async `complete()` method
- OpenAI-compatible client (default: DeepSeek API)

### IPC Protocol (`crates/ipc/src/lib.rs`)

Events (backend → frontend):
- `SessionStarted`, `PhaseChanged`, `ModelDelta`, `ToolCallStart`, `ToolCallResult`, `TurnDone`, `Error`

Commands (frontend → backend):
- `SubmitPrompt`, `Interrupt`, `Exit`

## Configuration

Config file location: `{config_dir}/astrcode/config.json`

Example config:
```json
{
  "profiles": [{
    "name": "default",
    "providerKind": "openai-compatible",
    "baseUrl": "https://api.deepseek.com",
    "apiKey": "DEEPSEEK_API_KEY",
    "models": ["deepseek-chat"]
  }]
}
```

- `apiKey` can be a literal key or environment variable name
- Default LLM: DeepSeek API (`https://api.deepseek.com`)

## Shell Tool Notes

On Windows, the shell tool auto-detects:
- `powershell.exe` if available
- Falls back to `pwsh.exe` if PowerShell Core is installed
- Uses `sh` on Unix systems
