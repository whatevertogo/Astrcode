# Astrcode 架构总览（2026 重构后）

## 1. 架构目标

本仓库采用“无兼容层重建”策略，目标是把运行时拆成可读、可定位、可扩展的清晰分层：

- `application` 是唯一用例入口。
- `kernel` 是唯一全局控制面。
- `session-runtime` 是唯一会话真相面。
- `core` 只承载领域语义与端口契约，不依赖传输层。
- `adapter-*` 只实现端口，不承载业务真相。

## 2. crate 分层

```text
crates/
├── core/                  # 领域模型、强类型 ID、端口 trait、CapabilitySpec
├── protocol/              # HTTP/SSE DTO 与 wire 类型（仅依赖 core）
│
├── kernel/                # 全局控制：registry / gateway / surface / agent_tree / events
├── session-runtime/       # 会话真相：state / actor / turn / observe / catalog
├── application/           # 用例编排、参数校验、业务错误、治理模型
├── server/                # HTTP/SSE 边界与唯一组合根
│
├── adapter-storage/       # EventStore 等存储实现
├── adapter-llm/           # LlmProvider 实现
├── adapter-prompt/        # PromptProvider 实现
├── adapter-tools/         # 工具定义与桥接
├── adapter-skills/        # Skill 加载
├── adapter-mcp/           # MCP 传输与管理
├── adapter-agents/        # Agent 定义加载
│
├── plugin/
└── sdk/
```

## 3. 依赖规则

允许：

- `protocol -> core`
- `kernel -> core`
- `session-runtime -> core + kernel`
- `application -> core + kernel + session-runtime`
- `server -> application + protocol`
- `adapter-* -> core`

禁止：

- `core -> protocol`
- `application -> adapter-*`
- `kernel/session-runtime -> adapter-*`
- `server` 直接依赖已删除的 `runtime*` 旧门面
- handler 直接绕过 `application` 调底层实现

## 4. 关键边界约束

- `CapabilitySpec` 是运行时内部唯一能力语义模型。
- HTTP 状态码映射只在 `server` 层发生。
- `SessionActor` 不直接持有 provider（tool/llm/prompt/resource），统一经由 `kernel` gateway。
- 公共 API 不暴露内部并发容器（`DashMap`、`RwLock`、`Mutex` 等）。

## 5. 组合根

唯一业务组合根位于：

- `crates/server/src/bootstrap/runtime.rs`

它负责显式装配：

1. adapter 实现
2. `Kernel`
3. `SessionRuntime`
4. `App`
5. `AppGovernance`

`main.rs` 仅保留启动、路由挂载与优雅关闭。

## 6. 命名约定

- 实现层统一使用 `adapter-*` 前缀。
- 旧 `runtime*` 族 crate 已从 workspace 移除。
- 业务入口统一使用 `App` / `AppGovernance`，不再暴露 `RuntimeService` 门面。

## 7. 阅读路径建议

阅读代码时建议按以下顺序：

1. `core`：先理解语义与契约
2. `kernel`：看全局调度与控制
3. `session-runtime`：看单会话执行真相
4. `application`：看用例编排与治理
5. `server`：看 HTTP/SSE 映射与组合根

这样可以避免在历史实现细节中横跳，直接按稳定架构边界理解系统。
