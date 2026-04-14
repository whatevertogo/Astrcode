# Astrcode 架构总览（长期目标）

## 1. 架构目标

本仓库采用“无兼容层重建”策略。  
重构的目标不是把旧 `runtime` 分拆成更多 crate，而是把系统收敛成一套**长期稳定、可读、可定位、可扩展**的边界：

- `application` 是唯一用例入口与治理入口。
- `kernel` 是唯一全局控制面。
- `session-runtime` 是唯一单会话真相面。
- `core` 只承载领域语义、强类型 ID、端口契约与稳定配置模型。
- `adapter-*` 只实现端口，不承载业务真相，不偷渡业务策略。
- `server` 只承担传输边界与组合根，不重新长成新的 Runtime 门面。

这份文档描述的是**长期预期**，不是某一轮迁移中的临时落地手法。

## 2. crate 分层

```text
crates/
├── core/                  # 领域模型、强类型 ID、端口 trait、CapabilitySpec、稳定配置模型
├── protocol/              # HTTP/SSE DTO 与 wire 类型（仅依赖 core）
│
├── kernel/                # 全局控制：gateway / surface / registry / agent_tree / events
├── session-runtime/       # 单会话真相：state / catalog / actor / turn / context / factory / query
├── application/           # 用例编排、参数校验、权限/策略、错误归类、治理模型
├── server/                # HTTP/SSE 边界与唯一组合根入口
│
├── adapter-storage/       # EventStore、ConfigStore、审批持久化等存储实现
├── adapter-llm/           # LlmProvider 实现
├── adapter-prompt/        # PromptProvider 实现
├── adapter-tools/         # 内置工具定义与 capability 桥接
├── adapter-skills/        # Skill 加载与物化
├── adapter-mcp/           # MCP 传输、server 管理、resource 接入
├── adapter-agents/        # Agent 定义加载
│
├── plugin/                # 插件模型与宿主侧基础设施
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

条件允许但要克制：

- `server -> adapter-*`
  仅允许发生在组合根装配处，不允许渗入 handler 或业务用例层。

禁止：

- `core -> protocol`
- `application -> adapter-*`
- `kernel -> adapter-*`
- `session-runtime -> adapter-*`
- `server` 重新依赖已删除的 `runtime*` 旧门面
- handler 绕过 `application` 直接调用底层实现
- 为迁移方便引入新的“大一统 façade”

## 4. 长期边界约束

### 4.1 `core`

- `core` 不关心 HTTP、SSE、Tauri、CLI 等传输细节。
- `core` 不关心具体 provider、具体存储、具体工具实现。
- `core` 负责稳定领域语义，包括：
  - 强类型 ID
  - 领域事件与记录
  - 端口契约
  - `CapabilitySpec`
  - 稳定配置模型

### 4.2 `protocol`

- `protocol` 只承载传输层稳定数据结构，包括：
  - HTTP request/response DTO
  - SSE event DTO
  - wire-level enum / payload
- `protocol` 可以表达：
  - 字段命名
  - 可选字段
  - 兼容性序列化结构
  - 面向传输的扁平化表示
- `protocol` 不决定领域建模：
  - 不能反向要求 `core` 为某个 JSON 形状改模型
  - 不能引入传输层特有语义进入领域层
- `server` 负责：
  - `protocol <-> application/core` 的转换
  - HTTP 状态码映射
  - SSE 流封装
- handler 中允许存在薄 DTO 转换逻辑，但禁止：
  - 在 handler 中做业务规则判断
  - 在 handler 中拼 session 真相或治理策略
  - 让 `protocol` 成为事实源

### 4.3 `application`

- `application::App` 是同步业务用例的唯一入口。
- `application::AppGovernance` 是治理、重载、观测、生命周期入口。
- `application` 必须真正承担：
  - 参数校验
  - 权限检查
  - 配额与策略判断
  - 审批决策入口
  - 错误归类
  - reload 编排
- `application` 不保存 session shadow state。
- `application` 不直接依赖 adapter 实现。
- `application` 不直接理解 `agent_tree` 内部结构。

### 4.4 `kernel`

- `kernel` 是全局控制面，不是新的业务巨石。
- `kernel` 负责：
  - capability router / registry
  - tool / llm / prompt / resource gateway
  - agent tree 与全局控制合同
  - 统一 capability surface 的可见性
  - 全局事件协调
- `kernel` 不承接单 session 真相。
- `kernel gateway` 必须保持“轻量寻址层”定位：
  - 负责定位、鉴权、拿句柄
  - 不做重业务编排
  - 不做重序列化
  - 不做新的全局大锁
- `kernel` 只能做的典型事情：
  - 根据当前 capability surface 把 `readFile`、`call_llm`、`read_resource` 这类请求路由到正确 provider
  - 对某个 root agent / subrun 提供稳定的 query / observe / close / wake 控制合同
- `kernel` 明确不能做的事情：
  - 不能决定某个 session 的 prompt 应如何裁剪、是否 compact、是否 auto-continue
  - 不能在 gateway 内拼装业务级审批、配额、profile 选择、turn 策略

这些限制的判断标准不是“代码多不多”，而是看它回答的是不是“全局控制与能力寻址”这个问题域。

子 Agent 的能力裁剪也遵循同一原则：

- `AgentProfile` 只描述行为模板，例如 system prompt、模型偏好、协作风格。
- `spawn` 时的 task-scoped capability grant 只描述“这次任务申请的最小工具子集”。
- child 的最终 capability surface 必须由父级当前可继承能力面、spawn grant、runtime availability 与既有 policy 护栏求交得到。
- prompt 可见工具集合与 runtime 可执行工具集合必须读取同一份 filtered capability router，不能一边看 profile、一边看全局 registry。

### 4.5 `session-runtime`

- `session-runtime` 是单 session 真相面，不是新的超级 runtime。
- 只有回答“某个 session 当前发生了什么、如何推进一次 turn”的代码，才能进入 `session-runtime`。
- `session-runtime` 负责：
  - create/load/list/delete
  - history/view/replay
  - interrupt/compact/run_turn
  - branch/subrun 的单 session 执行真相
  - child delivery / mailbox / observe 的会话内推进部分
  - context window / compaction / request assembly
- `session-runtime` 不负责：
  - 全局 plugin discovery
  - 全局 surface assembler
  - provider factory
  - 业务审批规则
  - 全局治理策略

`session-runtime` 的长期内部建模方向也应保持稳定：

1. **优先 Event Log，再做投影**
   - 长期目标是把 session 视为 append-only event log，而不是一组可变字段。
   - `SessionState`、`history view`、`context window`、`branch snapshot` 都应是 log 的投影结果。
   - 当前项目已经部分符合这个方向：`EventStore.append/replay`、`SessionState` 内部 projector、`history/replay` 查询都建立在事件回放之上。
   - 但长期仍应继续收口：减少“字段即真相”的写法，让 `SessionLog -> Projection` 成为更显式的建模方式。

2. **Turn 优先建模为状态机，而不是散落的过程分支**
   - 合法状态转换应尽可能体现在类型或清晰枚举上，例如 pending / running / waiting_tool / completed / interrupted。
   - 当前项目的 `run_turn` 已经把 LLM、tool、compaction 拆成 step loop，但 turn 本身仍更接近过程编排，而不是显式状态机。
   - 长期方向不是把所有逻辑塞进一个巨大 enum，而是把“哪些转换合法”从运行时 if/guard 逐步提升为结构化状态转换。

3. **Context Window 优先建模为 budget 分配问题**
   - compaction、prune、file recovery、auto-continue 的共同问题不是“字符串怎么拼”，而是“预算如何分配”。
   - 当前项目已经有 `TokenUsageTracker`、`PromptTokenSnapshot`、`ContextWindowSettings`，说明方向是对的。
   - 长期应继续推进到更显式的 `ContextBudget + Strategy` 模型，使：
     - request assembly 可独立测试
     - compaction 策略可替换
     - budget 决策与 prompt 拼装解耦

4. **Mailbox / child delivery 优先使用类型化消息契约**
   - child delivery、interrupt、observe、wake、close 这些能力，长期应尽量通过明确的消息类型表达，而不是靠共享可变状态和隐式约定拼接。
   - 当前项目已经有 durable mailbox 事件和投影，但 `SessionActor` 还不是一个以 typed channel 驱动的真正 actor loop。
   - 如果后续 child delivery / subrun 控制继续增长，typed mailbox 是优先级很高的收口方向。

5. **Query 与 Command 逻辑继续分离**
   - 写侧负责追加事件、推进 turn、驱动 mailbox。
   - 读侧负责 history、snapshot、replay、context view 等投影查询。
   - 当前项目已经开始分离：有 `query/` 模块，也有只读快照类型。
   - 但 `SessionRuntime` 仍同时暴露较多 command/query 方法；长期可以继续向轻量 CQRS 靠拢，让只读查询尽量不被写侧执行路径阻塞。

当前 `session-runtime` 子域的 allowed / forbidden responsibilities 进一步固定为：

- `context`
  - allowed：上下文来源、继承链、解析结果、结构化快照
  - forbidden：token 裁剪、预算决策、最终 request assembly
- `context_window`
  - allowed：预算、裁剪、压缩、窗口化消息序列
  - forbidden：最终 request assembly、profile/context 来源解析
- `turn/request`
  - allowed：最终 prompt/request 组装、prompt metadata、与 window/compaction 的编排接缝
  - forbidden：上下文来源解析、跨 session 编排
- `actor`
  - allowed：live truth、推进执行所需状态、writer 桥接
  - forbidden：observe 快照投影、外部订阅协议映射
- `observe`
  - allowed：replay/live 订阅语义、scope/filter、状态来源整合
  - forbidden：同步快照投影算法、turn 推进、副作用
- `query`
  - allowed：拉取、快照、投影、durable replay 后的只读恢复
  - forbidden：推进、副作用、长时间持有运行态协调逻辑
- `factory`
  - allowed：执行输入或执行对象构造
  - forbidden：策略决策、校验、状态读写、业务权限判断

`application` 在这个边界上继续保持：

- allowed：参数校验、权限检查、错误归类、跨 session 编排、稳定 `SessionRuntime` API 调用
- forbidden：单 session 终态投影细节、durable append 细节、observe 快照拼装细节

### 4.6 `adapter-*`

- `adapter-*` 只实现端口，不持有业务真相。
- `adapter-*` 可以提供：
  - 具体 provider
  - 存储实现
  - 传输桥接
  - 配置读写
  - 能力物化辅助
- `adapter-*` 不能决定：
  - 会话真相
  - 审批策略
  - 业务权限
  - 用例编排

### 4.7 `plugin`

- `plugin` crate 承载的是插件模型与宿主侧基础设施，不是新的业务层。
- `plugin` 的长期职责是：
  - 描述插件包、插件能力、插件生命周期状态
  - 为 `server/bootstrap` 提供可装载、可监督、可物化的插件输入
- `plugin` 不能直接注入业务真相，也不能绕过组合根直接修改 `kernel` 或 `session-runtime`。
- 插件能力的注入路径必须固定为：
  - `plugin` / `adapter-*` 发现与装载
  - `server/bootstrap` 物化与汇总
  - `application` 编排 reload
  - `kernel` 原子替换 capability surface
- 插件生命周期至少要有明确状态：
  - discovered
  - loaded
  - failed
  - disabled
- 若插件能力不能通过统一 capability surface 表达，则默认视为边界未收敛，而不是允许额外开旁路。

## 5. 关键不变量

- `CapabilitySpec` 是运行时内部唯一能力语义模型。
- HTTP 状态码映射只在 `server` 层发生。
- `SessionActor` 不直接持有 provider；统一经由 `kernel` gateway 或已解析句柄。
- `application::App` 不保存 session shadow state；session 列表、history、replay、turn 推进都由 `session-runtime` 提供。
- 公共 API 不暴露内部并发容器（`DashMap`、`RwLock`、`Mutex` 等）。
- reload 的完成条件不是“内部 manager 变了”，而是“整份 capability surface 已被一致替换”。
- builtin、MCP、plugin 三类能力必须统一并入同一 capability surface。
- 插件、MCP、skills、tools 的“发现能力”只能依赖当前事实源，不能再长出平行 registry。

## 6. 组合根

唯一业务组合根入口位于：

- `crates/server/src/bootstrap/`

其中：

- `runtime.rs` 是组合根入口
- 其他 `bootstrap/*` 模块只负责装配细节，不构成新的业务层

组合根负责显式装配：

1. adapter 实现
2. `Kernel`
3. `SessionRuntime`
4. `App`
5. `AppGovernance`

组合根允许：

- 选择具体实现
- 连接依赖
- 把 builtin / MCP / plugin 等能力来源汇总成统一输入

组合根禁止长期承载：

- 业务逻辑
- 会话真相
- 大量临时 stub/noop 的本体
- 全局治理算法
- 新的 runtime façade

`main.rs` 仅保留启动、路由挂载与优雅关闭。

## 7. 治理与重载

- `AppGovernance` 只依赖治理端口，不直接绑定某个旧 runtime 协调器实现。
- reload 是 `application` 的治理行为，不是任意 manager 的内部副作用。
- reload 必须最终驱动：
  - 配置重新解析
  - plugin / MCP / builtin 能力重新收集
  - `kernel` capability surface 原子替换
  - 治理快照更新
- 当存在运行中的 session 时，治理层必须拒绝 reload，而不是尝试在执行中途切换能力面。
- server 侧的 `/api/config/reload` 只是治理入口的 HTTP 映射，不允许退化回“仅重读 config store”的旁路接口。

如果某次“重载”没有让上述链路闭合，那么它不是完整实现。

这里还需要一个独立 ADR 明确 reload 的失败语义，至少回答：

- 原子性边界是什么
- 哪一步失败会中止整次 reload
- 哪些步骤允许保留旧状态继续服务
- capability surface 替换失败时如何避免“半刷新”
- 治理快照如何表达成功、失败与部分可用状态

在 ADR 落地之前，任何 reload 实现都应优先选择“显式失败并保留旧状态”，而不是静默接受半更新。

## 7.1 当前落地语义

当前仓库的治理 reload 已经按以下顺序闭合：

1. `application::AppGovernance` 先检查是否存在运行中的 session。
2. 组合根重读磁盘配置，并重新解析 MCP 声明配置。
3. 重新发现 plugin、重建 base skill 列表，并同步外部 capability invoker。
4. `kernel` capability surface 与治理快照一起刷新。
5. 若 capability surface 替换失败，则保留旧 surface 与旧治理快照继续服务。

这意味着“reload 成功”的定义不再是某个 manager 内部状态变化，而是整份外部能力事实源已经一致替换。

## 7.2 观测与执行控制

- 观测快照由 `application::RuntimeObservabilityCollector` 统一采集，并接入治理状态输出。
- `session-runtime` 负责记录 session rehydrate、SSE catch-up、turn 执行与手动 compact 延迟落地。
- `application` 负责校验 `tokenBudget`、`maxSteps`、`manualCompact` 等执行控制输入，再把结果下沉到单次执行上下文。
- 忙碌 session 的手动 compact 以服务端登记为准，前端只展示接受/延迟执行反馈，不维护平行真相。

## 8. Discovery / Skill / Plugin 的长期归位

- 工具发现、技能发现、Skill Tool 等能力是否保留，首先看产品价值，其次才看旧项目是否存在。
- 若保留：
  - 工具发现必须依赖 `capability surface`
  - 语义字段必须依赖 `capability semantic model`
  - 技能发现必须依赖 `skill catalog / materializer`
- 若不再需要，应明确废弃并删除；不允许保留空壳接口或兼容性 skeleton。

## 9. 命名约定

- 实现层统一使用 `adapter-*` 前缀。
- 旧 `runtime*` 族 crate 已从 workspace 移除，且不应回归。
- 业务入口统一使用 `App` / `AppGovernance`。
- 不再暴露 `RuntimeService`、`RuntimeFacade` 一类“看似方便、实则重新中心化”的命名。

## 10. 阅读路径建议

阅读代码时建议按以下顺序：

1. `core`：先理解语义与契约
2. `kernel`：理解全局控制与 capability surface
3. `session-runtime`：理解单会话执行真相
4. `application`：理解用例编排与治理
5. `server`：理解 HTTP/SSE 映射与组合根
6. `adapter-*`：最后再看具体实现

这样可以避免在历史实现细节中横跳，直接按稳定边界理解系统。

## 11. 两条长期防腐线

必须持续盯住两个最容易回潮的点：

1. `application` 不要长成新的 RuntimeService  
   它必须承担治理与策略，但不能把所有执行细节重新吞进去。

2. `adapter-*` 不要偷渡业务真相  
   它们可以很强，但只能强在实现层，不能反向决定业务边界。

只要这两条线守住，系统就不会再次回到“到处都能调 runtime、谁都知道一点真相”的状态。
