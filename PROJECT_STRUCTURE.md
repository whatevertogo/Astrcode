# AstrCode 架构设计总纲

> 状态：Draft  
> 目标：作为 AstrCode 的长期架构总纲，只固定**架构设计**与**实现设计约束**，不描述当前实现细节。  
> 更新重点：在保留现有平台原则的基础上，引入面向插件与可替换 Runtime 的 **AstrCode Protocol V4** 设计方向。

---

## 1. 项目定位

AstrCode 是一个**面向编码场景的智能体平台**。

AstrCode 的核心不是某个固定 Agent，也不是某个固定前端，而是一套可扩展的：

- **Server 真源**
- **协议边界**
- **Core 内核**
- **Runtime 执行层**
- **插件系统**
- **SDK 开发模型**

AstrCode 的长期目标是：

- 前端可替换
- Runtime 可替换
- 插件可独立演进
- 协议稳定可扩展
- 会话、事件、工具、状态始终由平台统一管理
- 既优先服务 coding agent，又保留向更通用 agent 平台扩展的空间

### 1.1 冻结 ADR

以下 ADR 作为当前架构边界的冻结决策：

- [ADR-0001: Freeze AstrCode Protocol V4 Wire Model](docs/adr/0001-astrcode-protocol-v4-wire-model.md)
- [ADR-0002: Freeze Coding Profile Context Boundary](docs/adr/0002-astrcode-coding-profile-context.md)
- [ADR-0003: Use Core-Owned Unified Capability Routing](docs/adr/0003-unified-capability-routing.md)

约束：

- `PROJECT_STRUCTURE.md` 负责长期总纲
- ADR 负责冻结关键协议与边界决策
- 实现如果偏离 ADR，必须先更新 ADR 再改代码

---

## 2. 核心原则

### 2.1 平台为主

平台是系统唯一真源，统一管理：

- Session
- Event
- Tool / Capability
- Policy / Permission
- Plugin 生命周期
- Runtime 调度

平台不依赖某个具体 Agent 框架，也不把业务真状态下放到前端或插件内部。

### 2.2 协议优先

所有跨层、跨进程、跨语言通信，先定义协议，再定义实现。

协议是稳定边界；实现可以演进、替换、重构。

### 2.3 插件优先

Tool、Agent、Context Provider、Memory Provider、Policy Hook、Renderer、Trigger Handler 等能力都应优先通过插件扩展。

内核只保留最小必要抽象，不直接承载复杂产品逻辑。

### 2.4 Server is the Truth

前端不是状态真源。  
所有会话、消息、任务、工具调用记录、状态变更都以 Server / Core 侧为准。

### 2.5 Runtime 可替换

Runtime 是执行层，不是平台真源。

平台允许存在多种 Runtime，例如：

- Native Runtime
- Workflow Runtime
- 其他编排 Runtime

但 Runtime 不得反向定义平台边界。

### 2.6 UI 可替换

Desktop、Web、CLI/TUI 共享同一套后端协议与核心能力。

UI 只负责展示与交互，不承载核心业务决策。

### 2.7 DTO 穿边界

跨边界传输一律使用 DTO / 协议消息，不直接传递运行时对象、语言对象、框架对象。

### 2.8 通用骨架，专业 Profile

AstrCode 的协议和插件模型应遵循：

- **消息骨架尽量通用**
- **能力语义允许专业化**
- **Coding 是默认优先 profile，不是唯一 profile**

这意味着：

- `initialize / invoke / result / event / cancel` 这类协议壳应保持稳定、通用
- `workspace / code edit / diagnostics / patch / approval` 等编码语义放入 descriptor、context、profile 扩展，而不是写死在顶层消息结构中

---

## 3. 一句话定义

> AstrCode 不是“一个固定的编码 Agent”，而是一个以 Server 为真源、以协议为边界、以插件为扩展单位、以 Runtime 为可替换执行层、以 coding profile 为第一优先的智能体平台。

---

## 4. 总体架构

```text
┌────────────────────────────────────────────┐
│                 Frontends                  │
│        Desktop / Web / CLI / TUI           │
└─────────────────────┬──────────────────────┘
                      │ HTTP / SSE / WS
┌─────────────────────▼──────────────────────┐
│              AstrCode Server               │
│      API / stream / state access entry     │
└─────────────────────┬──────────────────────┘
                      │ internal contracts
┌─────────────────────▼──────────────────────┐
│               Core Kernel                  │
│ session / event / registry / policy        │
└───────────────┬───────────────┬────────────┘
                │               │
        ┌───────▼───────┐   ┌───▼──────────────────┐
        │ Agent Runtime │   │ Plugin Runtime       │
        │ replaceable   │   │ peer / router / io   │
        └───────┬───────┘   └──────────┬───────────┘
                │                      │
        ┌───────▼────────┐      ┌──────▼─────────────┐
        │ Built-in caps  │      │ External plugins   │
        │ & orchestrator │      │ & SDK implementations│
        └────────────────┘      └────────────────────┘
```

