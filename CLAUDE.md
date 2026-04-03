# Repository Guidelines

## Workflow Checklist

- pre-commit：format、lint fix、大文件/冲突标记/密钥拦截
- pre-push：`cargo check --workspace && cargo test --workspace --exclude astrcode --lib && cd frontend && npm run typecheck`
- CI 完整检查：`cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace --exclude astrcode && cd frontend && npm run typecheck && npm run lint && npm run format:check`
- 依赖边界改动（`Cargo.lock`/`deny.toml`）：补跑 `cargo deny check bans`
- CI 配置 4 个工作流（`rust-check` / `frontend-check` / `tauri-build` / `dependency-audit`），推 `master` 或开 PR 自动触发

## Crate 依赖关系

```
protocol (纯 DTO，无业务依赖)
    ↑
  core (核心契约：Tool trait、Policy、Event/持久化接口)
    ↑
  storage     tools          runtime-config  runtime-llm  runtime-prompt  plugin
  (JSONL持久化) (内置工具)    (配置)           (LLM)        (Prompt)       (插件宿主)
    ↑            ↑                ↑              ↑            ↑              ↑
    +────────── runtime (RuntimeService 门面) ─────────────────────────────────+
                                   ↑
                                server (HTTP/SSE API)
                                   ↑
                               src-tauri (桌面端壳)
```

**依赖规则：**
- `protocol` 不得依赖 `core`/`runtime`；跨边界走显式 DTO + mapper
- `tools` 仅依赖 `core`，不依赖 `runtime`
- `storage` 实现持久化（`EventLog`、`FileSystemSessionRepository`）；`core` 只定义接口
- `runtime-prompt`/`runtime-llm`/`runtime-config` 保持编译隔离，`runtime` 作为门面组合，不重复实现
- 环境变量常量源头在 `crates/core/src/env.rs`，`runtime-config/src/constants.rs` 聚合导出

## Architecture Rules

- **Server Is The Truth**：所有业务入口只走 `crates/server` 的 HTTP/SSE API；前端和 Tauri 不直接调 `runtime`
- **Server 入口瘦身**：`main.rs` 只保留启动装配，新逻辑落到 `routes/`、`mapper.rs`、`bootstrap.rs`
- **不需要向后兼容**，以最佳架构和代码风格书写，追求 0 技术债

## Session / Event / Tool

- 配置：`~/.astrcode/config.json`；会话：`~/.astrcode/projects/<project>/sessions/<id>/session-*.jsonl`
- JSONL append-only `StoredEvent { storage_seq, event }`；`storage_seq` 由 writer 独占分配
- SSE 端点先 replay 再实时订阅；事件 id 形如 `{storage_seq}.{subindex}`
- 工具增量输出（`ToolCallDelta`）必须先持久化再广播
- diff/shell metadata 解析收口到 `frontend/src/lib/`

## Skill Architecture

- 两阶段：system prompt 只暴露索引（`name` + `description`），正文通过 `Skill` tool 按需加载
- `SKILL.md` frontmatter 只认 `name`/`description`，`name` 须与文件夹名一致（kebab-case）
- builtin skill 资源由 `crates/runtime-prompt/build.rs` 打包，不手写 `include_str!`

## Gotchas

- **Sidecar 文件名**：须带 `-${TAURI_ENV_TARGET_TRIPLE}` 后缀
- **多实例**：新 exe 复用 `~/.astrcode/run.json` 指向的 server，共享事件流
- **Bootstrap 时序**：Vite 先启动，sidecar 后启动；首请求等 `window.__ASTRCODE_BOOTSTRAP__`
- **Windows WebView**：用 `initialization_script(...)` 注入 bootstrap，不要在 `setup()` 里同步 `eval`
- **Sidecar 锁文件**：桌面端复制 server 到 `~/.astrcode/runtime/sidecars/` 再启动，避免 Windows 锁 exe
- **Windows 测试**：用 `test_support::test_home_dir()` / `TestEnvGuard`，`dirs::home_dir()` 不受临时环境变量影响
- **Windows 启动前端**：用 `node` 脚本或 `npm.cmd`，不用 `npm.ps1`
