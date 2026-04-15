## Context

Astrcode 当前的正式产品面只有桌面端和浏览器端，terminal 用户没有一等入口。现有 server 已经承担唯一组合根和唯一业务真相，前端依赖 HTTP / SSE 与一套较重的本地事件投影逻辑来构建聊天 transcript、thinking、tool block、subrun 导航和 slash command 体验。  

这次变更不是“再做一个能看消息的 CLI”，而是要做一个可正式发布的终端版 Astrcode，满足以下现实约束：

- 终端端必须依赖现有 `astrcode-server`，不能直接嵌入 `application`、`kernel` 或 `session-runtime`
- `server is the truth` 不能被打破，terminal 只是新的 client surface
- 终端端必须覆盖完整聊天工作流，而不是 debug-only 观测台
- thinking、tool streaming、child agent / subagent、会话切换与 slash command 都需要稳定显示
- 不能简单复制现有 React reducer，否则会形成第二套私有 transcript 投影真相

当前最核心的设计问题不是“ratatui 怎么画”，而是“终端端应该消费什么稳定合同，才能既保留全部能力，又不把前端私有实现复制到 Rust terminal 客户端里”。

## Goals / Non-Goals

**Goals:**

- 新增可复用的 typed client facade，避免终端端直接拼 HTTP / SSE 细节
- 新增 launcher 层，隔离本地发现、spawn、ready handshake 与 client 连接职责
- 新增正式发布的终端客户端 crate，作为桌面端、浏览器端之外的第三个一等 client
- 保持 `server` 作为唯一组合根与唯一业务事实源
- 为终端端提供完整聊天工作流，包括：
  - prompt 提交与流式回复
  - thinking 展示
  - tool call / stdout / stderr 展示
  - child agent / subagent 展示与导航
  - `/skill`、`/compact`、`/new`、`/resume` 等 slash command 体验
  - session 创建、恢复、切换
- 新增终端友好的稳定 read model / delta 契约，避免终端端复刻图形前端 reducer
- 为终端 surface 明确版本化、错误分类、重连恢复与测试策略
- 保持现有事件日志、历史回放、SSE catch-up 与执行控制语义继续由现有层负责

**Non-Goals:**

- 不替换现有桌面端或浏览器端
- 不让终端客户端直接依赖 `application` / `kernel` / `session-runtime` 内部实现
- 不引入新的 runtime façade 或第二套组合根
- 不新增终端端专属 durable 存储或平行会话真相
- 不把终端端退化成 debug-only UI
- 不在本次变更中定义通用 approval / review 终端交互闭环；后端尚未提供稳定合同，当前只保留 TODO

## Decisions

### 决策 1：terminal 方案收敛为 `launcher / client / terminal_projection / server route / tui app` 五层

正式终端版不再笼统地说“新增 cli + client”，而是明确分成五层：

- `launcher`
  - binary 发现
  - `run.json` 读取
  - 本地 `astrcode-server` spawn
  - ready handshake
  - 子进程生命周期管理
- `client`
  - auth token 交换
  - typed HTTP API
  - terminal snapshot 拉取
  - terminal SSE stream/cursor catch-up
  - 结构化错误归一化
- `terminal_projection`
  - `TerminalFacts -> protocol::terminal::v1::*` 的纯投影构建
- `server route`
  - terminal v1 路由、auth、状态码、SSE framing
- `tui app`
  - ratatui / crossterm 事件循环、布局、输入、渲染与状态机

依赖方向固定为：

- `cli -> launcher + client`
- `client -> core + protocol`
- `server -> application + protocol + terminal_projection`

这里有一个关键边界：

- `launcher` 负责“拿到一个可连接的 origin/token”
- `client` 只负责“连接一个已经存在的 origin/token”

否则 managed-local 逻辑会把 `client` 重新做成“launcher + client”的混合物。

### 决策 2：`client` 保持纯 client，不承担 spawn、发现或本地宿主管理

`client` 不再抽象 managed-local transport 选择，更不内嵌 server 生命周期。它只暴露面向已存在 server 的 typed facade，例如：