---

## 5. 分层职责

### 5.1 Frontends

负责：

- 输入输出
- 交互呈现
- 事件订阅
- 局部缓存
- 乐观更新

不负责：

- 会话真状态
- 工具调度
- 插件管理
- 核心业务决策
- 持久化真源

### 5.2 Server

负责：

- 提供统一 API
- 提供流式事件出口
- 暴露状态读取入口
- 协调前端与内核

约束：

- Server 是唯一业务入口
- 不允许前端绕过 Server 直接写 Core 状态

### 5.3 Core Kernel

负责：

- SessionManager / SessionRepository
- EventStore
- Projection
- ToolRegistry / CapabilityRegistry
- PluginRegistry
- PolicyEngine / PermissionEngine
- RuntimeCoordinator

不负责：

- 具体 Agent 策略
- UI 逻辑
- 某个第三方框架的内部模型

### 5.4 Agent Runtime

负责：

- 单轮生成
- 工具调用循环
- 任务拆解
- Planner / Executor / Verifier 协调
- 模型调用与流式输出
- 将执行层结果映射成统一事件流

约束：

- Runtime 可替换
- Runtime 不得定义平台真状态
- Runtime 不得绕过 Core 直接修改持久化结构

### 5.5 Plugin Runtime

负责：

- 插件加载
- 握手初始化
- 注册能力与 handler
- 生命周期管理
- 隔离执行
- 请求分发
- 流式事件转发
- 取消传播

约束：

- 插件通过协议接入平台
- 插件不直接嵌入 Core 内存模型
- 插件只能调用平台公开能力

### 5.6 SDK

负责：

- 为插件作者提供稳定开发接口
- 封装协议细节
- 提供 descriptor / context / stream helper
- 提供声明式注册模型
- 提供 coding profile 的高级 helper

约束：

- SDK 不是协议本身
- 协议必须独立于 SDK 存在
- SDK 对象不得直接作为跨边界传输格式

---

## 6. 核心对象模型

### 6.1 Session

Session 是平台一级对象，表示一次工作会话。

最小语义应包含：

- 唯一标识
- 标题
- 工作区
- 创建时间
- 更新时间
- Runtime 配置
- 当前状态

要求：

- Session 生命周期由平台统一管理
- Session 读取应基于 Projection
- Session 修改应通过事件驱动

### 6.2 Event

Event 是系统唯一可追溯事实。

所有重要状态变化都应可以由事件重建，包括但不限于：

- 用户输入
- 模型输出
- 工具调用
- 任务状态变化
- 会话元数据变化
- 错误与中断
- 压缩与归档

要求：

- Event 追加写入
- Event 不应被静默篡改
- Event 应支持回放与追踪

### 6.3 Projection

Projection 是事件投影结果，用于高效读取。

典型用途：

- 会话列表
- 对话快照
- 工具历史
- 任务状态
- 当前上下文摘要

要求：

- Projection 可重建
- Projection 不作为事实真源
- Projection 更新必须来源于事件流

### 6.4 Capability

Capability 是平台统一能力抽象。

Capability 可来源于：

- 内置能力
- 插件能力
- Runtime 暴露能力
- 平台系统能力

要求：

- 必须有稳定名称
- 必须有输入/输出边界
- 必须可受权限系统控制
- 必须允许按 profile 和 tag 进行筛选

---

## 7. AstrCode Protocol V4 设计

### 7.1 协议目标

AstrCode Protocol V4 是平台核心与插件 / 外部 worker / 可替换 Runtime 之间的协议边界。

它必须同时满足两类目标：

- **对 AstrCode 来说足够专业**：能表达 coding agent 所需的 workspace、code edit、artifact、diagnostics、approval 等语义
- **对更通用 agent 来说足够开放**：不把消息骨架写死成“只能处理代码文件”的专用协议

### 7.2 协议形态：Core Protocol + Execution Profile

AstrCode Protocol V4 采用双层模型：

#### Core Protocol

Core Protocol 固定定义：

