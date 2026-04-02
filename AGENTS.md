# Repository Guidelines


## Workflow Checklist

- Rust 代码改动：`cargo fmt --all -- --check && cargo test --workspace --exclude astrcode`
- 前端代码改动：`cd frontend && npm run typecheck && npm run lint && npm run format:check`
- 依赖边界改动（`Cargo.lock`/`deny.toml`）：补跑 `cargo deny check bans`
- 同时改 Rust 与前端：以上检查都要过
- CI 已配置 4 个工作流（`rust-check` / `frontend-check` / `tauri-build` / `dependency-audit`），推送到 `master` 或开 PR 自动触发

## Coding Style & Naming Conventions

### Rust

- Use `cargo fmt --all`,`cargo clippy --all-targets --all-features -- -D warnings` before committing
- Follow standard Rust naming: `snake_case` for functions/variables, `PascalCase` for types
- Async functions should return `anyhow::Result<T>`

## Architecture Notes

### Crate 依赖关系

```
protocol (纯 DTO，无业务依赖)
    ↑
  core (核心契约：Tool trait、Policy、Event 接口、持久化接口)
    ↑
  storage (JSONL 会话持久化实现)
  tools (内置工具)    runtime-config (配置)    runtime-llm (LLM)    runtime-prompt (Prompt)    plugin (插件宿主)
    ↑                     ↑                       ↑                     ↑                        ↑
    +────────────── runtime (RuntimeService 门面) ──────────────────────────────────────────────────+
                                       ↑
                                    server (HTTP/SSE API)
                                       ↑
                                   src-tauri (桌面端壳)
```

- `protocol` 不得依赖 `core`/`runtime`；跨边界数据走显式 DTO + mapper。
- `runtime-prompt`、`runtime-llm`、`runtime-config` 为从 `runtime` 拆分出的独立 crate，保持编译隔离。
- `runtime` 作为门面组合上述三者，不重复实现具体逻辑。
- `tools` 仅依赖 `core`，不直接依赖 `runtime`。
- `storage` 从 `core` 提取持久化实现（`EventLog`、`FileSystemSessionRepository`）；`core` 只定义接口（`EventLogWriter`、`SessionManager`）。

### Server Is The Truth

- 所有会话、配置、模型、事件流业务入口只通过 `crates/server` 暴露的 HTTP / SSE API。
- 前端和 Tauri 都不得直接调用 `runtime`；Tauri 只保留窗口控制与宿主 GUI 能力。

### Session / Event Model

- 全局配置保持在 `~/.astrcode/config.json`；会话按项目落在 `~/.astrcode/projects/<project>/sessions/<session-id>/session-*.jsonl`。
- JSONL 采用 append-only `StoredEvent { storage_seq, event }`；`storage_seq` 由会话 writer 独占分配。
- `GET /api/sessions/:id/events` 先通过 `SessionReplaySource` 回放历史，再实时订阅广播；SSE 事件 id 形如 `{storage_seq}.{subindex}`

### Tool Event / UX

- 工具事件除了 `ToolCall` / `ToolResult`，现在还有 `ToolCallDelta`；长耗时工具的增量输出必须先持久化再广播，不能只留在前端本地状态。
- `shell` 工具按 stdout / stderr 增量流式输出；前端工具卡片会基于 `metadata.display.kind = terminal` 渲染终端视图，断线重连后通过 replay 恢复。
- `writeFile` / `editFile` 的 diff 仍通过 metadata 驱动展示；diff/shell metadata 的解析逻辑优先收口到 `frontend/src/lib/`，避免在 `ToolCallBlock` 里散落协议细节。

### Skill Architecture

- Claude 风格 skill 走两阶段模型：system prompt 只暴露 skill 索引（`name` + `description`），真正的正文通过内置 `Skill` tool 按需加载。
- `SKILL.md` frontmatter 只认 `name` 和 `description`，且 `name` 必须与文件夹名一致（kebab-case）；不要再往 markdown frontmatter 里塞 Astrcode 专用执行元数据。
- skill 目录整体都是资源面，`references/`、`scripts/` 等资产会被索引并随 `Skill` tool 一起暴露；builtin skill 的整目录资源由 `crates/runtime-prompt/build.rs` 打包，不要再手写 `include_str!` 清单。

## Development Tips

### Tauri/桌面端
- **Sidecar**: 文件名需带 `-${TAURI_ENV_TARGET_TRIPLE}` 后缀，通过 `scripts/tauri-frontend.js` 构建
- **桌面端多实例**: 新打开的 exe 会优先复用 `~/.astrcode/run.json` 指向的现有 server；只有没有可用实例时才会再起 sidecar，这样多个桌面实例才能共享同一会话事件流
- **Bootstrap 时序**: Vite 先启动，sidecar 后启动；首个 API 请求须等 `window.__ASTRCODE_BOOTSTRAP__` 注入
- **HTTP 桥接**: 鉴权用 bootstrap token，调 origin/CSP 时同步更新 CORS 白名单
- **Windows 命令**: 不用 `npm.ps1`，用 `node` 脚本或 `npm.cmd` 启动前端

### 调试陷阱
- **Windows Home 目录测试**: `dirs::home_dir()` 不受临时环境变量影响；用 `test_support::test_home_dir()` / `TestEnvGuard`
- **PR 评论修复前**: 先查 `git status --short` 和 `git diff`，避免覆盖未提交改动
- **Protocol 独立**: `crates/protocol` 不得依赖 `core/runtime`；所有跨边界数据都通过显式 DTO 和 mapper 转换，避免重新出现“共享内部类型就是协议”的耦合
- **Server 入口瘦身后**: `crates/server/src/main.rs` 只保留启动与装配；新增逻辑优先落到 `routes/`、`mapper.rs`、`bootstrap.rs`，避免重新把 HTTP 路由、DTO 转换、静态资源托管全部堆回入口
- **Tauri sidecar 运行时副本**: 桌面端现在会先把 `astrcode-server` 复制到 `~/.astrcode/runtime/sidecars/` 的唯一副本再启动，避免 Windows 把 `target/debug/astrcode-server.exe` 锁死导致后续构建失败。若仍遇到 `Os { code: 5, kind: PermissionDenied }`，先确认没有遗留的旧版 `astrcode-server.exe` 仍在运行

# 注意

- 项目自定义环境变量常量的底层源头放 `crates/core/src/env.rs`；`crates/runtime-config/src/constants.rs` 负责按 home / plugin / provider / build 分类聚合与对外导出，新增环境变量时不要散落硬编码
- 不需要向后兼容，尽量以最良好的架构和代码风格书写代码，尽量0技术债