- `exchange_auth`
- `list_sessions`
- `create_session`
- `fetch_terminal_snapshot`
- `stream_terminal`
- `submit_prompt`
- `request_compact`
- `list_slash_candidates`

`launcher` 负责：

- 显式 `--server-origin` / `--token`
- `~/.astrcode/run.json`
- 本地 spawn + ready 协议
- 失败重试与进程回收

这样做的原因是：

- 保持 `client` 纯粹，便于 mock 与多宿主复用
- managed-local 只是终端启动策略，不是 client 语义的一部分
- 未来 IDE 插件或自动化 harness 可直接复用 `client`，无需继承本地 spawn 逻辑

备选方案：

- 把 managed-local、loopback HTTP、future in-process transport 都塞进 `client`
  - 放弃原因：职责会混成 launcher + client，边界不再可测试

### 决策 3：Mutation 继续复用现有业务 API，但统一收口到纯 `client` facade

终端版不会新造一套 mutation 语义，而是继续复用现有稳定入口：

- 认证：`/__astrcode__/run-info` + `/api/auth/exchange`
- session create / delete / list
- prompt submit
- interrupt / compact
- discovery / composer options

但这些入口不再由 `cli` 直接拼接，而是由 `client` 暴露 typed 方法，例如：

- `create_session`
- `list_sessions`
- `submit_prompt`
- `request_compact`
- `subscribe_terminal`
- `list_slash_candidates`

`client` 的 transport 在 v1 只有一条：

- HTTP request/response
- SSE stream

本次不引入 WebSocket，也不把旧 `/api/sessions/{id}/events` 复用成 terminal stream。

### 决策 4：terminal surface 从第一天起使用单一版本轴

新增 terminal surface 不沿用“先上 unversioned 路径，后续再补版本”的策略，而是直接采用 path version：

- snapshot：`GET /api/v1/terminal/sessions/{id}/snapshot`
- stream：`GET /api/v1/terminal/sessions/{id}/stream?cursor=...`
- `protocol` 中新增对应版本命名空间，例如 `http::terminal::v1`

兼容性规则明确为：

- `v1` 内仅允许加字段、加可选值、加新的非破坏性 delta kind
- 改字段语义、改必填性、改 cursor/rehydrate 行为等破坏性变更 MUST 升到 `v2`
- CLI 只绑定一个明确路径版本，不做运行时猜测

本次变更不强行给全仓库所有旧 `/api/*` 路由补齐统一版本迁移，但 terminal surface 必须从起点就可演进。

terminal v1 不再叠加第二套 surface 内部版本号：

- terminal DTO 内不再额外放 `PROTOCOL_VERSION`
- cursor 保持 opaque
- 旧事件流里的 `PROTOCOL_VERSION=1` 继续只服务 legacy event surface

协议冻结策略优先采用仓库现有模式：

- `crates/protocol/tests/fixtures` 固定 JSON 形状
- conformance tests 冻结序列化/反序列化行为

是否在本次顺带引入 `schemars` / TypeScript 导出，不作为硬前置；如果 terminal surface 很快要被多语言 client 消费，再单独补 schema export。

### 决策 5：projection builder 不放 `protocol`，而是放在 server 边界侧

终端 transcript / child pane 的拼装不再笼统地说“都放在 `application`”，也不放进 `protocol`。职责拆分为：

- `session-runtime`
  - 持有 durable event log、history、replay、observe、child lineage 等真相
- `application`
  - 负责权限、参数、cursor 合法性、hydration / catch-up 编排
  - 返回 surface-neutral 的 `TerminalFacts`
- `terminal_projection`
  - 把 `TerminalFacts` 映射成 `protocol::terminal::v1::*`
  - 必须保持无状态、可测试，不夹带用例校验或运行时副作用
- `server`
  - 挂路由、做 auth、状态码映射、SSE framing，并调用 projection builder

这样做的原因是：

- `application` 是唯一用例入口，但不应该堆满 surface-specific transcript 拼装细节
- `protocol` 只定义 DTO，不承载 runtime knowledge 或投影逻辑
- builder 不直接吃 `session-runtime` 内部类型，避免边界倒灌
- CLI 必须消费 authoritative block，而不是自己再复制一份 GUI reducer

这里的关键约束是：

