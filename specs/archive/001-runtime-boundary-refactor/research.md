# Research: Runtime Boundary Refactor

## Decision 1: 内部 surface 采用 clean break，只有旧 durable 历史保留读侧降级

**Decision**  
本次重构默认不为旧的 Rust façade、旧的 protocol DTO 或旧的前端类型保留兼容层；server 与 frontend 在同一分支同步升级。唯一保留的兼容义务是：新代码仍能读取旧 durable 历史，但必须把 lineage 缺失明确暴露出来。

**Rationale**  
当前维护成本来自职责重叠和双轨 surface，而不是来自外部生态兼容。继续保留 alias、适配器或双写层只会把本次边界重构再拖成长期模糊状态。旧 durable 历史仍要能读，是因为规格已经明确仓库和运行环境里存在既有 session 数据。

**Alternatives considered**

- 保留旧 runtime API 并加新 API：会让 `session_service.rs` / `execution_service.rs` 与新边界长期并存，违背本次重构目标。
- 同时保留旧 protocol 字段和新 protocol 字段：会把前后端再次拖回“双轨语义”。
- 完全不读旧 durable 历史：与规格中的 legacy 历史要求冲突。

## Decision 2: 第一刀先改 subrun durable 协议，而不是先改 façade

**Decision**  
第一阶段先为 `SubRunStarted` / `SubRunFinished` 引入显式 `SubRunDescriptor` 和 `tool_call_id`，先把 durable lineage 真相补齐，再做查询层与边界迁移。

**Rationale**  
没有稳定 durable truth，后面的 status 查询、scope 过滤、frontend subrun tree、boundary owner 讨论都会继续围绕推断逻辑打转。协议层是所有后续收敛的前提。

**Alternatives considered**

- 先删旧 façade 再补事件字段：会让删除动作建立在不完整事实源之上，评审无法判断 blast radius。
- 先改 frontend tree 或 server filter：这些都是派生层，如果 durable 事实不补齐，只会把启发式挪地方。

## Decision 3: 生命周期事件只持有 durable lineage；查询层统一构建 lineage index

**Decision**  
不把 parent/depth/tool-call 信息复制到每一条普通事件上，而是只在 `subRunStarted` / `subRunFinished` 生命周期事件中存储 durable lineage 与 trigger link；server filter、status query、frontend subrun tree 统一从生命周期事件构建 `ExecutionLineageIndex`。

**Rationale**  
普通消息、工具输出和 delta 事件只需要知道自己属于哪个 `sub_run_id`。父子树和 trigger 来源属于生命周期事实，不需要复制到所有事件上。集中从 lifecycle 事件构建 lineage index，可以消除 turn-owner 这类顺序启发式。

**Alternatives considered**

- 把 parent/depth/tool-call 加到所有事件：字段扩散过大，而且把 lineage 和普通消息载荷耦合在一起。
- 保留当前基于 `parent_turn_id` + turn owner map 的推断：事实层与投影层继续混在一起，无法保证多路径一致。

## Decision 4: `runtime-execution` 不直接依赖 `runtime-session` crate；共享契约上提到 `core`

**Decision**  
最终依赖方向保持为 `core` 在上、各个 `runtime-*` boundary 并列、`runtime` 门面在最上层装配。`runtime-execution` 只依赖 `core` 中的共享类型与 trait，不直接编译依赖 `runtime-session`、`runtime-agent-control`、`runtime-agent-loop`。

**Rationale**  
如果 `runtime-execution` 直接依赖 `runtime-session`，再加上 `runtime-session` 目前已经依赖 `runtime-agent-loop` / `runtime-agent-control`，边界很容易重新形成反向渗透。把共享协作契约上提到 `core`，让 `runtime` 门面做装配，才能维持清晰的单向依赖。

**Alternatives considered**

- 允许 `runtime-execution -> runtime-session`：实现更快，但会破坏当前仓库既定的 crate 分层图。
- 把所有执行逻辑都塞回 `runtime` 门面：会让 façade 再次退化成第二套实现层。

## Decision 5: subrun status 采用 durable-first + live-overlay 查询策略

**Decision**  
subrun status 查询的历史真相以 durable lifecycle event 为主；仅当 live registry 中存在运行态 handle 时，才叠加 live 的 running/cancel 状态与最新 step/token 计数。

**Rationale**  
live registry 是运行态控制平面，不适合解释过去。durable-first 可以保证重启后、回放后、filtered replay 后仍然看到同一份 lineage；live-overlay 则保留 running 阶段的即时性。

**Alternatives considered**

- 继续 live-first 并把 durable 只当 fallback：会让历史解释依赖进程内状态，违背 Durable Truth First。
- 完全不用 live registry：会损失 running subrun 的及时状态与 cancel 能力。

## Decision 6: 根执行必须显式带 `working_dir`，会话执行从 session 元数据继承

**Decision**  
`POST /api/v1/agents/{id}/execute` 的 `working_dir` 变成必填；已有 session 上的 prompt / subrun 继续从 session 元数据或 parent execution context 继承工作目录。

**Rationale**  
根执行是 resolver 的起点。如果允许它静默退回进程 cwd，就会让“同一个 agent id 在不同项目下解析到不同定义”的行为失去可解释性。会话执行已经天然绑定了 session working dir，不需要再额外传参。

**Alternatives considered**

- 继续缺省到 `std::env::current_dir()`：最方便，但把解析语义绑在进程启动环境上，和 feature 目标冲突。
- 每次请求都让前端重复传 working_dir，包括 session prompt：没有必要，会增加调用面噪声。

## Decision 7: legacy 历史不再伪造 ancestry 结果；缺失 lineage 时显式降级

**Decision**  
旧历史若缺少 `SubRunDescriptor`，status 返回部分可用字段并标记 `source=legacyDurable`；`scope=directChildren` / `scope=subtree` 这类依赖 ancestry 的查询直接返回显式错误，而不是继续根据事件顺序猜。

**Rationale**  
legacy 数据不完整时，最危险的不是“返回少”，而是“返回看起来完整但其实是猜的”。本次重构必须把“事实缺失”和“当前没有值”区分开。

**Alternatives considered**

- 给 legacy 数据默认 `depth=1`、`parent_agent_id=None`：会把推断伪装成事实。
- 后台迁移脚本批量回填旧历史：成本高，且回填本身仍然依赖推断。

## Decision 8: façade 删除以 caller inventory 为边界，而不是靠编译报错清扫

**Decision**  
`session_service.rs`、`execution_service.rs`、`service/replay.rs` 以及它们对应的 wrapper 不做“先删再修”，而是在 `migration.md` 里先列出现有调用方、替代入口和删除前提，再按阶段移动调用方。

**Rationale**  
这两个 façade 已经被 server route、runtime wrapper 和内部 delete/cancel 流程间接引用。没有 caller inventory，删除动作只会把风险推迟到编译期，并且很难判断 blast radius 是否收敛。

**Alternatives considered**

- 直接删 façade 后按编译错误回填：可操作，但不满足本次文档门槛，也不利于大型重构并行协作。
- 长期保留 façade 作为 alias：删除动作会不断后延，owner 关系也不会真正收敛。

