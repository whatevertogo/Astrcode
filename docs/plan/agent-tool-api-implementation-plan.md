# Agent as Tool + 开放 API 实施计划（同步版）

> **最后更新**：2026-04-07  
> **当前状态**：Phase 0-2 已完成；Phase 3 已有真实 root execution、sub-run 状态查询与显式取消；Phase 4 暂缓；Phase 5 为初版完成；Phase 6 为部分完成。

```text
Phase 0 (设计) ✅
  → Phase 1 (Agent Loader) ✅
  → Phase 2 (Agent as Tool / Controlled Sub-Session) ✅
  → Phase 3 (扩展 API) 🟡 部分完成
  → Phase 4 (WebSocket) ⏸ 暂缓
  → Phase 5 (前端适配) 🟡 初版完成
  → Phase 6 (测试与回归) 🟡 持续补强
```

---

## 已完成内容（简化总结，放最上面）

### 1. Agent Loader 已完成

- 已支持从 Markdown / YAML 加载 Agent 定义
- 已内置 profile：`explore` / `plan` / `reviewer` / `execute`
- 已集成到 Runtime Bootstrap
- 已支持热重载

**对应实现：**
- `crates/runtime-agent-loader/src/lib.rs`

### 2. `spawnAgent` 工具与受控子会话主链路已完成

当前仓库已经明确从早期“共享 session + 事件打标”的原型，演进到：

> **`spawnAgent + controlled sub-session`**

已经具备：

- `SpawnAgentTool` 通过 `SubAgentExecutor` 委托执行
- `SpawnAgentParams` 使用稳定的扁平 schema：`type` / `description` / `prompt` / `context`
- 参数校验失败会返回结构化 `ToolExecutionResult`
- `launch_subagent()` 使用统一执行入口
- 默认后台启动，快速返回 `SubRunResult`
- 通过结构化 `ArtifactRef { kind: "subRun" }` 暴露后台句柄

**对应实现：**
- `crates/runtime-agent-tool/src/lib.rs`
- `crates/runtime/src/service/execution/subagent.rs`
- `crates/runtime-execution/src/prep.rs`

### 3. 核心数据模型与生命周期事件已完成

已具备当前主线需要的关键 DTO：

- `InvocationKind::{RootExecution, SubRun}`
- `SubRunStorageMode::{SharedSession, IndependentSession}`
- `SubRunHandle`
- `SubRunResult / SubRunHandoff / SubRunFailure`
- `SubRunStarted / SubRunFinished`
- `AgentEventContext.child_session_id`

**对应实现：**
- `crates/core/src/agent/mod.rs`
- `crates/core/src/event/types.rs`

### 4. API 初版已可用

已落地：