- `protocol` 负责定义 DTO / block 结构
- `application` 不依赖 `protocol`
- `terminal_projection` 可以依赖 `protocol`
- builder 最好作为 `server` 内部模块或仅被 `server` 依赖的内部 crate，而不是 `protocol`

### 决策 6：terminal v1 采用“snapshot + SSE stream”合同，不复用旧 `/events`

terminal v1 的 authoritative read surface 明确为：

- `snapshot`
  - terminal 的 authoritative hydration
- `stream`
  - terminal 专属 delta 流，使用 SSE + opaque cursor

旧 surface 继续保留原职责：

- `/api/sessions/{id}/view`
  - 旧 surface / 兼容用途
- `/api/sessions/{id}/history`
  - 导出、诊断、历史查看，不是 terminal hydration 来源
- `/api/sessions/{id}/events`
  - legacy event surface，不等于 terminal stream

v1 不上 WebSocket。等 future approval / review 真需要 server→client 双向请求时，再单开 `/ws` 或 `v2`。

### 决策 7：Transcript 使用“结构化块 + 增量更新 + 可恢复 cursor”模型

终端端的核心视图不是“原始事件流”，而是终端可直接渲染的 transcript block。block 至少覆盖：

- user message
- assistant message
- thinking
- tool call（含 running / completed / failed）
- tool stream（stdout / stderr）
- turn-scoped error
- compact / system note
- child agent / subagent handoff

terminal delta 不是任意 JSON patch，而是有限、typed 的更新操作，例如：

- hydrate snapshot
- append block
- patch block
- complete block
- reset / rehydrate required

每条可恢复增量都必须绑定稳定 cursor / sequence。客户端恢复时只根据 cursor 继续，不重放本地推理规则。

错误呈现边界也在 block 模型里显式固定：

- 进入 transcript block 的错误
  - provider error
  - context window exceeded
  - tool fatal
  - 当前 turn 的 rate limit
- 不进入 transcript、改走 banner/status 的错误
  - `auth_expired`
  - `cursor_expired`
  - `stream_disconnected`

持久化策略保持不变：

- 不为 terminal transcript 新增 durable store
- terminal transcript 从现有 event log / replay / live stream 派生

### 决策 8：Slash command 是本地 UX 壳，但命令语义与候选事实必须来自服务端

terminal 端需要 slash palette，但 slash command 只是一层输入 UX，不是新的业务协议。命令语义拆分如下：

- `/new`
  - 通过现有 session create 入口创建会话并切换焦点
- `/resume`
  - 基于 session list / search 切换到已有 session
- `/compact`
  - 通过既有 execution controls / compact 入口提交
- `/skill`
  - 基于 discovery / composer options 获取候选，插入或触发正式输入流程

终端 palette 必须由服务端返回：

- command / skill 候选
- 标题、描述、关键字
- 插入文本或动作类型

CLI 只负责：

- 输入解析
- 候选过滤与选择
- 键盘导航与展示

CLI 不负责维护一份与 GUI 平行的命令注册表。

### 决策 9：连接、重连与错误恢复使用统一的结构化客户端合同

正式终端版需要兼顾：

- 本地单机体验
- 已运行 server 的附着体验
- 远程或自定义 server origin 的高级用法

连接优先级保持为：

1. 显式 `--server-origin` / `--token`
2. 读取 `~/.astrcode/run.json`
3. 无可附着 server 时由 `launcher` 进入 managed-local

但恢复协议进一步明确：

- `client` 持有最后一个 durable cursor / event id
- 断线重连时通过 `cursor` 查询参数请求 catch-up；SSE `id` 仅作为流内回执，不再构成第二套版本轴
- 若 server 判定 cursor 已失效，返回结构化 `rehydrate_required`
- 若 token 过期，返回结构化 `auth_expired`

错误类型不直接把 HTTP 状态码或 `reqwest::Error` 暴露给 TUI，而是收口为结构化客户端错误，例如：

- auth / permission
- validation
- not_found / conflict
- stream_disconnected
- cursor_expired
- transport_unavailable

backpressure 处理策略也收口到 `client`：

