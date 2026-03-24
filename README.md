# AstrCode

一个 AI 编程助手应用，支持桌面端（Tauri）和浏览器端，基于 HTTP/SSE Server 架构实现前后端解耦。

## 功能特性

- **多模型支持**：支持 OpenAI 兼容 API（DeepSeek、OpenAI 等），运行时可切换 Profile 和 Model
- **流式响应**：实时显示 AI 生成的代码和文本
- **多工具调用**：内置文件操作、代码搜索、Shell 执行等工具
- **会话管理**：支持多会话切换、按项目分组、会话历史浏览
- **双模式运行**：
  - **桌面端**：Tauri 打包，自动管理本地 Server
  - **浏览器端**：独立运行 Server，浏览器访问

## 内置工具

| 工具 | 描述 |
|------|------|
| `read_file` | 读取文件内容 |
| `write_file` | 写入或创建文件 |
| `edit_file` | 精确替换文件内容 |
| `list_dir` | 列出目录内容 |
| `find_files` | Glob 模式文件搜索 |
| `grep` | 正则表达式内容搜索 |
| `shell` | 执行 Shell 命令 |

## 快速开始

### 环境要求

- [Rust](https://www.rust-lang.org/tools/install) 1.70+
- [Node.js](https://nodejs.org/) 18+
- [pnpm](https://pnpm.io/) 或 npm

### 安装依赖

```bash
# 安装前端依赖
cd frontend && npm install

# 安装 Tauri CLI（可选，用于桌面端构建）
cargo install tauri-cli
```

### 开发模式

```bash
# 桌面端开发（推荐）
cargo tauri dev

# 浏览器端开发
# 终端 1：启动 Server
cargo run -p astrcode-server

# 终端 2：启动前端
cd frontend && npm run dev
# 然后打开 http://127.0.0.1:5173/
# 前端会自动通过 Vite 的本地桥接读取 run.json，不再需要手工拼 ?token=
```

### 构建

```bash
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
  "activeProfile": "default",
  "activeModel": "deepseek-chat",
  "profiles": [
    {
      "name": "default",
      "providerKind": "openai-compatible",
      "baseUrl": "https://api.deepseek.com",
      "apiKey": "DEEPSEEK_API_KEY",
      "models": ["deepseek-chat", "deepseek-reasoner"]
    }
  ]
}
```

`AstrCode` 默认始终使用真实用户配置目录。
测试会使用隔离目录；如果你需要临时把运行中的应用或调试脚本指向其他目录，可以显式设置 `ASTRCODE_HOME_DIR`。

### API Key 配置

`apiKey` 字段支持两种方式：

1. **明文值**：直接填写 API Key（如 `sk-xxxx`）
2. **环境变量名**：填写环境变量名称（如 `DEEPSEEK_API_KEY`），程序会自动读取

### 多 Profile 配置

可配置多个 API 提供商，在设置界面切换：

```json
{
  "profiles": [
    {
      "name": "deepseek",
      "baseUrl": "https://api.deepseek.com",
      "apiKey": "DEEPSEEK_API_KEY",
      "models": ["deepseek-chat"]
    },
    {
      "name": "openai",
      "baseUrl": "https://api.openai.com",
      "apiKey": "OPENAI_API_KEY",
      "models": ["gpt-4o", "gpt-4o-mini"]
    }
  ]
}
```

## 项目结构

```
AstrCode/
├── crates/
│   ├── core/        # 纯领域类型、事件存储、投影、注册表
│   ├── runtime/     # AgentLoop、配置与运行态 façade
│   ├── protocol/    # HTTP / SSE / Plugin DTO
│   ├── plugin/      # stdio 插件运行时
│   ├── sdk/         # 插件作者 API
│   ├── tools/       # Tool 实现，不依赖 runtime
│   └── server/      # Axum 本地 server，唯一业务入口
├── src-tauri/       # Tauri 薄壳：sidecar 管理、窗口控制、宿主 GUI 桥接
└── frontend/        # React + TypeScript + Vite UI，共用桌面端和浏览器端
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
│  │  Axum Router│───▶│RuntimeService│──▶│ ToolRegistry│ │
│  └─────────────┘    └──────┬───────┘   └─────────────┘ │
│                            │                            │
│  ┌─────────────┐    ┌──────▼──────┐   ┌─────────────┐ │
│  │  Auth/Token │    │ EventStore  │   │ Protocol DTO│ │
│  └─────────────┘    └─────────────┘   └─────────────┘ │
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
| `/api/sessions/:id/events` | GET (SSE) | 实时事件流 |
| `/api/config` | GET | 获取配置 |
| `/api/models/current` | GET | 当前模型信息 |

### SSE 事件

通过 Server-Sent Events 推送实时更新：

| 事件 | 描述 |
|------|------|
| `phaseChanged` | 阶段变化（Thinking/Streaming/CallingTool/Done） |
| `modelDelta` | 流式文本片段 |
| `toolCallStart` | 工具调用开始 |
| `toolCallResult` | 工具调用结果 |
| `turnDone` | 对话回合结束 |
| `error` | 错误信息 |

## 开发指南

### 代码风格

- Rust：运行 `cargo fmt --all` 格式化
- TypeScript：遵循现有代码风格，运行 `npm run typecheck` 检查

### 测试

```bash
# 运行所有测试
cargo test --workspace

# 前端类型检查
cd frontend && npm run typecheck
```

## 许可证

MIT License

## 致谢

- [Tauri](https://tauri.app/) - 跨平台桌面应用框架
- [React](https://react.dev/) - 前端框架
- [Vite](https://vitejs.dev/) - 构建工具
- [Axum](https://github.com/tokio-rs/axum) - Web 框架
