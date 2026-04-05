# AstrCode

一个 AI 编程助手应用，支持桌面端（Tauri）和浏览器端，基于 HTTP/SSE Server 架构实现前后端解耦。

## 功能特性

- **多模型支持**：支持 OpenAI 兼容 API（DeepSeek、OpenAI 等），运行时可切换 Profile 和 Model
- **流式响应**：实时显示 AI 生成的代码和文本
- **多工具调用**：内置文件操作、代码搜索、Shell 执行等工具
- **会话管理**：支持多会话切换、按项目分组、会话历史浏览
- **插件系统**：支持 stdio 插件扩展能力
- **双模式运行**：
  - **桌面端**：Tauri 打包，自动管理本地 Server
  - **浏览器端**：独立运行 Server，浏览器访问

## 内置工具

| 工具 | 描述 |
|------|------|
| `Skill` | 按需加载 Claude 风格 `SKILL.md` 指南与 `references/` / `scripts/` 等资源 |
| `read_file` | 读取文件内容 |
| `write_file` | 写入或创建文件，并返回结构化 diff metadata |
| `edit_file` | 精确替换文件内容（唯一匹配验证），并返回结构化 diff metadata |
| `list_dir` | 列出目录内容 |
| `find_files` | Glob 模式文件搜索 |
| `grep` | 正则表达式内容搜索 |
| `shell` | 执行 Shell 命令，stdout/stderr 以流式事件增量展示 |

## 快速开始

### 环境要求

