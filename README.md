# AstrCode

AstrCode 是一个本地优先的 AI 编程助手。桌面端、浏览器端和终端都通过同一个 `astrcode-server` 访问会话、工具、Agent 协作、MCP、插件和配置能力。

当前处于 `v0.1.0-alpha` 实验阶段，不维护向后兼容；当旧边界影响架构清晰度时，优先删除旧实现。

- 发布下载：[GitHub Releases](https://github.com/whatevertogo/Astrcode/releases)
- 架构约束：[PROJECT_ARCHITECTURE.md](PROJECT_ARCHITECTURE.md)
- 路线图：[ROADMAP.md](ROADMAP.md)
- 贡献指南：[CONTRIBUTING.md](CONTRIBUTING.md)
- 安全策略：[SECURITY.md](SECURITY.md)

## 能力概览

- **统一 Server**：`astrcode-server` 是唯一组合根，暴露 HTTP/SSE API；Tauri 和 CLI 都不直接访问运行时内部状态。
- **多端入口**：Tauri 桌面壳、浏览器前端、ratatui CLI 共用同一套 server API。
- **OpenAI 兼容模型**：支持 OpenAI Responses、Chat Completions 和 DeepSeek 等兼容网关。
- **工具执行**：文件读写、精确编辑、搜索、shell、skill 加载、模式切换和 Agent 协作工具。
- **Agent 协作**：root agent / child agent / sub-run lineage 统一落到 session durable events。
- **Skill / MCP / Plugin**：Skill 多源覆盖，MCP 工具/资源/提示桥接，plugin-host 管理内置与外部贡献。
- **事件溯源**：session 以 JSONL 事件日志为权威事实，live SSE 只承载低延迟视图更新。
- **真实 API Eval**：`npm run eval:api` 会启动真实 server，让 LLM 通过 AstrCode agent/runtime 跑评测任务。

## 环境要求

- Rust **nightly**（见 `rust-toolchain.toml`）
- Node.js 20+
- npm

首次安装：

```bash
npm install
cd frontend && npm install
```

`npm install` 会尝试把 Git hook 指向 `.githooks/`：

- `pre-commit`：运行轻量格式、安全和依赖图更新检查。
- `pre-push`：运行 `cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib` 和前端 typecheck。

## 常用命令

```bash
# 桌面端开发
npm run dev:tauri

# 仅启动 server
cargo run -p astrcode-server

# 仅启动前端
cd frontend && npm run dev

# CLI
cargo run -p astrcode-cli

# 桌面端构建
npm run build
```

浏览器开发时，分别启动 server 与 Vite，然后打开 `http://127.0.0.1:5173/`。生产 server 会在 `frontend/dist` 存在时托管前端构建产物，并注入 bootstrap token。

## 配置

首次运行会在 `~/.astrcode/config.json` 创建默认配置。推荐使用环境变量保存 API key：

```json
{
  "version": "1",
  "activeProfile": "deepseek",
  "activeModel": "deepseek-chat",
  "runtime": {},
  "profiles": [
    {
      "name": "deepseek",
      "providerKind": "openai",
      "baseUrl": "https://api.deepseek.com",
      "apiKey": "env:DEEPSEEK_API_KEY",
      "apiMode": "chat_completions",
      "models": [
        {
          "id": "deepseek-chat",
          "maxTokens": 8096,
          "contextLimit": 128000
        }
      ]
    }
  ]
}
```

`apiKey` 支持：

- `env:DEEPSEEK_API_KEY`：从环境变量读取。
- `literal:VALUE`：按字面量读取，避免被误判为环境变量。
- `VALUE`：直接使用明文值，不推荐提交到仓库。

常用环境变量：

| 变量 | 说明 |
|---|---|
| `ASTRCODE_HOME_DIR` | 覆盖 AstrCode home 目录 |
| `ASTRCODE_TEST_HOME` | 测试隔离 home，优先级高于 `ASTRCODE_HOME_DIR` |
| `ASTRCODE_PLUGIN_DIRS` | 追加插件搜索目录 |
| `DEEPSEEK_API_KEY` | 默认 DeepSeek profile 的 key |
| `OPENAI_API_KEY` | 默认 OpenAI profile 的 key |
| `ASTRCODE_MAX_TOOL_CONCURRENCY` | 工具并发兜底配置 |
| `ASTRCODE_TOOL_RESULT_INLINE_LIMIT` | 工具结果内联大小上限 |
| `TAURI_ENV_TARGET_TRIPLE` | Tauri sidecar 构建目标 triple |

## Eval

CI 中只保留 eval 框架的 smoke 测试，验证任务加载、runner、trace、scorer 和 report 不坏。真实模型能力评估用本地 API eval：

```bash
# 使用当前 ~/.astrcode/config.json 和环境变量
npm run eval:api -- --task-set eval-tasks/task-set.yaml --concurrency 1

# 隔离 home 与输出目录
npm run eval:api -- \
  --home .tmp/eval-home \
  --task-set eval-tasks/task-set.yaml \
  --output eval-reports/api-eval-report.json \
  --concurrency 1
```

`run-api-eval.mjs` 会自动：

1. 启动真实 `astrcode-server`。
2. 读取 server ready payload。
3. 用 bootstrap token 换取 API session token。
4. 调用 `astrcode-eval` 创建 session、提交 prompt、等待 turn 完成。
5. 从 JSONL session log 提取 trace 并评分。

输出：

- 报告：`eval-reports/api-eval-report.json`
- server 日志：`eval-reports/api-eval-server.log`

如果你已经有一个正在运行的 server，也可以直接使用底层 CLI：

```bash
cargo run -p astrcode-eval -- \
  --server-url http://127.0.0.1:<port> \
  --session-storage-root ~/.astrcode/projects \
  --task-set eval-tasks/task-set.yaml \
  --output eval-reports/report.json
```

## Workspace 结构

```text
crates/
  core/                  共享语义、强类型 ID、durable event 模型
  protocol/              HTTP/SSE/plugin wire DTO
  support/               host path、shell、tool result 等宿主工具

  prompt-contract/       prompt 声明与渲染契约
  governance-contract/   mode DSL、策略、治理 prompt 契约
  tool-contract/         工具 trait、上下文、事件 sink
  llm-contract/          LLM provider/request/output 契约
  runtime-contract/      runtime handle 与 turn event 契约

  context-window/        上下文窗口、compact、请求整形、tool result budget
  agent-runtime/         单 turn 执行循环、LLM 调用、工具调度
  host-session/          session owner、JSONL truth、投影、fork、input queue
  plugin-host/           plugin/MCP/builtin 贡献的快照、校验、调度

  adapter-agents/        Agent profile 加载
  adapter-llm/           OpenAI 兼容 LLM provider
  adapter-mcp/           MCP client 与能力桥接
  adapter-prompt/        prompt provider
  adapter-skills/        Skill 发现、解析、物化
  adapter-storage/       JSONL session/config/MCP 设置存储
  adapter-tools/         内置工具实现

  server/                唯一组合根 + Axum HTTP/SSE API
  client/                类型化 HTTP/SSE SDK
  cli/                   ratatui TUI
  eval/                  评测 runner / trace / scorer / report

frontend/                React + TypeScript + Vite
src-tauri/               Tauri 薄壳，负责 sidecar 和窗口能力
scripts/                 开发、检查、eval 和 hook 脚本
```

## 关键运行路径

### 提交 Prompt

```text
UI / CLI
  -> client/protocol DTO
  -> server HTTP route
  -> root_execute_service
  -> session_runtime_port adapter
  -> host-session accept/begin/persist/complete
  -> agent-runtime turn loop
  -> adapter-llm + adapter-tools
  -> durable JSONL + live SSE projection
```

### Agent 协作

```text
spawn/send/observe/close tools
  -> server agent runtime bridge
  -> host-session child session / sub-run lineage
  -> agent-runtime child turn
  -> durable collaboration events
```

### 配置重载

```text
POST /api/config/reload
  -> governance service
  -> config / MCP / plugin / skill reload
  -> capability surface candidate
  -> RuntimeCoordinator 原子替换
```

## Server API 摘要

完整 DTO 在 `crates/protocol/src/http`，路由在 `crates/server/src/http/routes`。

| 端点 | 说明 |
|---|---|
| `POST /api/auth/exchange` | bootstrap token 换 API token |
| `GET/POST /api/sessions` | 列出或创建 session |
| `POST /api/sessions/{id}/prompts` | 提交用户 prompt |
| `POST /api/sessions/{id}/compact` | 手动 compact |
| `POST /api/sessions/{id}/fork` | fork session |
| `POST /api/sessions/{id}/interrupt` | 中断执行 |
| `GET/POST /api/sessions/{id}/mode` | 查询或切换 mode |
| `GET /api/session-events` | session catalog SSE |
| `GET /api/v1/conversation/sessions/{id}/snapshot` | conversation snapshot |
| `GET /api/v1/conversation/sessions/{id}/stream` | conversation delta SSE |
| `GET/POST /api/config...` | 配置读取、reload、active selection |
| `GET/POST /api/models...` | 模型列表、当前模型、连接测试 |
| `GET/POST /api/v1/agents...` | agent profiles、root execution、subrun 状态、关闭 agent |
| `GET/POST /api/mcp...` | MCP 状态、审批、重连、配置写入 |

## 开发检查

```bash
# 快速 Rust 检查
cargo check --workspace
cargo test --workspace --exclude astrcode --lib

# 架构边界
node scripts/check-crate-boundaries.mjs --strict
node scripts/generate-crate-deps-graph.mjs --check

# 完整 Rust CI 子集
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode

# 前端
cd frontend && npm run typecheck && npm run lint && npm run format:check
```

## CI

| Workflow | 内容 |
|---|---|
| `rust-check` | fmt、clippy、crate 依赖图、边界检查、targeted regression、Ubuntu/Windows Rust 测试 |
| `frontend-check` | typecheck、lint、format |
| `dependency-audit` | 依赖审查 |
| `tauri-build` | tag 发布时构建桌面端 |

## 许可证

本项目采用仓库根目录 [LICENSE](LICENSE) 中声明的许可证文本：**Apache License 2.0 with Commons Clause**。

- 允许个人使用、学习和研究
- 允许非商业开源项目使用
- 商业用途需先获得作者许可

联系方式：

- Email: 1879483647@qq.com
- GitHub Issues: https://github.com/whatevertogo/Astrcode/issues
