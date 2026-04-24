# Astrcode 项目架构

本文档描述仓库的**当前实际架构约束**。它不是历史说明，也不是愿景文档；实现与本文冲突时，优先修实现或修本文，不保留模糊兼容层。

## 架构原则

- 不维护向后兼容。旧边界阻碍正确架构时，迁移调用点并删除旧类型。
- `server` 是唯一组合根，负责装配、热替换和 HTTP/SSE 暴露。
- `core` 只放稳定、无副作用、无宿主策略的共享语义对象和 durable event 模型。
- contract crate 只定义窄领域契约，不放宿主 I/O、adapter 逻辑或运行时编排。
- owner crate 持有各自真相：
  - `host-session`：session durable truth、投影、fork、input queue、child session。
  - `plugin-host`：插件和外部贡献的 active snapshot。
  - `agent-runtime`：单 turn 执行循环。
  - `context-window`：上下文窗口、compact、请求整形。
- adapter 之间禁止横向依赖。跨 adapter 类型必须下沉到对应 contract 或 owner port。
- durable/live 分层必须清晰：JSONL durable event 是恢复、fork、历史回放的权威事实；SSE live event 只承担低延迟视图更新。
- 类型边界显式转换。不同层的 mode、prompt、tool、runtime event 不得靠 re-export 偷渡。

## 分层视图

```text
entry
  src-tauri       桌面薄壳
  cli             终端 TUI
  eval            评测 runner
  frontend        React UI

client
  astrcode-client HTTP/SSE SDK

composition
  astrcode-server 唯一组合根 + Axum API

adapters
  adapter-agents adapter-llm adapter-mcp adapter-prompt
  adapter-skills adapter-storage adapter-tools

owners / runtime
  host-session plugin-host agent-runtime context-window

contracts
  prompt-contract governance-contract tool-contract
  llm-contract runtime-contract

base
  core protocol support
```

## Crate 职责

### Base

| Crate | 职责 |
|---|---|
| `core` | 强类型 ID、`CapabilitySpec`、phase、durable storage event、错误类型、环境变量名常量 |
| `protocol` | HTTP/SSE/plugin wire DTO；`CapabilityWireDescriptor` 只用于传输边界 |
| `support` | host path、project dir name、shell 检测、大型 tool result 文件引用 |

### Contracts

| Crate | 职责 |
|---|---|
| `prompt-contract` | prompt declaration、prompt source/kind/render target、cache hint |
| `governance-contract` | mode DSL、tool policy、action policy、治理 prompt/model request |
| `tool-contract` | `Tool`、`ToolContext`、tool event sink、stream delta |
| `llm-contract` | `LlmProvider`、request/output、usage、model limits |
| `runtime-contract` | runtime handle、turn event、execution accepted |

### Owners / Runtime

| Crate | 职责 |
|---|---|
| `context-window` | compact、prompt-too-long recovery、tool-result budget、provider request 组装 |
| `agent-runtime` | 单 turn 循环：provider 调用、工具调度、hook dispatch、pending event 输出 |
| `host-session` | session JSONL truth、投影、恢复、fork、input queue、child session、session plan |
| `plugin-host` | builtin/external plugin active snapshot、贡献校验、hook bus、能力调度 |

### Adapters

| Crate | 职责 |
|---|---|
| `adapter-agents` | builtin/user/project agent profile 加载 |
| `adapter-llm` | OpenAI 兼容 Responses / Chat Completions provider |
| `adapter-mcp` | MCP stdio/HTTP/SSE 客户端，工具/提示/资源桥接 |
| `adapter-prompt` | prompt provider，按 prompt declaration 分层组装 |
| `adapter-skills` | Skill 发现、解析、builtin 打包、运行时物化 |
| `adapter-storage` | JSONL event store、config store、MCP settings store |
| `adapter-tools` | 文件、搜索、shell、skill、mode、todo、agent 协作等内置工具 |

### Entry / Composition

| Crate | 职责 |
|---|---|
| `server` | 唯一组合根、HTTP/SSE API、auth、runtime reload、governance surface、agent route bridge |
| `client` | 类型化 HTTP/SSE SDK，只依赖 `protocol` |
| `cli` | ratatui TUI，连接或拉起 server |
| `eval` | task YAML、隔离 workspace、提交 prompt、trace 提取、诊断、评分、report |
| `src-tauri` | Tauri 薄壳：sidecar 管理、窗口能力、系统对话框 |

## 依赖规则

强约束由 `scripts/check-crate-boundaries.mjs` 执行。设计意图如下：

```text
entry -> client -> protocol -> core

server -> all crates

adapter-* -> contract/owner/base
owner/runtime -> contract/base
contract -> core
support -> core
core -> no workspace dependency
```

关键例外：

- `protocol -> governance-contract`：仅用于 mode 相关 DTO。
- `host-session -> agent-runtime`：仅用于 turn/runtime event 宿主集成。
- `adapter-storage` 可作为少数底层存储实现被需要的地方消费，但 adapter 横向依赖不能扩散。

必须通过以下命令验证：

```bash
node scripts/check-crate-boundaries.mjs --strict
node scripts/generate-crate-deps-graph.mjs --check
```

## 核心运行路径

### Prompt 提交

```text
frontend / cli
  -> astrcode-client / protocol DTO
  -> server http routes
  -> root_execute_service
  -> session_runtime_port adapter
  -> host-session accept / begin / persist / complete
  -> agent-runtime turn loop
  -> adapter-llm + adapter-tools
  -> host-session durable JSONL
  -> conversation projection + SSE
```

要点：

