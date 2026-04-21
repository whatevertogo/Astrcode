## Context

`astrcode-core` 当前同时承担了三类职责：

- 纯语义模型与稳定端口契约
- 单 session durable replay / projection 算法
- 文件系统、shell、进程协调等环境副作用

这让 `core` 变成了“什么都能放”的大仓库，直接后果是：

- `session-runtime` 无法完整拥有自己的 projection / replay 真相
- `application` 的治理与全局协调边界不清晰
- adapter 层与领域层之间的副作用边界持续漂移
- `agent/mod.rs` 这类入口文件不断膨胀，难以维护

这次 change 的目标不是引入新的“万能工具层”，而是把已经存在但放错位置的职责迁回正确 owner：`core` 留下纯语义与稳定契约，`session-runtime` 拥有会话 replay/projection 真相，`server` 组合根拥有运行时设施协调，`application` 只消费稳定治理/路径契约，副作用实现落到 adapter 或职责受限的 `astrcode-support`；对于跨多个 crate 共享、但又不应继续滞留在 `core` 的宿主能力，则由 `astrcode-support` 统一承接。

## Goals / Non-Goals

**Goals:**

- 让 `core` 收敛为“纯语义类型 + 稳定契约 + 无副作用辅助逻辑”
- 把 input queue replay、turn projection snapshot 等会话投影逻辑迁回 `session-runtime`
- 把 `RuntimeCoordinator` 与等价的全局运行时协调语义迁到 `server` 组合根附近
- 把 tool result persist、project path 解析、shell/process 探测等环境副作用迁到 adapter 端口后面或 `astrcode-support` 这样的受限共享宿主 crate
- 拆分 `core/agent/mod.rs`，在不改变对外语义的前提下恢复模块边界
- 去掉 `reqwest`、`dirs`、`toml` 这些由 owner 错位实现带进 core 的具体依赖

**Non-Goals:**

- 允许新增一个受限的共享基础设施 crate：`astrcode-support`
- 不改写已经合理的 `core` trait / type 定义，只调整 owner 错位的实现
- 不把 `kernel` 重新变成运行时状态 owner
- 不改变 HTTP、SSE 或前端消费协议
- 不在本次 change 内抽象 `tokio::sync::mpsc::UnboundedSender`
- 不在本次 change 内迁移 `TurnProjectionSnapshot` 的类型 owner

## Decisions

### D1: `core` 只保留纯语义与稳定契约

`core` 保留以下内容：

- 领域语义类型、ID、新旧层共享的稳定 DTO
- port trait / gateway trait
- 不依赖进程内状态、文件系统或 shell 的纯函数算法

以下内容不再允许留在 `core`：

- 会话 durable replay / projection 真相
- 全局运行时协调状态
- 文件系统 canonicalize / working dir 归一化等 IO 逻辑
- shell / process 探测与命令执行
- durable tool result 落盘实现

这样可以把 `core` 恢复成所有层都能稳定依赖、但不会反向拖入实现细节的基础层。

### D2: 会话 replay / projection 算法由 `session-runtime` 完整拥有，共享 checkpoint 载体暂留 core

`InputQueueProjection` 的 replay / 恢复算法以及等价的单 session durable 派生事实都迁回 `session-runtime`。

判断标准很简单：凡是“需要依赖 session event 流恢复”或“属于单 session authoritative read model”的逻辑，都应由 `session-runtime` 拥有，而不是留在 `core` 作为通用工具。

这保证了 `session-runtime` 既拥有 durable 事件，也拥有由这些事件导出的 projection 真相，避免 `core` 里再藏一个会话子运行时。

`TurnProjectionSnapshot` 本次不直接迁移类型 owner。原因是它目前仍是 `SessionRecoveryCheckpoint` / `ProjectionRegistrySnapshot` / `EventStore` checkpoint 合同的一部分，若强行迁入 `session-runtime`，会把 `core` trait 反向绑到 `session-runtime`，形成新的循环依赖。
本次只要求 projector / query / watcher 语义继续归 `session-runtime`，共享 checkpoint 载体先留在 core，等待后续 checkpoint 边界清理。

### D3: 全局运行时协调归 `server` 组合根，而不是 `application`

`RuntimeCoordinator` 及其等价语义不是 `core` 基础能力，也不是 `application` 业务用例；它本质上是组合根的运行时设施。

迁移后：

- `server/bootstrap/*` 拥有 `RuntimeCoordinator` 的具体实现与生命周期
- `application` 继续只通过治理端口消费这些能力，而不是持有设施 owner
- `core` 不再持有全局可变状态 owner