- peer 身份
- capabilities / handlers 描述
- 调用与取消
- 流式生命周期
- 错误模型
- 版本协商

#### Execution Profile

Execution Profile 用于表达特定场景的专业语义。

首个官方 profile 为：

- `coding`

未来允许扩展：

- `general-agent`
- `workflow`
- `chat`

约束：

- profile 扩展不能破坏 Core Protocol 基本结构
- profile 语义优先放入 descriptor、context、event taxonomy，而不是顶层消息壳

### 7.3 协议分层

AstrCode 插件运行时协议采用四层结构：

```text
┌─────────────────────────────────────────┐
│            Business Logic               │
│ tool / agent / context / memory / hook │
├─────────────────────────────────────────┤
│ Capability Router / Handler Dispatcher │
├─────────────────────────────────────────┤
│             Protocol Peer               │
├─────────────────────────────────────────┤
│               Transport                 │
└─────────────────────────────────────────┘
```

各层职责如下：

- **Transport**：只负责字符串收发，不理解协议业务
- **Protocol Peer**：处理握手、消息解析、请求响应匹配、流式事件分发、取消传播、连接收敛
- **Capability Router / Handler Dispatcher**：基于 descriptor 做业务路由、权限检查与本地 handler 分发
- **Business Logic**：插件或平台真正实现的 tool / agent / context / memory / hook 逻辑

### 7.4 基础消息集合

AstrCode Protocol V4 基础消息固定为五类：

- `InitializeMessage`
- `InvokeMessage`
- `ResultMessage`
- `EventMessage`
- `CancelMessage`

说明：

- `InitializeMessage`：发起握手
- `ResultMessage`：既可作为普通调用结果，也可作为初始化结果
- `EventMessage`：流式事件，承担 started / delta / completed / failed 生命周期
- `CancelMessage`：取消进行中的调用

长期约束：

- 不轻易新增第六类消息
- 新语义优先通过字段追加或 descriptor 扩展实现

### 7.5 对等端模型

协议两端是**对等端（Peer）**，但不意味着平台和插件拥有相同权限。

Peer 至少应声明：

- `id`
- `name`
- `role`
- `version`
- `protocol_version`
- `supported_protocol_versions`
- `supported_profiles`
- `metadata`

推荐 role：

- `core`
- `plugin`
- `runtime`
- `worker`
- `supervisor`

原则：

- 协议是对等的
- 权限不是对等的
- Core 始终拥有 policy 和 state authority

### 7.6 Descriptor 设计

AstrCode Protocol V4 保留 v4 的 descriptor 思想，但做 AstrCode 化收敛。

#### CapabilityDescriptor

用于声明“我能被调用什么”。

最小语义应包含：

- `name`
- `kind`
- `description`
- `input_schema`
- `output_schema`
- `streaming`
- `profiles`
- `tags`
- `permissions`
- `side_effects`
- `stability`

推荐 kind：

- `tool`
- `agent`
- `context_provider`
- `memory_provider`
- `policy_hook`
- `renderer`
- `resource`

#### HandlerDescriptor

用于声明“我会响应什么触发”。

适用于：

- 平台事件订阅
- hook 回调
- command / trigger 型插件
- 未来的通用 agent 事件处理

最小语义应包含：

- `id`
- `trigger`
- `input_schema`
- `profiles`
- `filters`
- `permissions`

### 7.7 Capability 命名空间

为了既服务 coding，又保留通用 agent 空间，Capability 应采用 namespaced naming。

首批推荐命名空间：

- `code.*`
- `workspace.*`
- `repo.*`
- `diagnostics.*`
- `artifact.*`
- `memory.*`
- `agent.*`
- `message.*`
- `system.*`

原则：

- Coding 域优先设计
- 非 coding 域允许存在
- 不把“通用”与“编码”混成一个巨大 capability

### 7.8 调用模型

统一调用模型：

- 输入：`capability + input + context`
- 输出：`success | error + output`
- 可选流：`started -> delta* -> completed | failed`

长期约束：

- 普通调用与流式调用使用同一逻辑模型
- 调用必须可追踪
- 调用必须支持取消传播
- 调用必须能附带 profile-aware context

### 7.9 InvocationContext 设计

为了兼顾通用性与 coding 优先，调用上下文分为两层：

#### Base Context

适用于所有 profile：

- `request_id`
- `trace_id`
- `session_id`
- `caller`
- `workspace`
- `deadline`
- `budget`
- `profile`
- `metadata`

#### Profile Context

按 profile 扩展。

`coding` profile 推荐至少支持：

