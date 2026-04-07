# 子会话模式边界分析

## 结论先行

Astrcode 当前采纳的是“受控子会话”，不是“默认独立 session”，也不是“共享父全部状态”的轻量分叉。

## 已固定的方向

### 1. `shared_session` 是正式路径

它适合当前主线，因为：

- 父会话可直接看到子执行事件
- UI 和父流程消费结果最直接
- 生命周期与 observability 更容易统一

### 2. `independent_session` 保持 experimental

它可以保留，但只作为扩展面；不能反过来定义主协议。

### 3. 先做 shared observability，不做 shared mutable state

父流程可以聚合子执行的：

- token
- step
- outcome
- findings
- artifacts

但不应该让子 Agent 直接修改父的运行时状态。

### 4. 控制面永远归根执行域

任务注册、cleanup、kill、timeout、MCP 生命周期等能力，必须从 session mode 中抽离出来，回到 root-owned task control。

## 四层边界

后续如果要增加 override 或共享能力，必须先说明它落在哪一层：

1. Storage
2. Context inheritance
3. Control linkage
4. Observability

不要再出现“共享一点父状态”这种模糊设计。

## 明确拒绝

- 直接照搬 Claude 的父状态共享模型
- 共享 UI 权限提示状态
- 新增模糊的 shared app state override

## 对应规范

- [../spec/agent-tool-and-api-spec.md](../spec/agent-tool-and-api-spec.md)
- [../spec/open-items.md](../spec/open-items.md)