- `GET /api/v1/agents`
- `POST /api/v1/agents/{id}/execute`
- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`
- `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`
- `GET /api/v1/tools`
- `POST /api/v1/tools/{id}/execute`（当前仍为 `501 Not Implemented` 骨架）

**对应实现：**
- `crates/server/src/http/routes/mod.rs`
- `crates/server/src/http/routes/agents.rs`
- `crates/server/src/http/routes/tools.rs`

### 5. 前端与测试已有可用闭环

**前端：**
- SSE 事件已能提取 `subRunId / storageMode / childSessionId`
- `MessageList` 已按 `subRunId` 归组
- `SubRunBlock` 已内联展示子会话运行状态与结果

**测试：**
- tool 参数解析 / 错误路径测试
- 后台 `subRun` artifact 测试
- 基础集成测试
- 显式取消释放并发槽位测试
- Agent Control 的 spawn / cancel / wait / GC 测试

**对应实现：**
- `frontend/src/lib/applyAgentEvent.ts`
- `frontend/src/components/Chat/MessageList.tsx`
- `frontend/src/components/Chat/SubRunBlock.tsx`
- `crates/runtime/src/service/execution/tests.rs`
- `crates/runtime-agent-tool/src/lib.rs`
- `crates/runtime-agent-control/src/lib.rs`

---

## Phase 0：设计 ✅ 已完成

### 已完成

- [x] 明确 Agent as Tool 的整体边界
- [x] 确定 Agent Profile、Tool、Runtime、Server 的分层职责
- [x] 明确后续 API 由现有 `server` crate 承载，不新增独立 API crate

### 建议

- 后续设计评审统一以 **controlled sub-session** 为中心，不再回到早期“单纯共享 session + 事件打标”的表述。

---

## Phase 1：Agent Loader ✅ 已完成

### 已完成

- [x] Markdown / YAML Agent Profile 加载
- [x] builtin profile 注册
- [x] Runtime Bootstrap 集成
- [x] 热重载支持

### 建议

- 除非出现新的 profile 继承或分层需求，否则这一阶段不应继续膨胀。
- 后续 profile 侧增强应优先围绕：
  - 可见工具边界
  - 提示词来源可追踪
  - 配置诊断体验

---

## Phase 2：Agent as Tool / Controlled Sub-Session ✅ 已完成

## 2.1 工具定义与参数

### 已完成

- [x] `SpawnAgentTool` 通过 trait 解耦 runtime
- [x] 参数 schema 稳定为 `type` / `description` / `prompt` / `context`
- [x] 参数校验失败时返回结构化 tool failure
- [x] tool description 不再固化动态 profile 列表

### 如何完成（基于实际内容的思考）

这一阶段最重要的完成点，不是“把 Agent 做成一个更强大的超级工具”，而是**把工具面收窄为稳定的子会话入口**。  
这让：

- LLM 面对的 schema 更简单
- runtime 可以自由演进
- profile 发现能力也能独立演进

### 建议

- **保持 `spawnAgent` 工具 schema 极简。**
- **不要把 `storageMode` 或更多 override 直接暴露给 LLM。**

## 2.2 Runtime 执行链

### 已完成

- [x] 统一入口 `launch_subagent()`
- [x] `resolve_profile()` / `resolve_parent_execution()`
- [x] `prepare_child()` / `spawn_child()`
- [x] `build_event_sinks()` / `run_child_loop()`
- [x] `finalize_child_execution()`
- [x] 后台启动时返回 `subRun` artifact

### 已确认的当前行为

- `spawnAgent` 默认后台启动
- `SubRunResult.status=Running` 时，`handoff.artifacts` 可包含：
  - `subRun`
  - `session`（当存在独立子会话时）

### 如何完成（基于实际内容的思考）

仓库已经实际选择了：

- **后台默认**
- **结构化返回句柄**
- **受控子会话**

这条路线比早期“tool 内部编排多任务 / 用布尔参数切换后台”更稳定，也更容易维护。

### 建议

- **不要再回到 `runInBackground` 这类显式布尔开关。**
- **不要恢复工具内部的多任务 DSL。**

## 2.3 策略与预算

### 已完成

- [x] `SubAgentPolicyEngine`：白名单过滤 + Ask → Deny
- [x] `ChildExecutionTracker`：步数 / token 预算跟踪
- [x] 子 Agent 继续受父策略上界约束

### 建议

- 该阶段后续应以**补测试**为主，而不是继续扩充更多策略开关。

---

## Phase 3：扩展 API 🟡 部分完成

## 3.1 已完成

### 路由

- [x] `GET /api/v1/agents`
- [x] `POST /api/v1/agents/{id}/execute`
- [x] `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`
- [x] `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`
- [x] `GET /api/v1/tools`
- [x] `POST /api/v1/tools/{id}/execute`（501 骨架）

### DTO 与映射

- [x] `AgentProfileDto`
- [x] `AgentExecuteRequestDto`
- [x] `AgentExecuteResponseDto`
- [x] `SubRunStatusDto`
- [x] `ToolDescriptorDto`

### override / 观测现状

- [x] root execution API 已支持有限 `contextOverrides`
- [x] 当前允许 `storageMode`、`includeCompactSummary`、`includeRecentTail`
- [x] 当前明确拒绝：
  - `inheritCancelToken=false`
  - `includeRecoveryRefs=true`
  - `includeParentFindings=true`
- [x] runtime status 已可暴露 `metrics.subrunExecution`

## 3.2 待完成

- [ ] `POST /api/v1/sessions/{id}/abort` — turn 级取消
- [ ] `POST /api/v1/sessions/{id}/fork` — 从指定 turn 派生新 session
- [ ] `POST /api/v1/sessions/{id}/revert` — 回滚到指定 turn
- [ ] `/api/v1/tools/{id}/execute` 升级为真实执行端点，或正式确认长期保留 501

## 3.3 如何完成（基于实际内容的思考）

这一阶段的关键不是“多加几个 API”，而是保持边界清晰：

- `spawnAgent` 是给 LLM 用的稳定入口
- root execution API 是给外部系统用的显式入口
- `tools/{id}/execute` 如果要变真实执行端点，必须复用现有执行链，不能另起一套主路径

## 3.4 建议

- **优先补 `abort / fork / revert`，因为它们更接近 session 生命周期主线。**
- `/api/v1/tools/{id}/execute` 如果短期无明确消费者，可以继续保留 501。
- 如果未来实现真实工具执行端点，必须复用现有 session / tool runtime，不要新造直通路径。

---

## 多 Agent 后续 Roadmap（当前仍成立）

### 1. `root-owned task registry`

- 子 Agent 启动的 shell / MCP / 长任务应统一归 runtime 根级 task registry 管理
- kill / cleanup / timeout 不应依赖 `SharedSession` / `IndependentSession`

### 2. `shared observability aggregation`

- 允许父流程聚合 step / token / outcome / findings / artifacts 摘要
- 不引入父状态直写能力

### 3. `IndependentSession` 继续保持 experimental

- 当前只承诺：可查询 / 可展示 / 可回填结果
- 在控制平面清晰前，不扩大产品承诺范围

---

## Phase 4：WebSocket ⏸ 暂缓

### 当前判断

当前已经通过：

- SSE 事件流
- 断点续传

满足实时推送需要。  
因此 WebSocket 不是当前优先项。

### 何时再考虑推进

仅当出现以下真实需求时再重启：

- 客户端主动 steer / follow-up
- 需要真正的双向交互
- 需要比 SSE 更复杂的会话控制语义

### 建议

- **Phase 4 暂不实施，不要抢在控制平面之前推进。**

---

## Phase 5：前端适配 🟡 初版完成

## 5.1 已完成

- [x] SSE 事件已提取 `subRunId / storageMode / childSessionId`
- [x] 状态层已携带子会话相关字段
- [x] `MessageList` 已按 `subRunId` 分组
- [x] `SubRunBlock` 已支持：
  - 运行中状态
  - 完成状态
  - 失败信息
  - 结果 handoff 展示
  - 显式取消按钮

## 5.2 待完成

- [ ] 改善 `SubRunBlock` 的信息层级与可读性
- [ ] 对 `IndependentSession` 增加“打开子会话”入口
- [ ] 评估是否需要“完整子会话详情页”
- [ ] 改善多层 / 长输出场景下的 UI 体验

## 5.3 如何完成（基于实际内容的思考）

当前前端已经证明协议链路打通了，所以后续重点不是再造协议，而是：

- 降低阅读成本
- 更清晰地区分运行中 / 完成 / 失败态
- 让 `childSessionId` 成为真正可点击的引用

## 5.4 建议

- **短期继续保留 inline `SubRunBlock`。**
- **中期只在存在 `childSessionId` 时增加“打开子会话”入口。**
- **不要急着做复杂树状多层 UI。**

---

## Phase 6：测试与回归 🟡 持续补强

## 6.1 已完成

- [x] Agent Loader 基础测试
- [x] Agent Tool 参数解析 / 失败路径测试
- [x] `spawnAgent` 基础集成测试
- [x] 后台 `subRun` artifact 测试
- [x] 显式取消释放并发槽位测试
- [x] Agent Control 的 spawn / list / cancel / wait / GC 测试

## 6.2 待补充

- [ ] `SubAgentPolicyEngine::check_capability_call` 三个分支测试
- [ ] `CapabilityRouter::subset_for_tools` 测试
- [ ] `IndependentSession` 端到端测试：
  - child session 建立
  - parent / child sink 分离
  - `child_session_id` 回填
  - 查询 / 取消 / 恢复显示
- [ ] 生命周期事件完整性测试：
  - `SubRunStarted`
  - `SubRunFinished`

## 6.3 如何完成（基于实际内容的思考）

测试不应只继续补 happy path，而应优先守住最容易在重构里退化的边界：

- 参数校验
- 工具裁剪
- 存储切换
- 后台取消
- 生命周期事件完整性

## 6.4 建议

- **优先补“边界回归测试”，而不是继续堆功能展示型测试。**
- **独立子会话测试一定要覆盖 parent sink / child sink 分离。**

---

## 风险与缓解（同步版）

| 风险 | 影响 | 缓解 |
|------|------|------|
| 文档继续沿用旧语义（`runAgent` / `isolated_session`） | 设计讨论反复回摆 | 统一到 `spawnAgent + controlled sub-session` |
| `IndependentSession` 过早扩大承诺 | 产品语义不稳 | 在控制平面补齐前保持 experimental |
| API 再分叉出第二套执行主链 | 维护成本上升 | 新端点必须复用现有 runtime 执行链 |
| 前端过早追求复杂树状视图 | UI 复杂度失控 | 先优化 inline `SubRunBlock` |
| 缺少存储切换与取消回归测试 | 重构后隐性退化 | 优先补 boundary tests |

---

## 验证命令速查（已修正）

```bash
# 全量检查
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode

# 单个 package
cargo test --package astrcode-runtime-agent-loader
cargo test --package astrcode-runtime-agent-tool
cargo test --package astrcode-runtime

# 前端检查
cd frontend && npm run typecheck && npm run lint
```

---

## 一句话建议

**后续不要再按“重构一个更复杂的 Agent 工具”推进，而应继续沿着 `spawnAgent + controlled sub-session` 主线，收口文档、补强控制平面、完善前端与测试。**
