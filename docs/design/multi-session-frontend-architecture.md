# 多会话前端架构设计

## 问题

前端同时要处理：

- 根 session
- 同 session 内的 subrun
- 独立 child session

如果把它们混成一种对象，导航、渲染和 SSE 订阅都会变乱。

## 设计结论

### 1. session 与 subrun 是两种不同导航对象

- session 是 durable 会话实体
- subrun 是某个 session 中的子执行视图对象

二者可以关联，但不能混成同一种 ID 或同一种列表。

### 2. 先做“当前会话内可导航”，再做“全局会话树”

当前优先级应该是：

- 根 session 页面可看见 subrun 卡片
- SharedSession 可在当前 session 内切换到某个 subrun 视图
- IndependentSession 可跳转到 child session

不要一开始就做全局 session tree。

### 3. MVP 直接基于 durable 事件构建 subrun 树

当前推荐：

- 首屏从 `/history` hydration
- 增量从 `/events` 订阅
- 本地从 `SubRunStarted / SubRunFinished` 重建 subrun 列表与层级

如果后续载荷变大，再补 `list_subruns` 一类只读接口。

### 4. 一条 session 一个 SSE 连接

先保证：

- 每个 session 只有一条 SSE 连接
- 不为每个 subrun 单独开连接
- 过滤视图优先在客户端完成

### 5. 前端 render model 不是后端契约

后端只需要稳定提供统一事件协议；具体 UI 卡片结构、breadcrumb、折叠策略由前端决定。

## 当前不建议

- 用 `tool_name == "spawnAgent"` 硬编码识别 subrun
- 用简单 `subRunId == ...` 判断事件归属全部视图
- 为了前端方便，把 session tree 下沉成 runtime 基础领域对象

## 对应规范

- [../spec/session-and-subrun-spec.md](../spec/session-and-subrun-spec.md)
- [../spec/open-items.md](../spec/open-items.md)
