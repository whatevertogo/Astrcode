# AstrCode

一个 AI 编程助手，支持桌面端（Tauri）、浏览器端和终端（CLI），基于 Rust + React 构建的 HTTP/SSE 分层架构。

> 当前处于 `v0.1.0-alpha` 实验阶段。适合试用、评估架构和参与共建，不承诺接口稳定性。

- 发布下载：[GitHub Releases](https://github.com/whatevertogo/Astrcode/releases)
- 安装说明：见下文“下载与安装”
- 路线图：[ROADMAP.md](ROADMAP.md)
- 贡献指南：[CONTRIBUTING.md](CONTRIBUTING.md)
- 安全策略：[SECURITY.md](SECURITY.md)

## 功能特性

- **多模型支持**：统一走 OpenAI 家族接口，支持 OpenAI Responses、OpenAI Chat Completions 与兼容网关（DeepSeek 等），运行时切换 Profile 和 Model
- **流式响应**：实时显示 AI 生成的代码和文本，支持 thinking 内容展示
- **内置工具集**：文件读写、编辑、搜索、Shell 执行、Skill 加载等
- **Agent 协作**：支持主/子 Agent 模式，内置 spawn / send / observe / close 工具链
- **Skill 系统**：Claude 风格两阶段 Skill 加载，支持项目级、用户级和内置 Skill
- **MCP 支持**：完整的 Model Context Protocol 接入，支持 stdio / HTTP / SSE 传输
- **插件系统**：基于 stdio JSON-RPC 的插件扩展，提供 Rust SDK(未完善)
- **会话管理**：多会话切换、按项目分组、事件溯源持久化、会话历史浏览
- **三种运行模式**：
  - **桌面端**：Tauri 打包，自动管理本地 Server
  - **浏览器端**：独立运行 Server，浏览器访问
  - **终端**：ratatui TUI 界面，本地或远程连接 Server

## 技术栈

| 层级 | 技术 |
|------|------|
| 后端 | Rust (nightly), Axum, Tokio, Tower |
| 前端 | React 18, TypeScript, Vite, Tailwind CSS |
| 桌面端 | Tauri 2 |
| 终端 | ratatui, crossterm |
| 通信 | HTTP/SSE, JSON-RPC (stdio) |
| 持久化 | JSONL 事件日志, 文件系统存储 |
| CI | GitHub Actions, cargo-deny |

## 内置工具

| 工具 | 描述 |
|------|------|
| `Skill` | 按需加载 Claude 风格 `SKILL.md` 指南与 `references/` / `scripts/` 等资源 |
| `readFile` | 读取文件内容 |
| `writeFile` | 写入或创建文件，并返回结构化 diff metadata |
| `editFile` | 精确替换文件内容（唯一匹配验证），并返回结构化 diff metadata |
| `apply_patch` | 应用 unified diff 格式的多文件批量变更 |
| `listDir` | 列出目录内容 |
| `findFiles` | Glob 模式文件搜索 |
| `grep` | 正则表达式内容搜索 |
| `shell` | 执行 Shell 命令，stdout/stderr 以流式事件增量展示 |
| `tool_search` | 搜索可用工具 |
| `spawn` | 创建子 Agent |
| `send` | 向 Agent 发送消息 |
| `observe` | 观察 Agent 状态 |
| `close` | 关闭 Agent |

## 下载与安装

### 预编译版本

`v0.1.0-alpha` 起，预编译二进制会发布在 [GitHub Releases](https://github.com/whatevertogo/Astrcode/releases)：

- **桌面端**：下载对应平台的 Tauri 安装包
- **源码包**：下载 tag 对应源码，按下文方式本地构建

当前 alpha 版本定位：

- 验证桌面端、浏览器端、CLI 三端形态
- 验证 Rust + React + HTTP/SSE 分层架构
- 验证工具调用、Agent 协作、MCP/插件等核心能力

### 从源码安装

```bash
# 安装仓库级依赖
npm install
cd frontend && npm install

# 运行桌面端
npm run dev:tauri

# 或单独运行服务端 / CLI
cargo run -p astrcode-server
cargo run -p astrcode-cli
```

如果你想把 CLI 安装到本机：

```bash
cargo install --path crates/cli
```

## 快速开始

### 环境要求

- Rust **nightly** 工具链（见 `rust-toolchain.toml`）
- [Node.js](https://nodejs.org/) 20+
- npm（Node.js 自带）

### 安装依赖

```bash
# 安装仓库级工具（会自动注册 .githooks/pre-commit / pre-push）
npm install

# 安装前端依赖
cd frontend && npm install
```

执行 `npm install` 时，会自动把仓库的 `core.hooksPath` 指向 `.githooks/`。三层校验：

- `pre-commit`：快速检查 — 自动格式化 Rust / 前端改动，修复已暂存 TS/TSX 的 ESLint 问题，阻止大文件、冲突标记和密钥泄漏
- `pre-push`：中等检查 — `cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib` 和前端 `typecheck`
- GitHub Actions：完整校验 — 格式检查、clippy、全量 Rust 测试、前端 lint/format、依赖审查与发布构建

### 开发模式

```bash
# 桌面端开发（推荐）
npm run dev:tauri

# 只启动前端
cd frontend && npm run dev

# 只启动后端
cargo run -p astrcode-server

# 浏览器端开发：分别启动 server 和前端，然后打开 http://127.0.0.1:5173/
```

### 构建

```bash
# 桌面端构建
npm run build

# 浏览器端构建
cd frontend && npm run build
# 然后启动 cargo run -p astrcode-server，并打开它输出的 http://localhost:<port>/
# server 会直接托管 frontend/dist，并自动注入浏览器端 bootstrap
```

## 项目预览

当前仓库已经先补齐 release、安装入口和维护文档；桌面端/终端的正式截图与 GIF 会在下一轮产品化迭代补上。

![AstrCode Icon](src-tauri/icons/icon.png)

## 配置

首次运行会在 `~/.astrcode/config.json` 创建配置文件：

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

### API Key 配置

`apiKey` 字段支持三种方式：

1. **显式环境变量引用**：`env:DEEPSEEK_API_KEY`
2. **明文字面量**：直接填写 API Key（如 `sk-xxxx`）
3. **字面量前缀**：`literal:MY_VALUE`，用于强制把看起来像环境变量名的字符串按普通文本处理

推荐优先使用 `env:...`，配置含义最明确。

### 模型配置

`models` 为对象列表，每个模型需要配置 `maxTokens` 和 `contextLimit`：

- **OpenAI profile**：统一使用 `providerKind: "openai"`
- **`apiMode: "chat_completions"`**：适合 DeepSeek 等 OpenAI 兼容网关
- **`apiMode: "responses"`**：适合 OpenAI 官方原生 Responses API

### 多 Profile 配置

可配置多个 API 提供商，在设置界面切换：

```json
{
  "profiles": [
    {
      "name": "deepseek",
      "providerKind": "openai",
      "baseUrl": "https://api.deepseek.com",
      "apiKey": "env:DEEPSEEK_API_KEY",
      "apiMode": "chat_completions",
      "models": [{ "id": "deepseek-chat", "maxTokens": 8096, "contextLimit": 128000 }]
    },
    {
      "name": "openai",
      "providerKind": "openai",
      "baseUrl": "https://api.openai.com/v1",
      "apiKey": "env:OPENAI_API_KEY",
      "apiMode": "responses",
      "models": [
        { "id": "gpt-4.1", "maxTokens": 32768, "contextLimit": 128000 },
        { "id": "gpt-4.1-mini", "maxTokens": 32768, "contextLimit": 128000 }
      ]
    }
  ]
}
```

### Runtime 配置

`runtime` 用于放置 AstrCode 进程级运行参数：

```json
{
  "runtime": {
    "maxToolConcurrency": 10,
    "compactKeepRecentTurns": 4,
    "compactKeepRecentUserMessages": 8,
    "compactMaxOutputTokens": 20000
  }
}
```

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `maxToolConcurrency` | 10 | 并发工具上限 |
| `compactKeepRecentTurns` | 4 | 压缩时保留最近的 turn 数 |
| `compactKeepRecentUserMessages` | 8 | 压缩时额外保留最近真实用户消息的数量（原文重新注入） |
| `compactMaxOutputTokens` | 20000 | 压缩请求的最大输出 token 上限（自动取模型限制的较小值） |

### 内建环境变量

项目自定义环境变量按类别集中维护在 `crates/application/src/config/constants.rs`：

| 类别 | 环境变量 | 作用 |
|------|----------|------|
| Home / 测试隔离 | `ASTRCODE_HOME_DIR` | 覆盖 AstrCode 的 home 目录 |
| Home / 测试隔离 | `ASTRCODE_TEST_HOME` | 为测试隔离临时 home 目录 |
| Plugin | `ASTRCODE_PLUGIN_DIRS` | 追加插件发现目录，按系统路径分隔符解析 |
| Provider 默认值 | `DEEPSEEK_API_KEY` | DeepSeek 默认 profile 的 API Key |
| Provider 默认值 | `OPENAI_API_KEY` | OpenAI 默认 profile 的 API Key |
| Runtime | `ASTRCODE_MAX_TOOL_CONCURRENCY` | 并发工具上限兜底 |
| Build / Tauri | `TAURI_ENV_TARGET_TRIPLE` | 构建 sidecar 时指定目标 triple |

## 项目结构

```
AstrCode/
├── crates/
│   ├── core/                 # 领域模型、强类型 ID、端口契约、CapabilitySpec、稳定配置
│   ├── protocol/             # HTTP/SSE/Plugin DTO 与 wire 类型（含 CapabilityWireDescriptor）
│   ├── kernel/               # 全局控制面：surface / registry / agent tree / events
│   ├── session-runtime/      # 单会话真相：state / turn / replay / context window
│   ├── application/          # 用例编排、执行控制、治理与观测
│   ├── server/               # Axum HTTP/SSE 边界与唯一组合根
│   ├── adapter-storage/      # JSONL 事件日志持久化与文件系统存储
│   ├── adapter-llm/          # LLM provider（OpenAI Responses / Chat Completions）
│   ├── adapter-prompt/       # Prompt 组装（贡献者模式 + 分层缓存构建）
│   ├── adapter-tools/        # 内置工具定义与 Agent 协作工具
│   ├── adapter-skills/       # Skill 发现、解析、物化与目录管理
│   ├── adapter-mcp/          # MCP 协议支持（stdio/HTTP/SSE 传输 + 能力桥接）
│   ├── adapter-agents/       # Agent profile 加载与注册表（builtin/user/project 级）
│   ├── client/               # 类型化 HTTP/SSE 客户端 SDK
│   ├── cli/                  # 终端 TUI 客户端（ratatui）
│   ├── plugin/               # stdio JSON-RPC 插件宿主基础设施
│   └── sdk/                  # 插件开发者 Rust SDK
├── examples/                 # 示例插件与示例 manifest
├── src-tauri/                # Tauri 薄壳：sidecar 管理、窗口控制、bootstrap 注入
├── frontend/                 # React + TypeScript + Vite + Tailwind CSS
│   └── src/
│       ├── components/       # React 组件（Chat / Sidebar / Settings）
│       ├── hooks/            # 自定义 hooks（useAgent 等）
│       └── lib/              # API 客户端、SSE 事件处理、工具函数
└── scripts/                  # 开发脚本（Git hooks、crate 边界检查等）
```

## 架构

### 分层架构概览

```
┌─────────────────────────────────────────────────────────────┐
│                     前端（三种接入方式）                       │
│  ┌──────────┐   ┌──────────┐   ┌──────────────────────────┐ │
│  │ Tauri UI │   │ Browser  │   │ CLI (ratatui TUI)        │ │
│  │ HostBrdg │   │ fetch/SSE│   │ client crate + launcher  │ │
│  └────┬─────┘   └────┬─────┘   └────────────┬─────────────┘ │
└───────┼──────────────┼──────────────────────┼────────────────┘
        │              │ HTTP/SSE             │ HTTP/SSE
        ▼              ▼                      ▼
┌─────────────────────────────────────────────────────────────┐
│                    astrcode-server                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │ Axum Router  │─▶│ application  │─▶│    kernel        │  │
│  │ /api/*       │  │ App / Gov.   │  │ surface / events │  │
│  └──────────────┘  └──────┬───────┘  └────────┬─────────┘  │
│  ┌──────────────┐         │                    │            │
│  │ Protocol DTO │◀────────┤           ┌────────▼────────┐  │
│  └──────────────┘         │           │ session-runtime │  │
│  ┌──────────────┐         │           │ turn / replay   │  │
│  │ Auth/Bootstrp│         │           │ context window  │  │
│  └──────────────┘         │           └─────────────────┘  │
│                           ▼                                 │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ adapter-* : storage | llm | prompt | tools | skills  │   │
│  │            | mcp | agents                             │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### 核心分层职责

- **`core`**：领域语义、强类型 ID、端口契约、`CapabilitySpec`、稳定配置模型。不依赖传输层或具体实现；`CapabilitySpec` 是运行时内部的能力语义真相。
- **`protocol`**：HTTP/SSE/Plugin 的 DTO 与 wire 类型，仅依赖 `core`；其中 `CapabilityWireDescriptor` 只承担协议边界传输职责，不是运行时内部的能力真相。
- **`kernel`**：全局控制面 — capability router/registry、agent tree、统一事件协调。
- **`session-runtime`**：单会话真相 — turn 执行、事件回放、compact（保留最近用户消息 + 摘要 + 输出上限控制）、context window、input queue 推进。
- **`application`**：用例编排入口（`App`）+ 治理入口（`AppGovernance`），负责参数校验、权限、策略、reload 编排。通过 `AppAgentPromptSubmission` 端口向 session-runtime 提交 turn。
- **`server`**：HTTP/SSE 边界与唯一组合根（`bootstrap/runtime.rs`），只负责 DTO 映射和装配。
- **`adapter-*`**：端口实现层，不持有业务真相，不偷渡业务策略。核心类型（`LlmProvider`、`LlmRequest`、`EventStore` 等）统一在 `core` 定义，adapter 仅提供具体实现。

### Agent 协作

- 内置 Agent profile：explore、reviewer、execute
- Agent 文件来源：builtin + 用户级（`~/.astrcode/agents`）+ 项目级（`.astrcode/agents`，祖先链扫描）
- 子 Agent spawn 时按 task-scoped capability grant 裁剪能力面
- Agent 工具链：`spawn` -> `send` -> `observe` -> `close` 全生命周期管理

### Skill 系统

- 两阶段加载：system prompt 先展示 skill 索引，命中后再调用 `Skill` tool 加载完整 `SKILL.md`
- 目录格式：`skill-name/SKILL.md`（Markdown + YAML frontmatter）
- 加载来源：builtin（运行时物化到 `~/.astrcode/runtime/builtin-skills/`）+ 项目级 + 用户级
- 资产目录（`references/`、`scripts/`）随 skill 一起索引

### MCP 支持

- 完整 MCP 协议实现：JSON-RPC 消息、工具/prompt/资源/skill 桥接
- 传输方式：stdio、HTTP、SSE
- 连接管理：状态机、自动重连、热加载
- 配置集成：通过 config.json 声明 MCP server，reload 时统一刷新

### 插件系统

- 基于 stdio 双向通信
- 插件生命周期管理（discovered -> loaded -> failed -> disabled）
- 能力路由与权限检查
- 流式执行支持
- 提供 Rust SDK（`crates/sdk`），包含 `ToolHandler`、`HookRegistry`、`PluginContext`、`StreamWriter`
- 插件握手交换的是 `CapabilityWireDescriptor`；宿主内部消费和决策始终基于 `CapabilitySpec`

### 会话持久化与上下文压缩

- JSONL 格式追加写入（append-only event log）
- 存储路径：`~/.astrcode/projects/<project>/sessions/<session-id>/`
- 文件锁并发保护（`active-turn.lock`）
- Query / Command 逻辑分离

**上下文压缩（Compact）**：

- 触发方式：自动（token 阈值触发）和手动（`/compact` 命令或 API）
- 压缩策略：保留最近 N 个 turn 的完整上下文，对更早的历史生成结构化摘要
- 最近用户消息保留：压缩后原样重新注入最近 N 条真实用户消息，确保模型不会丢失当前意图
- 用户上下文摘要：为保留的用户消息生成极短目的摘要（`recent_user_context_digest`），帮助模型快速定位目标
- 输出控制：压缩请求有独立的 `max_output_tokens` 上限，防止压缩本身消耗过多 token

### 治理与重载

- `POST /api/config/reload` 走统一治理入口，串起：配置重载 -> MCP 刷新 -> plugin 重新发现 -> skill 更新 -> kernel capability surface 原子替换
- 运行中存在 session 时拒绝 reload，避免半刷新导致执行语义漂移
- capability surface 替换失败时保留旧状态继续服务

### Tauri 桌面端

Tauri 仅作为"薄壳"，负责：

1. **Sidecar 管理**：启动和管理 `astrcode-server` 进程
2. **Bootstrap 注入**：通过 `window.__ASTRCODE_BOOTSTRAP__` 注入 token 和 server 地址
3. **窗口控制**：最小化、最大化、关闭

### Server API

| 端点 | 方法 | 描述 |
|------|------|------|
| `/api/auth/exchange` | POST | Token 认证交换 |
| `/api/sessions` | GET/POST | 会话列表/创建 |
| `/api/sessions/{id}/messages` | GET | 获取会话消息 |
| `/api/sessions/{id}/prompts` | POST | 提交 prompt（支持 `manualCompact` 执行控制） |
| `/api/sessions/{id}/interrupt` | POST | 中断会话 |
| `/api/sessions/{id}/events` | GET (SSE) | 实时事件流 |
| `/api/sessions/{id}` | DELETE | 删除会话 |
| `/api/projects` | DELETE | 删除项目（所有会话） |
| `/api/config` | GET | 获取配置 |
| `/api/config/reload` | POST | 统一治理重载 |
| `/api/config/active-selection` | POST | 保存当前选择 |
| `/api/models/current` | GET | 当前模型信息 |
| `/api/models` | GET | 可用模型列表 |
| `/api/models/test` | POST | 测试模型连接 |
| `/api/runtime/plugins` | GET | 插件运行状态 |
| `/api/runtime/plugins/reload` | POST | 重新加载插件 |

### SSE 事件

通过 Server-Sent Events 推送实时更新：

| 事件 | 描述 |
|------|------|
| `phaseChanged` | 阶段变化（idle/thinking/streaming/callingTool） |
| `modelDelta` | 流式文本片段 |
| `thinkingDelta` | 推理内容片段 |
| `assistantMessage` | 最终助手消息 |
| `toolCallStart` | 工具调用开始 |
| `toolCallResult` | 工具调用结果 |
| `promptMetrics` | 回合级 token / 缓存命中率指标 |
| `compactApplied` | 上下文压缩完成，携带压缩摘要信息 |
| `turnDone` | 对话回合结束 |
| `error` | 错误信息 |

## 开发指南

### 代码检查

```bash
# 本地 push 前快速检查
cargo check --workspace
cargo test --workspace --exclude astrcode --lib
cd frontend && npm run typecheck

# 与 CI 对齐的完整检查
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
node scripts/check-crate-boundaries.mjs
cd frontend && npm run typecheck && npm run lint && npm run format:check
```

### 代码格式化

```bash
# Rust
cargo fmt --all

# 前端
cd frontend && npm run format
```

### 测试

```bash
# push 前 Rust 测试子集
cargo test --workspace --exclude astrcode --lib

# 与 CI 对齐的全量 Rust 测试
cargo test --workspace --exclude astrcode

# 运行前端测试
cd frontend && npm run test:watch
```

### 依赖审查

```bash
cargo deny check bans
```

## CI/CD

项目使用 4 个 GitHub Actions workflow：

| Workflow | 触发条件 | 执行内容 |
|----------|----------|----------|
| `rust-check` | push/PR 到 master（Rust 文件变更） | fmt、clippy、crate 边界检查、回归测试、全量测试（Ubuntu + Windows） |
| `frontend-check` | push/PR 到 master（前端文件变更） | typecheck、lint、format 检查 |
| `dependency-audit` | `Cargo.lock` / `deny.toml` 变更 | `cargo deny check bans` |
| `tauri-build` | 发布 tag (`v*`) | 三平台（Ubuntu/Windows/macOS）Tauri 构建 |

## 路线图

当前和后续计划见 [ROADMAP.md](ROADMAP.md)。如果你想看近期优先级，重点关注：

- `v0.1.0-alpha`：发布首个可下载预发布版本，补齐试用入口
- `v0.1.0-beta`：补齐稳定性、安装体验、截图/GIF、更多文档
- `v0.1.x`：收敛协议与配置，降低试用门槛

## 贡献与反馈

- 提交代码前请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)
- 安全问题请按 [SECURITY.md](SECURITY.md) 中的方式私下报告
- 普通 bug / 功能建议请使用 GitHub Issue 模板
- 与发布相关的已知计划和限制见 [docs/releases/v0.1.0-alpha.md](docs/releases/v0.1.0-alpha.md)

## 许可证

本项目采用仓库根目录 [LICENSE](LICENSE) 中声明的许可证文本：**Apache License 2.0 with Commons Clause**。

为避免 `Cargo.toml`、README 与许可证文本出现漂移，Rust crate 清单统一通过 `license-file` 指向根目录 `LICENSE`，以该文件为唯一许可证事实来源。

- 允许个人使用、学习和研究
- 允许非商业开源项目使用
- **商业用途**需先获得作者许可

联系方式：

- Email: 1879483647@qq.com
- GitHub Issues: https://github.com/whatevertogo/Astrcode/issues

详见 [LICENSE](LICENSE) 文件了解详情。

## 致谢

- [Tauri](https://tauri.app/) - 跨平台桌面应用框架
- [Axum](https://github.com/tokio-rs/axum) - Web 框架
- [Tokio](https://tokio.rs/) - 异步运行时
- [React](https://react.dev/) - 前端框架
- [Vite](https://vitejs.dev/) - 构建工具
- [Tailwind CSS](https://tailwindcss.com/) - CSS 框架
- [ratatui](https://ratatui.rs/) - 终端 UI 框架