这样可以把“全局控制面”从基础层剥离出来，同时让 owner 停留在最自然的组合根位置，而不是错误地下沉到业务层。

### D4: 环境副作用通过 adapter 端口实现，core 只保留协议与纯 helper

`tool_result_persist.rs`、`shell.rs`、`project.rs`、`home.rs`、`plugin/manifest.rs` 里的环境能力迁到 adapter、`astrcode-support` 或组合根端口后面。

迁移后的边界原则：

- `core` 只定义所需的稳定语义和端口
- `application` / `session-runtime` 只编排这些端口
- 真实的文件读写、路径解析、shell 调用由 `adapter-*` 或 `astrcode-support` 实现

细分策略如下：

- `tool_result_persist`: core 保留结果引用 DTO、常量与字符串/路径解析 helper；落盘实现迁入 `astrcode-support::tool_results`
- `shell`: core 保留 `ShellFamily`、`ResolvedShell`；shell 检测与命令存在性检查迁入 `astrcode-support::shell`
- `project`: core 保留 slug/hash 等纯 project identity 算法；`canonicalize`、Astrcode home / projects 路径拼装迁入 `astrcode-support::hostpaths`
- `home`: 迁出 core，由 `astrcode-support::hostpaths` 提供统一 home 目录解析
- `plugin manifest`: core 保留 `PluginManifest` 数据结构，TOML 解析迁出

这里允许新增一个受限的共享 crate：`astrcode-support`。它不是通用 `utils` 桶，只承载边界明确、跨多个 crate 共享的宿主能力；当前子域为 `hostpaths`、`shell`、`tool_results`。

### D5: `core::agent` 维持外部语义，内部按职责拆分

`core/agent/mod.rs` 的问题是组织方式，不是能力本身。

本次不改变外部公共语义，只把大文件按职责拆回子模块，例如：

- agent 定义/身份语义
- 运行配置或静态元数据
- 与执行无关的共享值对象

目标是降低入口文件复杂度，让 `core` 内部结构能反映真实子域，而不是继续把无关概念堆在单个 `mod.rs` 中。

### D6: 依赖瘦身以 owner 迁移为驱动，不为“零依赖”强行改 execution contracts

`EventStore` 等现有 trait 先以“是否阻碍 owner 迁移”为标准评估。

如果现有 trait 仍能表达迁移后的调用方向，就保留并仅调整实现 owner；只有当某个 trait 同时混入了恢复、协调、落盘等跨层语义，才进行最小拆分。

这样可以避免“边迁移边重写全部契约”，把范围控制在本次 change 真正要解决的问题上。

具体到依赖层面：

- `reqwest`：直接从 `AstrError` 解耦，属于低风险、应立即处理的基础层瘦身
- `dirs`：随着 `home.rs` 迁出一起移除
- `toml`：随着 `PluginManifest::from_toml` 迁出一起移除
- `tokio`：当前体现在 `ToolContext` / `CapabilityContext` 对 `UnboundedSender` 的直接绑定，改动会触达 `core`、`kernel`、`session-runtime` 与 adapter 执行合同；本次不硬塞，留作后续 change

## Risks / Trade-offs

- [Risk] `session-runtime` 与 `application` 之间可能出现新的 helper 循环依赖
  - Mitigation：坚持“单 session 真相进 `session-runtime`，全局协调留在 `server` 组合根，`application` 只消费端口”，不引入跨层便捷函数
- [Risk] tool result persist / project path / shell 探测落点不清，导致 support / adapter 侧再次扩散
  - Mitigation：先固化 capability spec，要求所有环境副作用都经稳定契约进入 adapter 或 `astrcode-support`
- [Risk] `core::agent` 拆分时误伤外部导出路径
  - Mitigation：优先保持 crate 根 re-export 稳定，只调整内部模块组织
- [Risk] 过早迁移 `TurnProjectionSnapshot` 造成 `EventStore` / checkpoint 循环依赖
  - Mitigation：本次显式延期该类型 owner 迁移，只收口算法 owner
- [Risk] 为了去掉 `tokio` 而扩大 execution contract 改动面
  - Mitigation：本次不处理 `UnboundedSender` 抽象，单独作为后续瘦身 change
- [Trade-off] 新增 `astrcode-support` 会引入一个新的共享依赖点
  - Mitigation：严格限制其职责为 `hostpaths`、`shell`、`tool_results` 这类受限子域，禁止演化成泛化 `utils` 桶
