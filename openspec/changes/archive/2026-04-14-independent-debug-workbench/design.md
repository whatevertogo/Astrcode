## Context

Astrcode 已经有一条可用但偏临时的调试链路：

- `AppGovernance::observability_snapshot()` 提供全局 runtime 指标快照。
- `server` 在 debug 构建中暴露 `/api/debug/runtime/metrics`。
- 前端浮层轮询这个接口，并在本地内存里累计最近 5 分钟趋势。
- 当前会话的实时工具活动则直接复用主聊天页已有的 SSE 消息流。

这条链路证明了调试值本身是有价值的，但形态和边界都还不够稳定：一方面 UI 形态过于拥挤，另一方面趋势真相只保存在前端内存，既不利于多窗口，也不利于未来做统一导出或问题回溯。

## Goals / Non-Goals

**Goals:**

- 将 debug 读取能力抽成一个独立后端 crate，并保持 `server` 只做装配与 DTO 映射。
- 用服务端维护的时间窗口替代“前端本地趋势缓存作为主真相”的做法。
- 在现有 `frontend` 工程中增加独立 Debug Workbench 入口，并在 Tauri 中以独立窗口打开。
- 第一版保持只读观测，不新增治理写操作。

**Non-Goals:**

- 不重写现有 observability pipeline。
- 不引入独立 debug 服务进程。
- 不让 debug UI 直接访问 runtime 内部结构，所有数据仍然通过 HTTP DTO 暴露。

## Decisions

### 决策 1：新增独立 debug crate，但 server 仍然是唯一组合根

新增 `crates/debug-workbench`，专门承载：

- runtime overview 读模型
- 时间窗口 timeline 聚合
- session trace 聚合
- session agent tree / lineage 聚合

`server` 继续负责：

- 注入 `App` / `AppGovernance`
- debug-only 路由挂载
- auth 与 DTO 映射

这样可以把 debug 领域逻辑与 HTTP 层分开，同时不打破现有组合根约束。

### 决策 2：趋势样本由服务端 ring buffer 维护，而不是继续以前端本地采样为主

Debug Workbench 需要“最近 5 分钟趋势”在窗口关闭、刷新甚至重开后仍可读，因此趋势样本应由服务端维护。

本次采用 debug-only ring buffer：

- 样本源：当前 `RuntimeObservabilitySnapshot`
- 采样内容：`spawn_rejection_ratio_bps`、`observe_to_action_ratio_bps`、`child_reuse_ratio_bps`
- 时间窗口：默认保留最近 5 分钟样本
- 维护方式：由 `debug-workbench` crate 在读取 overview/timeline 前更新当前样本，并对窗口进行裁剪

这个方案的权衡是：样本密度与读取频率相关，而不是固定后台定时任务。它足够支撑 v1 的只读调试窗口，同时避免再引入一条后台采样任务链路。

### 决策 3：会话 trace 与 agent tree 分离成两类读模型

Debug Workbench 需要同时支持：

- 以时间顺序查看某个 session 的实时/近实时事件
- 以结构方式查看该 session 下的 child agent lineage

因此读模型拆成两类：

- `SessionDebugTrace`
- `SessionDebugAgents`

前者优先复用现有 `session_history` 与事件 envelope；后者优先复用 session runtime 中已构建的 child node 投影，而不是在前端根据事件再做一次启发式重建。

### 决策 4：前端保持在现有工程内，但入口与窗口独立

本次不做独立 debug 项目，而是在现有 `frontend/` 工程中增加第二个 entry：

- 主应用入口：`index.html` + `src/main.tsx`
- Debug Workbench 入口：`debug.html` + `src/debug-main.tsx`

这样既能保持共享的样式、类型和 API 层，又能在 Tauri 里打开真正独立的 debug 窗口，而不是继续把调试 UI 嵌在主界面里。

### 决策 5：主窗口只保留轻量入口，不再渲染旧浮层

右下角浮层已经完成了验证使命，但不适合长期存在。本次改为：

- 主窗口内删除旧浮层与切换按钮
- 在 debug 模式下保留一个轻量入口，用于打开或聚焦 Debug Workbench 窗口

这样主聊天界面不会再被调试卡片挤压，同时桌面端仍然保留从主窗口进入 Workbench 的便利路径。

## Risks / Trade-offs

- [Risk] 新增 crate 后，debug 查询和已有 observability 逻辑边界不清  
  Mitigation：新 crate 只承载读模型与聚合，不迁移原始 collector 与 snapshot 真相。

- [Risk] 服务端 ring buffer 只在读取时更新，样本时间分辨率受实际读取频率影响  
  Mitigation：v1 明确定位为调试窗口驱动的时间窗口；若后续需要固定采样，再单独演进为后台采样任务。

- [Risk] 多入口前端与双窗口 Tauri 配置增加构建复杂度  
  Mitigation：保持同一 Vite 工程与同一 server origin，避免引入额外前端项目或额外 sidecar。

- [Risk] `/api/debug/sessions/{id}/trace` 与已有 `/api/sessions/{id}/history` 职责重叠  
  Mitigation：debug trace 返回 workbench 所需的聚合视图，允许复用底层真相但不直接暴露现有业务接口。

## Migration Plan

1. 增加 `debug-workbench` crate，并让 server 注入它。
2. 扩展 protocol DTO 与 `/api/debug/*` 路由。
3. 将旧前端浮层替换为独立 Debug Workbench 页面。
4. 在 Tauri 中新增 `debug-workbench` 窗口与主窗口入口。
5. 删除旧浮层渲染逻辑，并补充后端/前端/桌面端测试。

## Open Questions

- v1 的 session trace 是否直接暴露完整 event envelope，还是裁成更轻的 trace item DTO？本次优先采用更轻的 trace item，避免 debug 前端再运行一套完整聊天重建逻辑。
- 主窗口入口最终放在标题栏菜单、设置页还是侧边栏操作区？本次实现优先选择侵入最小的位置，只要不再回到右下角浮层即可。
