# 系统架构总览

## 1. 目标

Astrcode 的整体架构追求三件事：

1. **核心契约稳定**：协议、工具、策略、事件和会话边界不被产品细节污染。
2. **运行时可组合**：LLM、prompt、tool、session、plugin、agent control 可以独立演进。
3. **传输层可替换**：HTTP/SSE、桌面壳、前端只是 runtime 的投影，不反向定义内核。

## 2. 总体分层

```text
protocol
   ↑
core
   ↑
storage / runtime-* / plugin
   ↑
runtime
   ↑
server
   ↑
src-tauri + frontend
```

### 2.1 `protocol`

只放跨边界 DTO，不放业务策略和运行时状态。

### 2.2 `core`

只放核心契约：

- Tool
- Capability
- Policy
- Event
- Agent DTO
- Session 持久化接口
- Hook
- Plugin / Runtime 抽象

### 2.3 `storage` / `runtime-*` / `plugin`

这一层负责具体实现，但每个 crate 只做一件事：

- `storage`：durable persistence
- `runtime-llm`：模型调用
- `runtime-prompt`：prompt 组装
- `runtime-session`：session / turn 生命周期
- `runtime-agent-loop`：LLM + tool 主循环
- `runtime-agent-control`：多 Agent 控制面
- `runtime-agent-loader`：Agent profile 加载
- `runtime-agent-tool`：把 `spawnAgent` 暴露成工具
- `runtime-config`：配置
- `runtime-registry`：capability / tool surface
- `runtime-skill-loader`：skill 发现与解析
- `runtime-tool-loader`：内置工具
- `runtime-execution`：执行装配
- `plugin`：插件宿主

### 2.4 `runtime`

`runtime` 是门面层，负责把上述运行时能力组装成统一服务，而不是重复实现子模块逻辑。

### 2.5 `server` / `src-tauri` / `frontend`

这一层只负责把 runtime 投影成可用产品面：

- `server`：HTTP / SSE API
- `src-tauri`：桌面壳
- `frontend`：React SPA

## 3. 依赖规则

### 3.1 必须遵守

- `protocol` 不依赖 `core` / `runtime`
- `runtime-*` crate 之间尽量经由 `core` 契约协作
- `runtime-tool-loader` 只依赖 `core`，不依赖 `runtime`
- `storage` 实现持久化接口，不把持久化细节反推回 `core`
- `runtime` 只做聚合和装配，不复制子 crate 逻辑

### 3.2 尽量避免

- 把 transport / UI 需求直接塞进 `core`
- 让某个大 crate 同时承担协议、执行和产品逻辑
- 把“为了方便调用”的依赖反向打穿分层

## 4. Runtime 主链路

一次典型执行大致经过：

1. `server` 接收请求
2. `runtime` 选择会话与执行入口
3. `runtime-execution` 装配作用域、策略和 prompt 输入
4. `runtime-session` 管理 turn 生命周期与 durable 事件
5. `runtime-agent-loop` 执行 LLM → tool → LLM 循环
6. `storage` 落盘，`server` 通过 SSE 对外广播事件
7. `frontend` 基于 `/history + /events` 归并视图

## 5. 四个稳定核心契约

### 5.1 Tool Contract

定义工具如何声明、执行和返回结果。

### 5.2 Capability Contract

定义能力如何被注册、发现与路由；工具只是 capability 的一种外观。

### 5.3 Policy Contract

定义同步决策面，决定是否允许某次动作发生。

### 5.4 Event Contract

定义 durable 事件与观测事件，保证 replay、UI 和调试链路有统一事实源。

## 6. 会话与 Agent 的位置

当前架构已经把多 Agent 纳入 runtime 主线，但边界很明确：

- Agent as Tool 的设计与协议见 `docs/design` / `docs/spec`
- `runtime-session` 负责 session / turn 真相
- `runtime-agent-control` 负责 agent 控制面
- `runtime-agent-loop` 负责执行循环

不要把这三者重新混成一个“大而全 agent service”。

## 7. 对外接口面

当前产品主要通过以下接口面暴露 runtime：

- HTTP API
- SSE 事件流
- Tauri 桥接
- 插件进程（JSON-RPC）

这些接口面都应该是 runtime 的投影，而不是新的事实源。

## 8. 当前架构刻意不做的事

- 不把前端 render model 固化进后端核心层
- 不把 session tree 变成 runtime 基础领域对象
- 不让子 Agent 控制面与 session 写入模式耦合
- 不为了方便而削弱 crate 边界

## 9. 相关文档

- [./crates-dependency-graph.md](./crates-dependency-graph.md)
- [./frontend-architecture.md](./frontend-architecture.md)
- [./skills-architecture.md](./skills-architecture.md)
- [./agent-loop-roadmap.md](./agent-loop-roadmap.md)
- [../design/README.md](../design/README.md)
- [../spec/README.md](../spec/README.md)