- [Rust](https://www.rust-lang.org/tools/install) 1.70+
- [Node.js](https://nodejs.org/) 20+
- npm（Node.js 自带）

### 安装依赖

```bash
# 安装仓库级工具（会自动注册 .githooks/pre-commit / pre-push）
npm install

# 安装前端依赖
cd frontend && npm install
```

执行根目录或 `frontend` 的 `npm install` 时，会自动把仓库的 `core.hooksPath` 指向 `.githooks/`。仓库现在按三层校验运行：

- `pre-commit`：只做快检查，自动格式化 Rust / 前端改动，修复已暂存 TS/TSX 的 ESLint 问题，并阻止大文件、冲突标记和明显密钥泄漏进入提交。
- `pre-push`：做中等检查，运行 `cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib` 和前端 `typecheck`。
- GitHub Actions：保留完整校验，执行格式检查、clippy、全量 Rust 测试、前端 lint/format 校验，以及依赖审查与发布构建流程。

### 开发模式

#### 使用 Makefile（推荐）

```bash
# 桌面端开发
make dev          # Windows
make dev-unix     # Linux/macOS

# 只启动前端
make frontend

# 只启动 Tauri
make tauri

# 代码检查
make check
```

#### 直接命令

```bash
# 桌面端开发（推荐）
cargo tauri dev

# 浏览器端开发
# 终端 1：启动 Server
cargo run -p astrcode-server

# 终端 2：启动前端
cd frontend && npm run dev
# 然后打开 http://127.0.0.1:5173/
```

### 构建

```bash
# 使用 Makefile
make build

# 或直接命令
cargo tauri build

# Windows: 最终可分发产物在 target/release/bundle/ 下；
# target/release/astrcode.exe 更适合本地调试，安装包 / bundle 才是完整桌面应用形态。

# 浏览器端构建
cd frontend && npm run build
# 然后启动 cargo run -p astrcode-server，并打开它输出的 http://localhost:<port>/
# server 会直接托管 frontend/dist，并自动注入浏览器端 bootstrap
```

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
      "providerKind": "openai-compatible",
      "baseUrl": "https://api.deepseek.com",
      "apiKey": "env:DEEPSEEK_API_KEY",
      "models": [
        {
          "id": "deepseek-chat",
          "maxTokens": 8096,
          "contextLimit": 128000
        },
        {
          "id": "deepseek-reasoner",
          "maxTokens": 8096,
          "contextLimit": 128000
        }
      ]
    }
  ]
}
```

`runtime` 用于放置 AstrCode 自己的进程级运行参数，例如：

```json
{
  "runtime": {
    "maxToolConcurrency": 10
  }
}
```

### API Key 配置

`apiKey` 字段支持三种方式：

1. **显式环境变量引用**：`env:DEEPSEEK_API_KEY`
2. **明文字面量**：直接填写 API Key（如 `sk-xxxx`）
3. **字面量前缀**：`literal:MY_VALUE`，用于强制把看起来像环境变量名的字符串按普通文本处理

推荐优先使用 `env:...`，这样配置文件的含义最明确，不会让用户误以为 AstrCode 会自动把任意裸字符串当成环境变量读取。

### 模型 limits 配置

`models` 现在是对象列表，而不再是纯字符串数组：

- OpenAI-compatible profile 必须为每个模型手动设置 `maxTokens` 和 `contextLimit`
- Anthropic profile 会在运行时通过 `GET /v1/models/{model_id}` 自动获取 `max_input_tokens` 和 `max_tokens`
- 如果 Anthropic 远端探测失败，但本地模型对象里同时写了 `maxTokens` 和 `contextLimit`，运行时会回退到本地值

这让上下文窗口和最大输出 token 的来源保持单一且清晰，不再由 provider 内部各自硬编码。

### 内建环境变量

项目自定义环境变量按类别集中维护在 `crates/runtime-config/src/constants.rs`，底层常量源头在 `crates/core/src/env.rs`，避免低层 crate 反向依赖配置 crate。

| 类别 | 环境变量 | 作用 |
|------|------|------|
| Home / 测试隔离 | `ASTRCODE_HOME_DIR` | 覆盖 Astrcode 的 home 目录 |
| Home / 测试隔离 | `ASTRCODE_TEST_HOME` | 为测试隔离临时 home 目录 |
| Plugin | `ASTRCODE_PLUGIN_DIRS` | 追加插件发现目录，按系统路径分隔符解析 |
| Provider 默认值 | `DEEPSEEK_API_KEY` | DeepSeek 默认 profile 的 API Key |
| Provider 默认值 | `ANTHROPIC_API_KEY` | Anthropic 默认 profile 的 API Key |
| Runtime | `ASTRCODE_MAX_TOOL_CONCURRENCY` | `runtime.maxToolConcurrency` 未设置时的并发工具上限兜底 |
| Build / Tauri | `TAURI_ENV_TARGET_TRIPLE` | 构建 sidecar 时指定目标 triple |

像 `OPENAI_API_KEY` 这类自定义 profile 使用的环境变量仍然允许自由命名，但不属于平台内建环境变量目录。

### 多 Profile 配置

可配置多个 API 提供商，在设置界面切换：

```json
{
  "profiles": [
    {
      "name": "deepseek",
      "baseUrl": "https://api.deepseek.com",
      "apiKey": "env:DEEPSEEK_API_KEY",
      "models": [
        {
          "id": "deepseek-chat",
          "maxTokens": 8096,
          "contextLimit": 128000
        }
      ]
    },
    {
      "name": "openai",
      "baseUrl": "https://api.openai.com",
      "apiKey": "env:OPENAI_API_KEY",
      "models": [
        {
          "id": "gpt-4o",
          "maxTokens": 16384,
          "contextLimit": 200000
        },
        {
          "id": "gpt-4o-mini",
          "maxTokens": 16384,
          "contextLimit": 128000
        }
      ]
    }
  ]
}
```

## 项目结构

```
AstrCode/
├── crates/
│   ├── core/           # 纯领域类型、事件存储、投影、注册表
│   │   ├── action.rs   # Agent 动作类型
│   │   ├── cancel.rs   # 取消令牌
│   │   ├── event/      # 事件存储与查询
│   │   ├── plugin/     # 插件清单与注册表
│   │   ├── registry/   # 工具/能力路由
│   │   ├── session/    # 会话类型与持久化
│   │   └── tool.rs     # Tool trait 定义
│   ├── runtime/        # AgentLoop、配置与运行态 façade
│   │   ├── agent_loop/ # LLM/Tool 循环实现
│   │   └── service/    # RuntimeService 门面
│   ├── protocol/       # HTTP / SSE / Plugin DTO
│   │   ├── http/       # API 请求/响应类型
│   │   └── plugin/     # 插件协议定义
│   ├── plugin/         # stdio 插件运行时
│   ├── sdk/            # 插件作者 API
│   ├── tools/          # Tool 实现（不依赖 runtime）
│   └── server/         # Axum 本地 server（唯一业务入口）
├── examples/           # 示例插件与示例 manifest
├── src-tauri/          # Tauri 薄壳：sidecar 管理、窗口控制
├── frontend/           # React + TypeScript + Vite UI
│   ├── src/
│   │   ├── components/ # React 组件
│   │   ├── hooks/      # 自定义 hooks
│   │   └── lib/        # 工具函数
└── scripts/            # 开发脚本
```

## 架构

### HTTP/SSE Server 架构

系统采用前后端分离架构，Server 是唯一的业务入口：

```
┌─────────────────────────────────────────────────────────┐
│                      Frontend                           │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐ │
│  │   React UI  │───▶│  useAgent   │───▶│ HostBridge  │ │
│  └─────────────┘    │ fetch/SSE   │    │ (桌面/浏览器)│ │
│                     └──────┬──────┘    └─────────────┘ │
└────────────────────────────┼────────────────────────────┘
                             │ HTTP/SSE
                             ▼
