## Context

`kernel` 是全局控制面，应该承接 agent tree、能力面、全局事件协调，但不应该把内部数据结构直接泄漏给 `application` 或 `server`。当前迁移需要补齐的不是“更多内部模块”，而是“更稳定的控制合同”。

本 change 的目标是把旧项目里的 agent control 行为整理成两个边界清晰的接口族：

- 状态查询合同
- 控制与投递合同

## Design Decisions

### 1. `kernel` 暴露稳定控制合同，而不是内部树结构

`kernel` 负责：

- subrun status 查询
- observe 流订阅入口
- child delivery / mailbox 路由
- wake / close 控制入口

但这些入口都必须以稳定类型和稳定方法存在，调用方不允许直接依赖 `agent_tree` 内部节点表示、内部索引结构或内部事件分发细节。

### 2. `session-runtime` 继续只承接单 session 真相

`session-runtime` 负责：

- 某个 session 当前的状态与事件回放
- 单次 turn 的推进
- interrupt / compact / replay / branch

它不负责全局 subrun 控制合同，也不拥有全局 agent tree 的对外 API。

### 3. `application` 只编排控制请求

`application` 负责：

- 参数校验
- 身份与权限检查
- 错误归类
- 将 server 请求转成稳定 kernel 控制调用

它不理解 `agent_tree` 内部结构，也不直接操作底层 mailbox。

### 4. route / wake / close 只在确实是产品合同的前提下暴露

如果某个旧能力只是内部辅助方法，则不升级成公开合同。只有满足以下条件的能力才保留：

- 旧项目确实有真实外部调用语义
- 当前产品仍需要该能力
- 该能力能用稳定输入输出表达，而不是泄漏内部实现

### 5. observability 走稳定视图，不走内部事件实现

对外的 observe / status 必须基于稳定快照与稳定事件视图；内部事件总线、内部 actor 通讯协议不视为对外合同。

## Risks and Mitigations

### 风险：把 `kernel` 重新写成新的 runtime façade

缓解：

- `kernel` 只提供全局控制合同，不负责 session 真相、不负责 turn 构造。
- 组合根仍然只在 `server/bootstrap`。

### 风险：route / wake / close 继续沿用旧项目内部语义

缓解：

- 先定义合同，再按合同实现。
- 无法定义稳定合同的旧行为不迁入公开层。
