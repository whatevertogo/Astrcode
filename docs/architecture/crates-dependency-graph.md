# AstrCode Architecture

## Crate Dependency Graph

```
protocol (纯 DTO，零业务依赖)
    ↑
 core (核心契约：Event/Policy/Capability/Tool trait + 持久化接口、Agent 生命周期)
    ↑
 storage       runtime-tool-loader  runtime-config  runtime-llm  runtime-prompt  runtime-skill-loader  runtime-registry  plugin  sdk
 (JSONL持久化)  (内置工具集)          (配置管理)      (LLM适配)    (Prompt组装)    (Skill发现加载)       (能力路由)       (宿主)  (SDK)
    ↑                ↑                   ↑              ↑            ↑                ↑                    ↑             ↑       ↑
    +────────────────+───────────────────+──────────────+────────────+────────────────+────────────────────┼─────────────┼───────┘
                                                       ↑                                                                         ↑
                                runtime-session ───────┼─────────────────────────────────────────────────────────────────────────┘
                                runtime-execution ─────┤
                                runtime-agent-control ─┤
                                runtime-agent-loader ───┤
                                runtime-agent-loop ─────┤
                                runtime-agent-tool ─────┤
                                                          ↑
                                     runtime (RuntimeService 门面，assembly + bootstrap)
                                                          ↑
                                                     server (HTTP/SSE API)
                                                          ↑
                                                     src-tauri (桌面壳)
```

### 完整依赖表

| Crate | Path | 依赖的 workspace crate |
|-------|------|----------------------|
| `protocol` | crates/protocol | 无（叶子节点） |
| `core` | crates/core | `protocol` |
| `storage` | crates/storage | `core` |
| `runtime-tool-loader` | crates/runtime-tool-loader | `core` |
| `runtime-config` | crates/runtime-config | `core` |
| `runtime-llm` | crates/runtime-llm | `core` |
| `runtime-prompt` | crates/runtime-prompt | `core` |
| `runtime-skill-loader` | crates/runtime-skill-loader | `core` |
| `runtime-registry` | crates/runtime-registry | `core` |
| `plugin` | crates/plugin | `core`, `protocol` |
| `sdk` | crates/sdk | `protocol` |
| `runtime-session` | crates/runtime-session | `core`, `runtime-agent-control`, `runtime-agent-loop` |
| `runtime-execution` | crates/runtime-execution | `core`, `runtime-config`, `runtime-prompt`, `runtime-registry` |
| `runtime-agent-control` | crates/runtime-agent-control | `core`, `runtime-config` |
| `runtime-agent-loader` | crates/runtime-agent-loader | `core` |
| `runtime-agent-loop` | crates/runtime-agent-loop | `core`, `protocol`, `plugin`, `runtime-config`, `runtime-llm`, `runtime-prompt`, `runtime-registry`, `runtime-skill-loader` |
| `runtime-agent-tool` | crates/runtime-agent-tool | `core` |
| `runtime` | crates/runtime | `core`, `protocol`, `plugin`, `runtime-agent-control`, `runtime-agent-loader`, `runtime-agent-loop`, `runtime-agent-tool`, `runtime-config`, `runtime-execution`, `runtime-llm`, `runtime-prompt`, `runtime-registry`, `runtime-session`, `runtime-skill-loader`, `runtime-tool-loader`, `storage` |
| `server` | crates/server | `core`, `protocol`, `runtime`, `runtime-registry` |

## Build & Verification

```bash
# pre-commit
cargo fmt --all --check

# pre-push
cargo check --workspace && cargo test --workspace --exclude astrcode --lib

# 完整 CI 检查
cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace --exclude astrcode
```

详细依赖边界规则见 [AGENTS.md](../../AGENTS.md)。
