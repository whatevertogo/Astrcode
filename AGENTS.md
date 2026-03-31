# Repository Guidelines

## Key Files Quick Reference

| 文件                                           | 用途                                         |
| ---------------------------------------------- | -------------------------------------------- |
| `crates/server/src/main.rs`                    | 本地 HTTP/SSE server 启动入口与状态装配      |
| `crates/server/src/routes/`                    | HTTP/SSE 路由处理器                          |
| `crates/runtime/src/service/mod.rs`            | `RuntimeService` 门面与运行态入口            |
| `crates/core/src/registry/tool.rs`             | 冻结后的只读 `ToolRegistry`                  |
| `crates/protocol/src/http/`                    | HTTP/SSE DTO 与协议边界                      |
| `crates/runtime-config/src/lib.rs`             | 配置模型与加载/校验逻辑（独立 crate）        |
| `crates/runtime-llm/src/`                      | LLM 提供者抽象与 OpenAI/Anthropic 适配       |
| `crates/runtime-prompt/src/composer.rs`        | Prompt 组装引擎与 Contributor 调度           |
| `crates/runtime-prompt/src/contributors/`      | 系统提示各片段贡献者（identity/agents_md 等）|
| `crates/tools/src/tools/fs_common.rs`          | 文件工具共享的 diff 计算与输出格式化         |
| `frontend/src/hooks/useAgent.ts`               | 统一的 fetch + EventSource 客户端            |
| `frontend/src/components/Chat/ToolCallBlock.tsx`| 工具调用结果渲染（含 diff 可视化）           |
| `src-tauri/src/main.rs`                        | sidecar 启动、bootstrap 注入、退出清理       |
| `.github/workflows/`                           | CI 工作流：rust-check / frontend-check / tauri-build / dependency-audit |


## Workflow Checklist

- Rust 代码改动：`cargo fmt --all -- --check && cargo test --workspace --exclude astrcode`
- 前端代码改动：`cd frontend && npm run typecheck && npm run format:check`（lint 当前无效，见调试陷阱）
- 依赖边界改动（`Cargo.lock`/`deny.toml`）：补跑 `cargo deny check bans`
- 同时改 Rust 与前端：以上检查都要过
- CI 已配置 4 个工作流（`rust-check` / `frontend-check` / `tauri-build` / `dependency-audit`），推送到 `master` 或开 PR 自动触发

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

### Crate 依赖关系

```
protocol (纯 DTO，无业务依赖)
    ↑
  core (持久化状态、Tool trait、EventLog、SessionStore)
    ↑
  tools (基于 core 的 Tool 实现)
    ↑
  runtime-config (配置加载/校验)    runtime-llm (LLM 提供者适配)    runtime-prompt (Prompt 组装引擎)
    ↑                                   ↑                                ↑
    +───────────────────── runtime (RuntimeService 门面) ─────────────────+
                                       ↑
                                    server (HTTP/SSE API)
                                       ↑
                                   src-tauri (桌面端壳)
```

- `protocol` 不得依赖 `core`/`runtime`；跨边界数据走显式 DTO + mapper。
- `runtime-prompt`、`runtime-llm`、`runtime-config` 为从 `runtime` 拆分出的独立 crate，保持编译隔离。
- `runtime` 作为门面组合上述三者，不重复实现具体逻辑。
- `tools` 仅依赖 `core`，不直接依赖 `runtime`。

### Server Is The Truth

- 所有会话、配置、模型、事件流业务入口只通过 `crates/server` 暴露的 HTTP / SSE API。
- 前端和 Tauri 都不得直接调用 `runtime`；Tauri 只保留窗口控制与宿主 GUI 能力。

### Session / Event Model

- 会话持久化在 `~/.astrcode/sessions/session-*.jsonl`。
- JSONL 采用 append-only `StoredEvent { storage_seq, event }`；`storage_seq` 由会话 writer 独占分配。
- `GET /api/sessions/:id/events` 先通过 `SessionReplaySource` 回放历史，再实时订阅广播；SSE 事件 id 形如 `{storage_seq}.{subindex}`。

### Tool System

- `Tool` trait 和 `ToolContext` 定义在 `crates/core`。
- `ToolRegistryBuilder` 在 server 启动时组装工具，`build()` 后冻结为只读 `ToolRegistry` 并转移给 `RuntimeService`。
- 所有工具必须基于 `ToolContext.working_dir` 工作；禁止读取或修改进程级 cwd。

**Tool Error Semantics:**

- `Err(anyhow::Error)` → 系统级失败（IO 错误、参数解析失败、取消）
- `ToolExecutionResult { ok: false }` → 工具级拒绝（安全策略、需用户确认）

**Tool Diff 可视化:**

- `edit_file` 和 `write_file` 工具通过 `fs_common::compute_diff()` 生成 unified diff 输出。
- diff 结果随 `ToolResult` 返回前端，`ToolCallBlock.tsx` 负责渲染（含语法高亮）。
- 前端通过 `useAgent.ts` 的 `tool_result` 事件接收 diff 内容。

### Prompt System

- `runtime-prompt` crate 负责系统提示组装，采用 Contributor 模式。
- `PromptComposer` 按优先级调度各 `PromptContributor`，拼装最终系统提示。
- 已有 Contributor：`IdentityContributor`（用户身份）、`AgentsMdContributor`（项目指令）、`EnvironmentContributor`（环境信息）、`SkillSummaryContributor`（技能摘要）。
- 每个 Contributor 实现 `PromptContributor` trait，返回 `PromptContribution`（含优先级和内容块）。

### Identity System

- 用户可在 `~/.astrcode/IDENTITY.md` 创建自定义身份文件，定义 Agent 的角色、偏好和约束。
- `IdentityContributor` 在 Prompt 组装时读取该文件，将内容注入系统提示。
- 项目级的 `IDENTITY.md` 也可放在 `crates/runtime-prompt/src/contributors/IDENTITY.md` 作为默认模板。

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
- **代码审查单可能滞后**: 处理 review / issue 清单前，先打开对应文件确认问题在当前仓库里仍然存在；这里已经出现过审查结论落后于实现的情况（例如工具路径边界、turn 竞态、会话尾部扫描等项已先被修复）
- **Runtime / Core 边界**: 可持久化状态、投影、注册表、事件存储放在 `crates/core`；`RuntimeService` 只持有进程内运行态（broadcast、cancel、活动 session），不要把运行态重新下沉进 core
- **Protocol 独立**: `crates/protocol` 不得依赖 `core/runtime`；所有跨边界数据都通过显式 DTO 和 mapper 转换，避免重新出现“共享内部类型就是协议”的耦合
- **Server 入口瘦身后**: `crates/server/src/main.rs` 只保留启动与装配；新增逻辑优先落到 `routes/`、`mapper.rs`、`bootstrap.rs`，避免重新把 HTTP 路由、DTO 转换、静态资源托管全部堆回入口
- **当前 `cargo test --workspace` 在本机可能被 Tauri sidecar 权限拦截**: `src-tauri` 的 build script 访问 `binaries/astrcode-server-<triple>.exe` 时可能报 `Os { code: 5, kind: PermissionDenied }`。改 Rust 代码时先用 `cargo test --workspace --exclude astrcode` 验证业务 crate，再单独排查桌面端打包权限。
- **当前 `frontend` 的 `npm run lint` 脚本不会真正 lint `src/`**: 现有 ESLint 配置会把传入的 `src` 整体视为 ignored，命令只返回 ignored warning。前端改动至少补跑 `npm run typecheck` 和 `npm run format:check`，并在修复 ESLint 配置前不要把 `npm run lint` 的成功与否当成有效校验。
