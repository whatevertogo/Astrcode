## Context

我们已经完成了一轮把 `application` 中单 session 查询、durable append、终态投影等细节下沉到 `session-runtime` 的重构，但这只解决了“谁持有真相”的大问题，还没有彻底解决“`session-runtime` 内部怎么分块”的问题。

当前最明显的边界风险有三类：

1. `context` 与 `context_window` 容易重叠  
   `context` 已经偏向“本次执行可用上下文的来源、继承与解析结果”，但 `context_window` 内还存在 `request_assembler` 这类最终请求拼装职责。如果不及时收口，未来 request assembly、budget、compaction 会重新耦在一起。

2. `actor`、`observe`、`query` 容易相互渗透  
   目前大方向还守住了，但 `query` 已经承载了越来越多历史、agent 视图、mailbox 恢复、turn 结果投影。若不及早按读取场景分块，后续很容易把推进逻辑、副作用或观察协议也一起塞进去。

3. `factory` 名称过宽，长期最容易成为兜底杂项模块  
   现在它仍然比较薄，但如果没有明确约束，构造、校验、策略、状态读写都可能逐渐汇入这里。

这次设计的目标不是再改 crate 大边界，而是在不改变外部 API 的前提下，给 `session-runtime` 内部子域立清楚职责红线，并把 `application` 继续收成薄用例门面。

## Goals / Non-Goals

**Goals:**

- 让 `context`、`context_window`、`actor`、`observe`、`query`、`factory` 的职责边界变成可执行的代码组织规则，而不是口头约定。
- 把 `context_window` 中与最终 request assembly 深度绑定的实现迁到更中性的 request assembly 子域。
- 将 `query` 拆成按读取语义组织的子模块，至少覆盖 `history`、`agent`、`mailbox`、`turn`。
- 继续把 `application` 中残余的单 session 真相细节下沉，让它只保留用例校验、权限/所有权检查和跨 session 编排。
- 同步更新 `PROJECT_ARCHITECTURE.md` 与 OpenSpec，使后续扩展有统一的放置规则。

**Non-Goals:**

- 不改 `application`、`kernel`、`session-runtime` 的 crate 级依赖关系。
- 不修改 HTTP/SSE 协议、Tauri 启动方式或前端消费模型。
- 不把 `watch`、`config`、`mcp` 迁入 `session-runtime`。
- 不把跨 session 的 `wake` 主编排和父子协作协调硬塞进 `session-runtime`。
- 不为旧内部模块路径提供兼容层。

## Decisions

### 决策 1：把 `context` 和 `context_window` 切成“来源解析”与“预算窗口”两层

`context` 只保留：

- 上下文来源解析
- 继承关系与覆盖规则
- 结构化解析结果，例如 `ResolvedContextSnapshot`

`context_window` 只保留：

- token 预算
- 裁剪、压缩、窗口化
- 产出窗口化后的消息序列

最终 request assembly 不再长期挂在 `context_window`，而是迁到更中性的 request assembly 子域，例如 `turn/request` 或 `prompt_assembly`。

之所以这样切，是因为 budget 决策和输入组装虽然相邻，但回答的是两个不同问题：

- “有哪些输入可用” 是 `context`
- “这些输入在预算内怎么组织” 是 `context_window`
- “最终如何形成执行请求” 是 request assembly

备选方案：

- 保持 `request_assembler` 继续放在 `context_window`  
  放弃原因：会让 `context_window` 同时负责预算与最终执行请求，不利于后续 compaction 策略和 request 构造独立测试。

### 决策 2：固定 `actor / observe / query` 三个动词的语义

- `actor` 只负责推进与持有单 session live truth
- `observe` 只负责推送/订阅、scope/filter、replay/live receiver、状态源整合
- `query` 只负责拉取、快照与投影

`query` 可以读取 durable event 和 projected state，但不得承担：

- 推进执行
- 追加副作用
- 长时间持有运行态协调逻辑

备选方案：

- 继续把只读与轻副作用混放在 `query`  
  放弃原因：会让 query 逐渐演化成“什么都能查一点、什么都能顺手做一点”的大模块，后续很难审计哪些逻辑是纯投影、哪些逻辑已经有副作用。

### 决策 3：将 `query` 按读取语义拆成子模块

`query` 拆成至少四类：