- UI 和网络之间使用 bounded channel
- 若消费者落后，`client` 产出明确的 lagged / rehydrate-required 语义
- 不允许通过无限缓存把内存压力转嫁给 CLI 进程

### 决策 10：子智能体展示采用“主 transcript + side pane / focus view”双层模型

终端空间有限，但子智能体又是核心能力，因此不采用“把所有 child transcript 平铺进主时间线”的做法。终端版采用双层展示：

- 主 transcript
  - 展示 root conversation 与关键 child handoff / child terminal result
- child pane / focus view
  - 展示 direct child 列表、状态、最近输出、pending message、当前责任分支
  - 允许用户切换 focus 到某个 child / subrun

这部分 read model 必须保持所有权边界：

- 只展示当前 session 有权观察到的 child
- 直接父子关系和 subrun lineage 仍以现有 control / session truth 为准

### 决策 11：`crates/cli` 从一开始按模块边界拆开，避免长成单文件 TUI 巨石

CLI 内部至少按一级模块拆分为：

- `app/`
  - 主事件循环、tick、action dispatch
- `launcher/`
  - server 发现、`run.json`、spawn、ready、生命周期
- `state/`
  - transcript、焦点、pane、选择态、滚动位置
- `command/`
  - slash command 解析、动作路由、快捷键映射
- `ui/`
  - 组件、pane、widget
- `render/`
  - 布局与 buffer 渲染
- `capability/`
  - terminal feature detection 与 graceful degradation
- `test_support/`
  - fake client、fixture loader、render harness

`main.rs` 只负责组装，不承载业务状态机。

### 决策 12：TUI v1 先约束流与渲染复杂度，而不是过早支持所有高级模式

v1 的终端运行策略明确收口为：

- 只维护一个 active session 的 live stream
  - `/resume`、session list 与 snapshot 查询不等于并行 live subscribe
- resize 是一等事件
  - line-wrap cache 失效
  - scroll anchor 重算
  - child pane 布局重排
- stream chunking 采用两档策略
  - `Smooth`
  - `CatchUp`
  - 由队列深度与最老 chunk age 驱动切换
- terminal capability detection 必须集中处理
  - truecolor
  - unicode width
  - alt-screen
  - mouse
  - bracketed paste
  - 并允许退化到 ASCII + no-color

## TODO / Deferred

### TODO：Approval / Review 的终端适配暂缓

当前后端还没有通用的 approval request / resolve terminal contract；现有仓库只有 MCP 配置审批等局部能力，还不足以定义正式的聊天期审批交互。因此本次 design 明确：

- 不设计通用 approval / review 的终端 UX 流程
- 不把 approval prompt、approve / reject 键位、审批 delta 纳入本次 terminal v1 合同
- 只在 `client` / `cli` 模块边界上保留扩展位，等后端出现稳定合同后再开后续 change

## Risks / Trade-offs

- [Risk] 新增 `client` 层后，可能又包一层“镜像 DTO”，形成新的重复模型  
  Mitigation：`client` 优先复用 / re-export `protocol` 类型，不再造平行数据结构。

- [Risk] `launcher` / `client` 若边界失守，会重新长成一个“既会 spawn 又会 transport”的混合层  
  Mitigation：严格约束 `launcher` 只返回 origin/token，`client` 只消费 origin/token。

- [Risk] 新增 terminal read model 后，GUI 与 terminal surface 可能再次出现两套近似但不完全一致的视图语义  
  Mitigation：把 terminal 投影定义为正式 contract，并把 authoritative builder 固定在服务端边界侧；不要在 CLI 侧复制 GUI reducer。

- [Risk] block 级 transcript 投影如果做得过重，可能增加 server hydration 成本  
  Mitigation：基于现有 replay / query 结果做派生，不新增 durable store；必要时增加 bounded cache，但 cache 不是事实源。

- [Risk] CLI 自动拉起本地 server 会带来进程生命周期和认证复杂度  
  Mitigation：优先 attach 现有 server，managed-local 复用现有 `LocalServerInfo` 与 bootstrap exchange，并把 transport 分支收口到 `client` 层。

- [Risk] terminal v1 从一开始就版本化，短期会让新旧 `/api/*` 风格并存  
  Mitigation：只对新 terminal surface 启用显式版本；不在本次顺手改写全仓库旧路由。

