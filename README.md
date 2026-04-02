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
# 安装仓库级工具（会自动注册 .githooks/pre-commit）
npm install

# 安装前端依赖
cd frontend && npm install
```

执行根目录或 `frontend` 的 `npm install` 时，会自动把仓库的 `core.hooksPath` 指向 `.githooks/`。之后每次 `git commit` 都会在提交前格式化已暂存的 Rust 文件和 `frontend/src` 下的 `ts` / `tsx` / `css` 文件，并把格式化结果重新加入暂存区。

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
      "models": ["deepseek-chat", "deepseek-reasoner"]
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
      "models": ["deepseek-chat"]
    },
    {
      "name": "openai",
      "baseUrl": "https://api.openai.com",
      "apiKey": "env:OPENAI_API_KEY",
      "models": ["gpt-4o", "gpt-4o-mini"]
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
| `/api/sessions/:id/messages` | GET | 获取会话消息 |
| `/api/sessions/:id/prompts` | POST | 提交 prompt |
| `/api/sessions/:id/interrupt` | POST | 中断会话 |
| `/api/sessions/:id/events` | GET (SSE) | 实时事件流 |
| `/api/sessions/:id` | DELETE | 删除会话 |
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
# Rust 代码检查
make check
# 或
cargo check --workspace
cargo test --workspace --exclude astrcode
cargo fmt --all -- --check
cargo clippy --all-targets --all-features

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
# 运行业务 Rust 测试
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

- `rust-check`：`cargo fmt --all -- --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`
- `frontend-check`：`cd frontend && npm run typecheck && npm run lint && npm run format:check`
- `dependency-audit`：当 `Cargo.lock` 或 `deny.toml` 变更时执行 `cargo deny check bans`
- `tauri-build`：在发布 tag 时构建 Tauri 桌面端

## 许可证

MIT License

## 致谢

- [Tauri](https://tauri.app/) - 跨平台桌面应用框架
- [React](https://react.dev/) - 前端框架
- [Vite](https://vitejs.dev/) - 构建工具
- [Axum](https://github.com/tokio-rs/axum) - Web 框架
- [Tokio](https://tokio.rs/) - 异步运行时