- `history.rs`：history / replay / 视图快照
- `agent.rs`：agent observe snapshot、child/open session 视图
- `mailbox.rs`：recoverable delivery、pending mailbox 投影
- `turn.rs`：turn terminal snapshot、turn outcome 投影

crate 根上的 `query/mod.rs` 只做类型 re-export 和稳定入口聚合，不再同时承载全部实现。

备选方案：

- 保留单文件 `query/mod.rs`，仅靠注释约束  
  放弃原因：约束力太弱，后续读写与测试仍会持续堆进去。

### 决策 4：`factory` 保持超薄，并用规则阻止膨胀

`factory` 只允许做两类事：

- 构造执行输入
- 构造执行对象

不允许进入 `factory` 的内容：

- 策略决策
- 校验
- 状态读写
- 业务权限判断

这次不强行新增更多工厂抽象，只在 design/spec 中把禁区写清楚，并在实现上把越界职责直接移出。

备选方案：

- 把 `factory` 改名为更具体的模块  
  暂不采用：当前模块仍然较薄，先立规则比现在改名收益更高。

### 决策 5：`application` 继续只保留薄门面与跨 session 编排

`application` 保留：

- 参数校验
- 权限与所有权检查
- 错误归类
- 根执行与子执行入口
- 跨 session 协调，例如 child terminal handoff 与 parent wake

`application` 不再保留：

- 单 session 终态判定细节
- durable mailbox append 细节
- 单 session observe 投影拼装
- recoverable delivery 重放细节

这些都通过 `SessionRuntime` 的稳定 query/command API 获取。

备选方案：

- 让 `application/agent` 继续保留这类细节，只靠目录名区分  
  放弃原因：这会重新把 `application` 长成执行细节杂货铺，与 `PROJECT_ARCHITECTURE.md` 的长期边界冲突。

## Risks / Trade-offs

- [Risk] `query` 拆模块后，调用路径变长，短期阅读成本上升  
  → Mitigation：在 `query/mod.rs` 保留统一 re-export，并为每个子模块补简短模块注释。

- [Risk] request assembly 从 `context_window` 迁出时，容易引入消息顺序或预算行为回归  
  → Mitigation：先保持外部行为不变，只迁职责归属，并用现有 turn/context window 测试回归。

- [Risk] 继续下沉 `application` 细节时，可能误把跨 session 编排也下沉进 `session-runtime`  
  → Mitigation：所有需要同时协调 parent session、child session、kernel delivery queue 的逻辑一律留在 `application`。

- [Risk] `factory` 规则只写文档不写代码，可能再次失效  
  → Mitigation：同步把明显越界逻辑移走，并在 review/新 change 中按 spec 检查新增代码的落点。

- [Risk] 这轮重构主要是内部边界收口，用户感知弱，容易被后续功能迭代再次冲淡  
  → Mitigation：把边界要求落到 OpenSpec 和架构文档里，作为后续变更的强约束。

## Migration Plan

1. 为 `session-runtime` 增加或收口稳定 query/command API，先保证 `application` 有替代调用面。
2. 从 `context_window` 迁出 request assembly 相关实现，保留原有测试覆盖。
3. 将 `query/mod.rs` 拆成 `history`、`agent`、`mailbox`、`turn` 子模块，并保持 crate 外部调用接口稳定。
4. 清理 `application` 中残余的单 session 查询、终态投影和 durable append 细节。
5. 更新 `PROJECT_ARCHITECTURE.md` 与相关模块注释，让边界规则对后续开发可见。

回滚策略：

- 若中途发现某块 request assembly 或 query 拆分引起回归，可先恢复旧模块组织，但保留新建的稳定 API 和测试；
- 不回滚外部协议与持久化格式，因此回滚成本主要限于 Rust 模块布局和内部调用链。

## Open Questions

- request assembly 最终应落在 `turn/request` 还是独立 `prompt_assembly` 子域？这次先用更中性的子域落位，但命名可以在实现时根据现有 turn 模块组织再确定。
- `observe` 是否需要保留少量只读聚合类型，还是完全只保留推送/订阅管线？当前倾向于允许保留协议无关的 observe 类型，但不允许投影算法继续长进去。
- `factory` 是否未来需要进一步拆成 `input_factory` 与 `lease_factory`？这次暂不拆，等出现第二类明确构造对象时再决定。
