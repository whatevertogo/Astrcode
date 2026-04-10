# Design: Legacy 公开语义删除与当前主线切换

## 1. 目标

本设计关注两类遗留物：

1. 旧子智能体/旧共享历史/旧 subrun 模型的 downgrade 公开语义
2. 已经被注释为 legacy、但当前主线仍在使用的控制入口

我们的目标不是“保留兼容，同时尽量隐藏”，而是：

- 对不再支持的旧输入给出明确失败
- 对仍在使用的旧入口完成迁移，然后删除

## 2. Legacy 公开语义的删除原则

### 2.1 删除的对象

- `legacyDurable` 这类把“不支持旧历史”包装成“部分可用状态”的公开状态源
- descriptor-missing legacy tree 相关前端 helper 分支
- 仅为 legacy 样本 UI 展示保留的协议/前端类型
- live 文档里把 legacy 路径描述成稳定、实验性或仍在评估中的能力

### 2.2 保留的对象

- 明确失败能力
- 能帮助调用方识别失败原因的稳定错误信息
- archive 文档中的历史记录

### 2.3 失败方式

当旧共享历史或 legacy-only 输入进入当前主线流程时：

- 系统必须明确失败
- 不再返回 downgrade status、legacy tree 或“部分可用”视图
- 不再伪造 lineage、child 结构或可点击浏览结果

## 3. 当前 cancel 主线的 cutover

### 3.1 现状

当前“取消子会话”按钮虽然已经被注释成 legacy 路径，但它依然是产品主线：

- UI 按钮在 `SubRunBlock`
- 事件流通过 `Chat` / `App` / `useAgent`
- 最终打到 `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`

### 3.2 目标

把这条路径切到 `closeAgent` 协作能力，形成单一控制入口：

- UI 仍保留“取消子会话”动作
- 但动作语义变成“关闭目标 agent / child session”
- 删除 legacy cancel route 与 client wrapper

### 3.3 设计约束

- 不允许长期同时保留 REST cancel 和 `closeAgent`
- 不允许为了迁移再引入新的 adapter route
- 不允许因为迁移而失去当前用户可见的取消能力

## 4. 前端读模型收口

### 4.1 保留

- 当前 focused subrun 浏览
- 当前 child session 直开
- 当前消息/通知驱动的子执行展示

### 4.2 删除

- 无消费者的 parent summary projection
- legacy downgrade tree 分支
- 仅服务 legacy 展示的状态类型

### 4.3 原则

前端只消费当前 server 明确支持的 surface，不再承担“帮旧历史猜测一个还能看的样子”的职责。

## 5. 文档与开放项收口

### 5.1 live 文档必须删除的说法

- `SharedSession` 是当前正式主线
- `IndependentSession` 仍是 experimental
- `/api/v1/agents`、`/api/v1/tools`、`/api/runtime/plugins` 是当前支持 API 面
- `/api/v1/tools/{id}/execute` 的去留还未决定

### 5.2 live 文档必须保留的说法

- 当前产品依赖的会话历史 / SSE / 配置读写 / 模型选择面
- 当前 child session 入口与摘要事实
- 当前唯一主线 cancel 入口

## 6. Guardrails

- 不把 archive 文档当成 live 事实来维护
- 不把明确失败重新包装成 downgrade capability
- 不把 legacy 清理做成新的兼容系统