- [Risk] 旧 `/events` 与新 terminal `/stream` 并存，容易让实现者误复用 legacy 语义  
  Mitigation：在 design/spec 中明确 terminal snapshot/stream 是 authoritative surface，legacy `/view`/`history`/`events` 仅保留旧职责。

- [Risk] slash command UX 可能诱导在 CLI 本地偷偷堆业务逻辑  
  Mitigation：明确 slash palette 只是输入壳，所有语义都映射到既有 server 合同或新的 terminal read model。

- [Risk] 断线恢复和 backpressure 处理不当，会让终端状态漂移或吞掉增量  
  Mitigation：使用 structured error + lagged / rehydrate-required 语义，禁止 UI 靠猜测修复流状态。

- [Risk] thinking、tool streaming、child 状态同时出现时，终端布局容易失控  
  Mitigation：采用结构化 block + focus pane 模型，并允许折叠 / 截断 / 切换视图，而不是把所有细节平铺。

- [Risk] v1 如果一开始支持多会话并行 live subscribe，backpressure 与未读态会快速失控  
  Mitigation：v1 明确只维护一个 active session 的 live stream。

- [Risk] approval / review 暂未纳入 terminal v1，会留下一块产品缺口  
  Mitigation：在设计上显式标成 TODO，不假装已经闭环；等后端合同稳定后单独补 change。

## Testing Strategy

- `protocol`
  - 为 terminal v1 DTO、cursor、错误 envelope、新增 delta kind 增加 fixture + conformance tests
- `client`
  - 使用 mock transport 覆盖 auth、重连、cursor catch-up、lagged / rehydrate-required、结构化错误映射
- projection builder
  - 使用文本 fixture / snapshot 风格测试冻结 block 输出，避免 event -> block 映射漂移
- `cli`
  - 使用 ratatui test backend 做 buffer 级渲染断言，覆盖 transcript、child pane、slash palette、错误态与空态
- integration
  - 增加 server + client 集成测试，覆盖 hydration、delta、cursor 失效、token 过期与 managed-local attach

## Migration Plan

1. 在 `protocol` 中新增 `terminal::v1` DTO、cursor / error envelope 与 fixture/conformance 测试骨架。
2. 新增 `launcher` 边界，先收口 `run.json`、spawn、ready handshake 与 origin/token 解析。
3. 新增 `crates/client`，实现纯 typed API、auth、snapshot/stream 与 reconnect/backpressure 基础能力。
4. 在 `application` 中补齐 `TerminalFacts` 查询编排；在服务端边界侧落地 `terminal_projection` 与 `/api/v1/terminal/*` route。
5. 新增 `crates/cli`，按 `launcher / state / command / render / capability` 模块骨架实现 transcript 渲染主循环。
6. 补齐 slash command、tool streaming、thinking、child pane、single-active-stream、resize 与 degrade 策略。
7. 增加 `protocol` / `launcher` / `client` / projection / `cli` / integration 六层测试，并接入 release 构建。
8. 将终端版纳入正式 release artifact，但不影响桌面端与浏览器端；approval / review 终端适配保持 deferred TODO。

回滚策略：

- 若 terminal read model 或 CLI 在发布前不稳定，可以停止发布 `astrcode-cli` binary
- 若 terminal surface 路由已合入但客户端未发布，可保留未被主文档暴露的路由，不影响现有 GUI 合同
- 不对现有 GUI path、session 真相或 server 组合根做破坏式替换，因此回滚不需要迁移会话数据

## Open Questions

- managed-local 的首版是否只做 child-process + loopback HTTP，还是需要为后续本地 transport 预埋更明确的 launcher trait？
- `/resume` 的交互是否只做 session 列表模糊搜索，还是需要支持按 working directory / title / session id 多键搜索与最近使用排序？
- thinking 默认是始终展开、折叠显示，还是按模型类型 / token 量自动收起？
- 是否需要在第一版就支持进入 child focus transcript，还是先只展示 child 摘要与 terminal result？
- terminal v1 是否在本次 change 就顺带产出 schema export，还是先用 protocol fixture/conformance 冻结 JSON 形状？