┌────────────────────────────────────────────────────────┐
│                   astrcode-server                       │
│  ┌─────────────┐    ┌──────────────┐   ┌─────────────┐ │
│  │  Axum Router│───▶│RuntimeService│───▶│Capability   │ │
│  │             │    │              │   │  Router     │ │
│  │  /api/*     │    │  EventStore  │   │ ToolRegistry│ │
│  └─────────────┘    └──────────────┘   └─────────────┘ │
│  ┌─────────────┐    ┌──────────────┐   ┌─────────────┐ │
│  │Auth/Bootstrap│   │ Plugin       │   │ Protocol    │ │
│  │   Token     │    │ Supervisor   │   │   DTO       │ │
│  └─────────────┘    └──────────────┘   └─────────────┘ │
└────────────────────────────────────────────────────────┘
```

### Skill 架构

- skill 采用 Claude 风格的两阶段模型：system prompt 先给模型看 skill 索引，命中后再调用内置 `Skill` tool 加载完整 `SKILL.md`。
- skill 目录格式固定为 `skill-name/SKILL.md`，frontmatter 只认 `name` 和 `description`。
- `references/`、`scripts/` 等目录会作为 skill 资产一起索引；builtin skill 整目录会在运行时物化到 `~/.astrcode/runtime/builtin-skills/`，方便 shell 直接执行其中脚本。

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
| `/api/sessions/{id}/prompts` | POST | 提交 prompt |
| `/api/sessions/{id}/interrupt` | POST | 中断会话 |
| `/api/sessions/{id}/events` | GET (SSE) | 实时事件流 |
| `/api/sessions/{id}` | DELETE | 删除会话 |
| `/api/projects` | DELETE | 删除项目（所有会话） |
| `/api/config` | GET | 获取配置 |
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
| `turnDone` | 对话回合结束 |
| `error` | 错误信息 |

## 开发指南

### 代码检查

```bash
# 本地 push 前检查
make check

# 与 CI 对齐的完整检查
make check-ci

# 或直接运行 npm 脚本
npm run check:push
npm run check:ci

# 前端检查
cd frontend
npm run typecheck
npm run lint
npm run format:check
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

项目使用 4 个 GitHub Actions workflow，分工如下：

- `rust-check`：完整 Rust 质量门禁，执行 `cargo fmt --all -- --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`
- `frontend-check`：完整前端门禁，执行 `cd frontend && npm run typecheck && npm run lint && npm run format:check`
- `dependency-audit`：当 `Cargo.lock` 或 `deny.toml` 变更时执行 `cargo deny check bans`
- `tauri-build`：在发布 tag 时构建 Tauri 桌面端

## 许可证

本项目采用 **Apache License 2.0 with Commons Clause** 许可证。

- ✅ 个人使用、学习和研究：**允许**
- ✅ 非商业开源项目使用：**允许**
- ⚠️ **商业用途**：需先获得作者许可，请联系作者

详见 [LICENSE](LICENSE) 文件了解详情。

## 致谢

- [Tauri](https://tauri.app/) - 跨平台桌面应用框架
- [React](https://react.dev/) - 前端框架
- [Vite](https://vitejs.dev/) - 构建工具
- [Axum](https://github.com/tokio-rs/axum) - Web 框架
- [Tokio](https://tokio.rs/) - 异步运行时