- `working_dir`
- `repo_root`
- `open_files`
- `active_file`
- `selection`
- `branch`
- `approval_mode`
- `output_mode`

原则：

- 通用字段放 Base Context
- 编码语义放 Coding Profile Context
- 不把 UI 专属字段写进通用层

### 7.10 StreamExecution 模型

流式执行是 v4 的核心能力，AstrCode 应保留。

流式生命周期固定为：

- `started`
- `delta`
- `completed`
- `failed`

其中 `delta` 阶段承载不同 event taxonomy，例如：

- `message.delta`
- `reasoning.delta`
- `tool.call.started`
- `tool.call.completed`
- `artifact.patch`
- `artifact.file`
- `progress.step`
- `diagnostic`
- `log`

原则：

- 生命周期字段尽量稳定
- 领域语义通过 `event name / channel / payload schema` 表达
- 不为每种业务事件再发明新的顶层消息种类

### 7.11 错误模型

统一错误结构至少包含：

- `code`
- `message`
- `hint`
- `retryable`
- `details`

要求：

- 错误码使用稳定字符串
- 不暴露语言绑定异常类型作为协议标准
- 错误语义在不同 Runtime / 插件间保持一致

### 7.12 权限与副作用模型

CapabilityDescriptor 应显式声明权限与副作用。

推荐权限维度：

- `filesystem.read`
- `filesystem.write`
- `process.exec`
- `network.http`
- `repo.write`
- `secrets.read`
- `model.invoke`

推荐副作用级别：

- `none`
- `local`
- `workspace`
- `external`

原则：

- 权限声明是协议的一部分
- 实际授权仍由 Core / Policy 决定
- 插件不因“能声明”而自动“能执行”

### 7.13 版本兼容

要求：

- 协议优先追加字段，避免破坏性变更
- 新能力通过 capability 扩展，不污染基础消息语义
- 版本协商发生在初始化阶段
- profile 版本与 core protocol 版本应可独立演进

---

## 8. 传输层设计

### 8.1 目标

传输层只负责消息收发，不承载业务语义。

### 8.2 支持形态

默认支持：

- `stdio`
- `websocket`

后续可扩展其他传输实现，但上层协议不应感知传输差异。

### 8.3 设计约束

- 传输层不直接处理业务状态
- 协议与传输解耦
- 任何 Transport 失效都不应污染协议语义
- 传输错误必须向上收敛为统一错误模型

---

## 9. 插件系统设计

### 9.1 插件定位

插件是一等公民。  
插件不仅可以提供 Tool，也可以提供更高层能力。

插件可贡献的典型扩展单元包括：

- Tool
- Agent
- Context Provider
- Memory Provider
- Policy Hook
- Renderer
- Trigger Handler
- 动态注册Skill
- 动态注册mcp

### 9.2 插件角色

插件体系内部推荐区分：

- **Supervisor**：平台侧进程管理、加载、生命周期监管
- **Worker**：插件侧实际执行单元
- **Peer**：协议通信抽象

原则：

- 监管、传输、业务逻辑三者分离
- 插件协议层和插件业务层分离

### 9.3 插件边界

插件与平台之间只通过协议交互。

要求：

- 插件不得依赖 Core 私有实现
- 插件不得直接写入平台内部状态
- 插件只能调用权限允许范围内的平台公开能力

### 9.4 插件隔离

要求：

- 插件运行必须具备隔离边界
- 插件崩溃不应破坏平台整体可用性
- 插件异常必须被统一收敛与记录

### 9.5 插件发现与注册

要求：

- 插件初始化后显式声明自身 capabilities / handlers
- 平台统一维护注册表
- 能力冲突必须有确定性处理策略

---

## 10. Runtime 设计

### 10.1 Runtime 定位

Runtime 是“如何执行任务”的实现层，不是“系统真相”的保存层。

### 10.2 允许形态

AstrCode 允许多个 Runtime 并存，例如：

- Native Runtime
- Workflow Runtime
- 其他编排 Runtime

### 10.3 设计约束

- Runtime 通过统一接口接入 Core
- Runtime 输出统一事件流
- Runtime 不拥有最终会话真状态
- Runtime 不得绑定平台到某个具体外部框架

### 10.4 推荐职责

Runtime 适合承载：

- Prompt 组装
- 模型调用
- 工具循环
- 子代理协调
- 思维流 / 推理流转译
- 任务执行策略

### 10.5 Runtime 与协议关系

Runtime 本身不应重定义插件协议，但可以：

