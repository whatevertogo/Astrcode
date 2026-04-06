# Repository Guidelines


## Crate 依赖关系

```
protocol (纯 DTO，无业务依赖)
    ↑
  core (核心契约：Tool trait、Policy、Event/持久化接口)
    ↑
  storage     runtime-tool-loader  runtime-config  runtime-llm  runtime-prompt  plugin
  (JSONL持久化) (内置工具)          (配置)           (LLM)        (Prompt)       (插件宿主)
    ↑            ↑                ↑              ↑            ↑              ↑
    +────────── runtime (RuntimeService 门面) ─────────────────────────────────+
                                   ↑
                                server (HTTP/SSE API)
                                   ↑
                               src-tauri (桌面端壳)
```

**依赖规则：**
- `protocol` 不得依赖 `core`/`runtime`；跨边界走显式 DTO + mapper
- `runtime-...-loader` 仅依赖 `core`，不依赖 `runtime`
- `storage` 实现持久化（`EventLog`、`FileSystemSessionRepository`）；`core` 只定义接口
- `runtime-prompt`/`runtime-llm`/`runtime-config` 保持编译隔离，`runtime` 作为门面组合，不重复实现
- 环境变量常量源头在 `crates/core/src/env.rs`，`runtime-config/src/constants.rs`, 聚合导出

## Workflow Checklist

- pre-commit：format、lint fix、大文件/冲突标记/密钥拦截
- pre-push：`cargo check --workspace && cargo test --workspace --exclude astrcode --lib && cd frontend && npm run typecheck`
- CI 完整检查：`cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace --exclude astrcode && cd frontend && npm run typecheck && npm run lint && npm run format:check`
- 依赖边界改动（`Cargo.lock`/`deny.toml`）：补跑 `cargo deny check bans`
- CI 配置 4 个工作流（`rust-check` / `frontend-check` / `tauri-build` / `dependency-audit`），推 `master` 或开 PR 自动触发

## 注意

- 用中文注释，且注释尽量表明为什么和做了什么、
- 为了干净架构和良好实现可以不需要向后兼容，如果向后兼容需要说明为什么
- 最后需要cargo fmt --all --check  && cargo clippy --all-targets --all-features -- -D warnings && cargo test验证你的更改