- `/api/sessions/{id}/prompts` 是普通会话 prompt 入口。
- `root_execute_service` 负责把 HTTP 请求转为 root execution 请求，并应用治理输入。
- `host-session` 是 durable truth owner；server 只持有 bridge 和组合根资源。

### Agent 协作

```text
spawn / send / observe / close tool
  -> server agent runtime bridge
  -> SubAgentExecutor / CollaborationExecutor
  -> host-session child session / sub-run lineage
  -> agent-runtime child turn
  -> durable collaboration events
```

规则：

- child agent 表现为 child session。
- sub-run lineage 必须可持久化、可恢复、可查询。
- parent/child 通信必须通过协作端口和 input queue，不允许直接改内部状态。

### Config Reload

```text
POST /api/config/reload
  -> ServerGovernanceService
  -> config / MCP / plugin / skill reload
  -> candidate capability surface
  -> RuntimeCoordinator atomic replace
```

运行中存在 session 时拒绝 reload，避免执行中途语义漂移。candidate 构建失败时保留旧 active surface。

### Eval

```text
run-api-eval.mjs
  -> start astrcode-server
  -> exchange auth token
  -> cargo run -p astrcode-eval
  -> create session / submit prompt
  -> wait TurnDone in JSONL
  -> extract trace / diagnose / score / report
```

CI 只跑 smoke eval，验证框架不坏；真实 LLM 能力评估必须用 `npm run eval:api` 或直接调用 `astrcode-eval` 指向真实 server。

## Durable / Live 分层

### Durable

- 存储格式：JSONL append-only。
- 存储路径：`~/.astrcode/projects/<project>/sessions/<session-id>/session-<id>.jsonl`。
- 内容：session start、user input、assistant final、tool call/result、mode、compact、child/sub-run、turn done 等可回放事实。
- 用途：恢复、fork、baseline trace、eval scoring、历史回放。

### Live

- 传输：SSE。
- 内容：流式 delta、临时 thinking、control overlay、conversation block patch。
- 用途：低延迟 UI，不作为历史事实来源。

任何需要刷新后仍存在的状态，都必须能从 durable events 恢复。

## Prompt / Skill / MCP / Plugin

### Prompt

Prompt 由 `adapter-prompt` 根据 `prompt-contract` 组装。治理模式、agent profile、capability prompt、skill summary、MCP/plugin prompt 都必须通过声明式 prompt contribution 进入，不允许 adapter 私自拼 system prompt。

### Skill

加载来源优先级：

```text
builtin < mcp < plugin < user < project
```

Skill 目录格式：

```text
skill-name/
  SKILL.md
  references/
  scripts/
```

### MCP

MCP 作为 plugin-host 的外部贡献来源之一。工具名使用命名空间，避免与 builtin 冲突。MCP prompt/resource/tool 的 wire DTO 进入 server 后必须转换成内部 contract 类型。

### Plugin

plugin-host 维护 active snapshot：

1. 发现 manifest。
2. 校验全局唯一性。
3. 构建 candidate snapshot。
4. 启动 backend。
5. 原子提交。
6. 通过 capability router 调度。

## Governance Mode

`GovernanceModeSpec` 是治理模式 DSL，位于 `governance-contract`。它描述：

- mode id / 展示名 / 说明
- capability selector
- tool policy / action policy
- child agent policy / execution policy
- prompt program / artifact contract / exit gate
- prompt hooks / transition policy

边界要求：

- durable mode event 可使用 `core` 的历史表达。
- runtime/tool/prompt 使用 `governance-contract`。
- 转换点必须明确，通常位于 `server`、`host-session` 或工具事件桥接处。

## HTTP API 摘要

完整路由定义在 `crates/server/src/http/routes/mod.rs`。

| 分类 | 入口 |
|---|---|
| Auth | `/api/auth/exchange` |
| Session | `/api/sessions`, `/api/sessions/{id}/prompts`, `/compact`, `/fork`, `/interrupt`, `/mode` |
| Conversation | `/api/v1/conversation/sessions/{id}/snapshot`, `/stream`, `/slash-candidates` |
| Config / Model | `/api/config`, `/api/config/reload`, `/api/config/active-selection`, `/api/models...` |
| Agent | `/api/v1/agents`, `/api/v1/agents/{id}/execute`, `/api/v1/sessions/{id}/subruns/{sub_run_id}` |
| MCP | `/api/mcp/status`, approval/reject/reconnect/server management |
| Logs | `/api/logs` |

## 验证矩阵

### 日常快速检查

```bash
cargo check --workspace
cargo test --workspace --exclude astrcode --lib
cd frontend && npm run typecheck
```

### 边界变更

```bash
node scripts/check-crate-boundaries.mjs --strict
node scripts/generate-crate-deps-graph.mjs --check
cargo check --workspace
```

### 完整 CI 对齐

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
cd frontend && npm run typecheck && npm run lint && npm run format:check
```

### 真实模型评测

```bash
npm run eval:api -- --task-set eval-tasks/task-set.yaml --concurrency 1
```

## 当前允许的风险

| 风险 | 处理原则 |
|---|---|
| `context-window` 继续增长 | 到 compaction、tool-result budget、request shaping 出现明显独立演化时再拆 |
| plugin backend 形态较多 | 只保留 active snapshot 与调度边界稳定，不提前承诺所有 backend 产品化 |
| eval task 质量仍在建设 | CI 只验证 eval 框架；真实质量由 `eval:api` 报告驱动 |
| server 内 bridge 较多 | bridge 只作为组合根适配层存在，不能重新变成 application facade |