- 暴露 capabilities
- 作为 peer 参与调用链
- 消费统一流式事件
- 将内部执行结果转译为平台统一事件模型

---

## 11. SDK 设计

### 11.1 SDK 定位

SDK 面向插件作者，而不是面向平台内部实现。

### 11.2 SDK 职责

SDK 应封装：

- 消息编解码
- descriptor 构造
- context 读取
- stream 输出 helper
- 错误构造 helper
- coding profile helper

### 11.3 SDK 设计约束

- SDK 不是协议本身
- SDK 不直接暴露 Core 私有类型
- Coding helper 应作为 profile 扩展存在，而不是污染所有插件作者 API

---

## 12. 存储设计

### 12.1 存储原则

AstrCode 的持久化设计遵循：

- 文件系统优先
- 事件追加式写入
- Projection 独立构建
- 状态可重放
- 结构可迁移

### 12.2 事实与视图分离

要求：

- Event 是事实真源
- Projection 是读取视图
- Snapshot 只是优化手段，不替代事件真源

### 12.3 删除策略

要求：

- 删除必须是显式行为
- 删除后系统状态应保持可解释
- 不允许隐式悬挂状态

---

## 13. UI 与宿主边界

### 13.1 UI 边界

UI 只负责：

- 展示
- 输入
- 订阅事件
- 局部体验优化

UI 不负责：

- 真状态管理
- Agent 调度
- 插件生命周期
- 核心存储语义

### 13.2 桌面宿主边界

桌面宿主（如 Tauri）只承担：

- 启动服务
- 系统桥接
- 窗口控制
- 桌面能力接入

要求：

- 宿主层不承载核心业务逻辑
- 宿主层不成为真状态来源

---

## 14. 实现设计约束

以下约束应长期固定：

### 14.1 Core 约束

- Core 不依赖 UI
- Core 不直接依赖某个具体 Agent 框架
- Core 只保留平台级抽象
- Core 内模型优先保持纯领域化

### 14.2 Protocol 约束

- 协议独立于 SDK
- 协议先于实现稳定
- 跨边界只传 DTO
- 协议字段尽量追加不替换
- 通用骨架与 profile 扩展必须分层

### 14.3 Plugin 约束

- 插件通过协议接入
- 插件能力必须显式声明
- 插件只能使用公开能力
- 插件安全边界由平台控制

### 14.4 Runtime 约束

- Runtime 可替换
- Runtime 只负责执行，不负责真状态
- Runtime 输出必须被 Core 接纳为统一事件模型

### 14.5 Frontend 约束

- Frontend 不直接写真状态
- Frontend 不绕过 Server
- Frontend 对 Server 保持协议兼容

### 14.6 Storage 约束

- Event 作为事实真源
- Projection 可重建
- 不以临时缓存替代正式持久化

---

## 15. 非目标

当前阶段，AstrCode 不以以下目标为优先：

- 分布式多机调度
- 公网多租户安全模型
- 重数据库优先架构
- 对单一 Agent 框架的深度绑定
- 大而全的插件市场系统

AstrCode 现阶段优先保证：

- 单机架构闭环清晰
- 协议边界稳定
- 插件扩展模型成立
- Runtime 可替换
- UI 可替换

---

## 16. 演进原则

后续演进应尽量遵守：

1. 新能力优先做成插件或 Runtime 扩展  
2. 新协议字段优先追加，避免破坏兼容  
3. 新前端不得绕过 Server 改写核心状态  
4. 新实现不得把第三方框架反向注入 Core 边界  
5. 能用 DTO 表达的边界，不直接传运行时实例  
6. 所有重要状态变化都应尽量事件化  
7. 所有可替换组件都应优先面对抽象而不是面对实现  
8. 基础协议优先保持通用，专业语义优先下沉到 profile / descriptor / context  

---

## 17. 总结

AstrCode 的核心设计不是“做一个固定写法的编码 Agent”，而是先建立一套长期稳定的平台边界：

- Server 统一真源
- Core 管理平台抽象
- Protocol 定义跨边界通信
- Runtime 负责执行
- Plugin 负责扩展
- SDK 负责开发体验
- UI 负责展示与交互

在这个边界稳定之后，具体 Agent、具体模型、具体前端、具体插件形态都可以持续演进，而不破坏系统基础结构。

AstrCode Protocol V4 的长期方向是：

- 保留 v4 的分层设计
- 保留五类基础消息
- 保留 `Peer / Router / StreamExecution` 思想
- 把 coding 能力做成第一优先 profile
- 同时不给未来通用 agent 演进封路
