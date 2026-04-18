## Context

当前仓库已经完成大部分架构重构，新的稳定边界已经明确：

- `application` 是唯一业务入口与策略/治理边界
- `kernel` 是唯一全局控制面
- `session-runtime` 是唯一单 session 真相面
- `server/bootstrap` 是唯一组合根
- `adapter-*` 只实现端口，不承载业务真相

因此这份 change 的首要目标不再是“按旧 runtime 文件形态逐个复刻功能”，而是 **在不破坏现有架构的前提下，继续迁入旧仓剩余能力**。  
任何与当前边界冲突的旧设计，都必须让位于当前架构。

当前已完成的关键基础：

- `application::App` 已移除 session shadow state
- `session-runtime` 已接管 create/list/delete、history/view/replay、submit/interrupt/compact
- `core::EventStore` 与 `adapter-storage` 已能表达 session 创建、会话 meta 枚举、按项目清理
- `kernel::agent_tree` 已具备 inbox / observe / cancel propagation 等控制面基础

当前仍未完成的核心迁移：

- 根代理执行与子代理执行的完整业务入口
- token budget 驱动的 auto-continue 闭环
- plugin loader / hook / skill 到 capability surface 的完整接线
- subrun status 等 server 合同补齐

## Goals / Non-Goals

**Goals**

- 在保持当前分层不退化的前提下，继续迁入旧仓剩余 runtime 功能
- 完成单 session 执行真相、全局 agent 控制、执行用例入口、plugin 能力接线的闭环
- 让 change 文档真实反映当前实现，不再要求已经被架构否决的路径

**Non-Goals**

- 不为匹配旧目录结构而强行拆模块
- 不把 session 真相重新放回 `application`
- 不把 turn 构造、request assembly、loop 细节塞回 `kernel`
- 不引入新的 runtime façade 来模拟旧 `RuntimeService`
- 不做向后兼容适配

## Decisions

### D1: 单 session 真相只放 `session-runtime`

`create/load/list/delete`、`history/view/replay`、`interrupt/compact/run_turn`、context window、request assembly、actor 协调都属于“某个 session 当前发生了什么”，因此统一归 `session-runtime`。

这也意味着：

- `application` 不再维护 session `HashMap`
- 持久化 session 的创建/恢复/删除由 `session-runtime + EventStore` 处理

### D2: `application` 只保留用例编排与策略入口

`application` 继续负责：

- 参数校验
- 权限检查
- 审批/策略入口
- 配置解析
- 错误归类
- reload / governance 编排

`application` 不直接保存 session 真相，也不直接持有 adapter 实现。

### D3: `kernel` 只保留全局控制

`kernel` 承接：

- capability router / gateway / surface
- agent tree / subrun live control
- 全局事件协调

`kernel` 不负责：

- `build_agent_loop`
- request assembly
- 单 session turn 执行真相

### D4: `session-runtime` 内部继续按子域分块

为了避免“收口成功但 crate 内部重新长成一团”，`session-runtime` 内部保持以下分工：

- `state`
- `catalog`
- `actor`
- `turn`
- `context`
- `factory`
- `query`

迁移旧仓能力时按行为归位，不按旧 crate 文件机械复制。

### D5: plugin / hook / skill 只在组合根接线

plugin 发现、supervisor 生命周期、hook 适配、skill/capability 物化都应发生在 `server/bootstrap`，最终结果注入 `kernel` 的 capability surface。

这样做的原因是：

- `application` 不需要知道 plugin 细节
- `kernel` 只消费已经物化好的 capability/hook 句柄
- 不需要新的 runtime façade

### D6: 任务以“结果”而不是“文件名”定义

原始 tasks 过度绑定诸如 `mailbox.rs`、`surface.rs`、`connection.rs` 这类文件路径，已经与当前实现明显偏离。  
后续任务改为描述行为结果与边界要求，允许在符合架构的前提下采用不同文件组织。

## Risks / Trade-offs

**[Risk] 为了追旧仓功能而重新污染当前边界**  
通过“架构优先、迁移第二”的规则约束：任何功能迁移先判断归属，再决定实现。

**[Risk] `session-runtime` 收口后继续膨胀**  
通过内部 `state / actor / turn / context / query / factory` 分块控制复杂度，不允许把 policy / governance / plugin glue 混进去。

**[Risk] tasks 与实现持续漂移**  
通过把 tasks 改成行为导向，并在每轮实现后同步 change 文档，减少“代码已变、任务还停在旧假设”的回潮。

**[Trade-off] 不再追求旧 runtime 目录的 1:1 映射**  
牺牲了“对照旧文件容易搜”的便利，但换来了当前项目更稳定、更容易读懂的结构。
