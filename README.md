# AstrCode

一个基于 Tauri 的 AI 编程助手桌面应用，支持 OpenAI 兼容 API 的流式对话、多工具调用和会话管理。

## 功能特性

- **多模型支持**：支持 OpenAI 兼容 API（DeepSeek、OpenAI 等），运行时可切换 Profile 和 Model
- **流式响应**：实时显示 AI 生成的代码和文本
- **多工具调用**：内置文件操作、代码搜索、Shell 执行等工具
- **会话管理**：支持多会话切换、按项目分组、会话历史浏览
- **跨平台**：支持 Windows、macOS、Linux

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

# 安装 Tauri CLI（可选，用于构建）
cargo install tauri-cli
```

### 开发模式

```bash
# 方式一：Tauri 开发模式（推荐）
cargo tauri dev

# 方式二：仅前端开发
npm run dev
# 然后打开 http://127.0.0.1:5173/
```

### 构建

```bash
cargo tauri build
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
│   ├── core/           # Agent 核心逻辑
│   │   ├── agent_loop.rs    # 对话循环
│   │   ├── runtime.rs       # 运行时管理
│   │   ├── config.rs        # 配置系统
│   │   ├── event_log.rs     # 事件持久化
│   │   ├── provider_factory.rs  # Provider 工厂
│   │   └── tools/           # 工具实现
│   └── ipc/            # IPC 类型定义
├── src-tauri/          # Tauri 应用入口
├── frontend/           # React + TypeScript 前端
│   └── src/
│       ├── components/ # UI 组件
│       ├── hooks/      # React Hooks
│       └── types.ts    # 类型定义
└── CLAUDE.md           # Claude Code 指南
```

## 架构

### Agent Loop

基于 Turn 的执行模式：

```
1. 接收用户输入
2. 调用 LLM API
3. 执行工具调用
4. 发送事件到前端
5. 重复（最多 8 步）
```

### IPC 协议

后端通过事件驱动前端更新：

| 事件 | 描述 |
|------|------|
| `SessionStarted` | 会话创建 |
| `PhaseChanged` | 阶段变化（Thinking/Streaming/CallingTool/Done） |
| `ModelDelta` | 流式文本片段 |
| `ToolCallStart` | 工具调用开始 |
| `ToolCallResult` | 工具调用结果 |
| `TurnDone` | 对话回合结束 |
| `Error` | 错误信息 |

## 开发指南

### 代码风格

- Rust：运行 `cargo fmt --all` 格式化
- TypeScript：遵循现有代码风格

### 测试

```bash
# 运行所有测试
cargo test --workspace

# 运行特定 crate 测试
cargo test -p astrcode-core
```

## 许可证

MIT License

## 致谢

- [Tauri](https://tauri.app/) - 跨平台桌面应用框架
- [React](https://react.dev/) - 前端框架
- [Vite](https://vitejs.dev/) - 构建工具
