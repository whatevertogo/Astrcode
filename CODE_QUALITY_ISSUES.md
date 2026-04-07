# 项目混乱点分析

> 分析日期：2026-04-08
> 基于 `master` 分支暂存区状态（90 files, +7,167 / -1,780）

---

## 一、架构层面问题

### 1.1 core 依赖 protocol（违反架构规则）

CLAUDE.md 明确规定 **`protocol` 不得依赖 `core`/`runtime`**，但实际发现 **`core` 反向依赖了 `protocol`**。这违反了分层依赖原则，造成依赖方向混乱。

- 涉及文件：`crates/core/Cargo.toml`

### 1.2 core/runtime 和 runtime/ 职责重叠

- `core/src/runtime/traits.rs` 定义了 `ExecutionOrchestrationBoundary`、`SessionTruthBoundary`、`LiveSubRunControlBoundary` 等核心接口
- `runtime/src/lib.rs` 的 `RuntimeService` 又暴露了更高层但功能重叠的 API
- 两者之间的抽象层次不清晰，使用者容易困惑应该调用哪一层

### 1.3 runtime-session 和 runtime-execution 边界模糊

- `runtime-session/session_state.rs` 和 `turn_runtime.rs` 都管理会话执行状态
- `runtime-execution/context.rs` 也处理上下文和 lineage 跟踪
- 上下文解析、lineage 管理等逻辑散落在两个 crate 之间，职责归属不明确

---

## 二、代码质量问题

### 2.1 错误类型各自为政

| 位置 | 自定义错误类型 |
|------|--------------|
| `core/src/error.rs` | `AstrError`（主错误类型） |
| `protocol/src/plugin/error.rs` | `ProtocolError`、`ErrorPayload` |
| `runtime-agent-control/src/lib.rs` | `AgentControlError` |
| `runtime-llm/src/lib.rs` | 自定义错误 |
| `runtime-prompt/src/template.rs` | 自定义错误 |
| `sdk/src/error.rs` | 自定义错误 |

各 crate 独立定义错误类型，没有统一使用或兼容 `core::AstrError`，导致错误处理链路不一致，上层调用者需要处理多种错误类型转换。

### 2.2 编译警告/错误未清理

| 文件 | 问题 |
|------|------|
| `server/src/http/routes/sessions/filter.rs:3` | unused import `LINEAGE_METADATA_UNAVAILABLE_MESSAGE` |
| `runtime/src/service/watch_ops.rs:332` | `&Cow<str>` 的 `Pattern` trait bound 不满足（编译错误） |

### 2.3 service/mod.rs 文件体量过大

| 文件 | 行数 | 问题 |
|------|------|------|
| `runtime/src/service/mod.rs` | ~17,000 行 | 包含过多实现细节，违反单一职责 |
| `runtime/src/service/execution/mod.rs` | ~14,000 行 | root execution、subagent、status 多职责混合 |
| `runtime/src/service/composer_ops.rs` | ~14,000 行 | 体量过大 |
| `runtime/src/service/config_manager.rs` | ~11,000 行 | 体量过大 |
| `runtime/src/service/watch_ops.rs` | ~13,000 行 | 体量过大 |

---

## 三、前后端协议对齐问题

### 3.1 事件类型 DTO 不一致

- `core/src/event/domain.rs` 的 `AgentEvent` 中 `agent` 字段类型为 `AgentEventContext`
- `protocol/src/http/event.rs` 的 `AgentEventPayload` 中 `agent` 字段类型为 `AgentContextDto`
- 两个平行的枚举定义需要通过手动映射保持同步，容易遗漏字段

### 3.2 前端类型定义分散

- `frontend/src/types.ts` 定义了 `ToolCallResultEnvelope` 等核心类型
- `frontend/src/lib/api/models.ts` 定义了 `CurrentModelInfo`、`ModelOption` 等 API 类型
- 两个文件之间没有互相引用，新增类型时容易放错位置

### 3.3 Spec 与实现不同步

- `specs/001-runtime-boundary-refactor/contracts/` 中定义的 `source` 字段在 `server/src/http/routes/sessions/query.rs` 中未实现
- 子代理取消接口的 409 冲突处理逻辑在代码中不完整
- `legacyDurable` 状态的支持在代码中未找到对应实现

---

## 四、硬编码与 TODO 积压

### 4.1 硬编码常量散落

| 位置 | 值 | 应提取为 |
|------|---|---------|
| `core/src/local_server.rs` | `62000`（端口号） | 常量或配置 |
| `server/src/bootstrap/mod.rs` | `62000`（端口号，重复） | 同上 |
| `core/src/event/types.rs` | `128000`（context window） | 常量 |
| `runtime-tool-loader/src/builtin_tools/read_file.rs` | `20000`（maxChars） | 常量 |
| `runtime-tool-loader/src/builtin_tools/grep.rs` | `200000`（最大字符数） | 常量 |

端口号 `62000` 在两个文件中重复硬编码，修改时容易遗漏。

### 4.2 关键 TODO 未解决

| 位置 | 内容 | 影响 |
|------|------|------|
| `core/src/agent/mod.rs:61` | Agent 枚举消费未完整实现 | 功能缺失 |
| `runtime/src/plugin_hook_adapter.rs:11` | Hook 协议未实现 | 插件系统不可用 |
| `runtime-agent-loop/src/agent_loop.rs:637` | 手动压缩与自动压缩冲突 | 可能导致 bug |
| `runtime-agent-loop/src/context_window/compaction.rs:79` | 多模态消息不支持 | 功能受限 |
| `runtime-config/src/constants.rs:462` | 多 Agent 间消息传递未实现 | 多 Agent 架构缺失 |

### 4.3 前端空 catch 块

- `frontend/src/lib/api/client.ts` 存在 `catch {}` 空捕获块，错误被静默吞掉

---

## 五、测试覆盖缺口

以下核心模块缺少测试：

- `core/src/runtime/traits.rs` — 核心接口无测试
- `runtime-agent-tool/` — 工具注册无测试
- `runtime-prompt/` — Prompt 模板无测试
- `runtime-llm/` — LLM 集成无测试
- `runtime/src/plugin_hook_adapter.rs` — 插件适配器无测试

---

---

## 六、并发安全问题

### 6.1 Poisoned Lock 直接 panic

| 文件 | 行号 | 代码 |
|------|------|------|
| `plugin/src/peer.rs` | 300 | `.lock().unwrap()` — 读循环句柄锁 |
| `runtime-skill-loader/src/skill_catalog.rs` | 59 | `.write().unwrap()` — 技能目录写锁 |
| `runtime-skill-loader/src/skill_catalog.rs` | 65 | `.read().unwrap()` — 技能目录读锁 |
| `runtime/src/service/session/create.rs` | 15 | `RuntimeService::from_capabilities(…).unwrap()` |
| `server/src/bootstrap/mod.rs` | 281-282 | 硬编码 URL 解析 unwrap |

> 注：`runtime-session/src/support.rs` 已实现了 `with_lock_recovery` 和 `lock_anyhow` 安全工具，但只有 session 模块在用，其他模块未采用。

### 6.2 Tokio 任务泄漏（fire-and-forget）

| 文件 | 行号 | 问题 |
|------|------|------|
| `plugin/src/peer.rs` | 293 | `tokio::spawn(read_loop)` 无 handle 保存，无法取消 |
| `runtime/src/service/watch_manager.rs` | 28, 46 | config 热重载 watcher，fire-and-forget |
| `runtime/src/service/execution/subagent.rs` | 128 | 子 agent 执行 spawn 无 handle |
| `runtime/src/service/execution/root.rs` | 168 | turn 执行 spawn 无 handle |

这些任务在异常情况下无法被取消或回收，可能导致资源泄漏。

### 6.3 SessionState 锁粒度问题

`runtime-session/src/session_state.rs` 中 `SessionState` 使用 **9 个独立的 `StdMutex`**（phase、cancel、active_turn_id、turn_lease、token_budget、compact_failure_count、projector、recent_records、recent_stored）。同时访问多个字段时存在死锁风险。

建议合并为单一内部结构体 `SessionStateInner`，用一把锁保护。

---

## 七、不安全的 unwrap 和过度 clone

### 7.1 生产代码中的 unwrap

- `runtime-agent-loop/src/context_window/file_access.rs:233` — `tracker.entries.last().unwrap().path`
- `server/src/bootstrap/mod.rs:281-282` — URL 解析

### 7.2 过度 clone 热点

| 文件 | clone 次数 | 说明 |
|------|-----------|------|
| `runtime-agent-loop/src/agent_loop/turn_runner.rs` | ~22 次 | state、messages、agent、request、cancel 反复克隆 |
| `core/src/event/translate.rs` | ~20 次 | 事件传播时参数克隆 |
| `runtime/src/service/execution/root.rs` | 大量 | 会话和执行上下文克隆 |
| `core/src/tool.rs` | 大量 | 工具上下文克隆 |

`turn_runner.rs` 中 `state.messages.clone()` 在循环中出现，是潜在性能瓶颈。建议使用 `Arc` 或 `Cow` 减少拷贝。

### 7.3 dead_code 抑制

| 文件 | 数量 |
|------|------|
| `runtime-llm/src/openai.rs:832` | 1 处 |
| `runtime-llm/src/anthropic.rs:1010` | 1 处 |
| `runtime-agent-loop/src/prompt_runtime.rs:48,51` | 2 处 |
| `runtime-agent-loop/src/context_window/file_access.rs:23,26,110` | 3 处 |

这些 `#[allow(dead_code)]` 标注的代码需要审查：要么删除，要么补齐使用。

---

## 八、前端代码组织问题

### 8.1 过大文件

| 文件 | 行数 | 建议 |
|------|------|------|
| `frontend/src/App.tsx` | 712 行 | 拆分为主布局 + 子组件 |
| `frontend/src/store/reducer.ts` | 704 行 | 按 domain 拆分 reducer |
| `frontend/src/lib/agentEvent.ts` | 570 行 | 按事件类型拆分 |
| `frontend/src/types.ts` | 540 行 | 按功能分类拆分 |

### 8.2 状态管理分散

- `App.tsx` 使用 `useReducer` 管理全局状态
- 各组件独立管理本地状态
- 没有明确的全局 Context Provider 层，状态传递依赖 props 层层传递

### 8.3 其他前端问题

- `AssistantMessage.tsx` 存在 `eslint-disable-next-line @typescript-eslint/no-explicit-any`，any 类型逃逸
- React 导入方式不一致（部分 `import React`，部分 `import type { ReactNode }`）
- 状态管理缺乏统一架构（无 Redux/Zustand），全部靠 useReducer + props drilling

---

## 九、优先级排序（更新）

| 优先级 | 问题 | 理由 |
|--------|------|------|
| **P0** | `watch_ops.rs` 编译错误 | 代码无法通过编译 |
| **P0** | core 依赖 protocol | 违反架构规则 |
| **P0** | Poisoned lock panic（plugin/skill-loader） | 生产环境可能直接崩溃 |
| **P1** | Tokio 任务泄漏（4处 fire-and-forget） | 资源泄漏、无法优雅关闭 |
| **P1** | 错误类型各自为政 | 错误处理不可靠 |
| **P1** | service/mod.rs 过大（17K行） | 严重影响可维护性 |
| **P1** | Spec 与实现不同步 | 接口契约不可靠 |
| **P1** | turn_runner.rs 过度 clone | 性能瓶颈 |
| **P2** | SessionState 9 把锁 | 死锁风险 |
| **P2** | 硬编码常量 | 维护风险 |
| **P2** | 前端类型分散 / 文件过大 | 前后端对齐风险 |
| **P2** | TODO 积压 | 功能缺口 |
| **P2** | dead_code 抑制 | 代码腐化 |
| **P2** | 依赖版本未统一到 workspace（toml/tracing/async-stream/tower） | 版本管理风险 |
| **P2** | API 路径命名不一致（/api vs /api/v1，单复数混用） | 接口规范性 |
| **P2** | 工具/Agent 路由缺少输入验证 | 安全风险 |
| **P3** | 测试覆盖缺口 | 长期质量保障 |
| **P3** | SSE 流缺少速率限制和错误重试 | 高负载风险 |
| **P3** | CORS 硬编码 localhost | 部署灵活性 |

---

## 十、依赖管理问题

### 10.1 依赖版本未统一到 workspace

| 文件 | 依赖 | 当前写法 | 应改为 |
|------|------|---------|--------|
| `crates/core/Cargo.toml` | `toml = "1.1.2"` | 直接版本号 | `toml.workspace = true` |
| `crates/runtime-execution/Cargo.toml` | `tracing = "0.1"` | 直接版本号 | `tracing.workspace = true` |
| `crates/server/Cargo.toml` | `async-stream = "0.3"` | 直接版本号 | `async-stream.workspace = true` |
| `crates/server/Cargo.toml` | `tower = "0.5"` | 直接版本号 | `tower.workspace = true` |

> 其他依赖（tokio、serde、serde_json）已正确使用 `workspace = true`。

---

## 十一、API 设计问题

### 11.1 路由前缀不一致

- `/api/sessions`（无版本前缀）
- `/api/v1/tools`（有 v1 前缀,不应该有版本号）
- `/api/v1/agents`（有 v1 前缀,不应该有版本号）
- `/api/config`（无版本前缀）
- `/api/runtime/...`（无版本前缀）
- `/api/models`（无版本前缀）

应在全局统一选择一种风格。

### 11.2 输入验证缺失

| 路由 | 缺失验证 |
|------|---------|
| `POST /api/v1/tools/{id}` | 工具 ID 路径未验证 |
| `POST /api/v1/agents/{id}/execute` | Agent ID 路径未验证 |
| `POST /api/config/active-selection` | 请求体参数未验证 |

### 11.3 SSE 流健壮性

- ✅ 已处理客户端断开
- ✅ 有 15 秒 keep-alive 心跳
- ❌ 缺少流速率限制（高频事件可能压垮客户端）
- ❌ lagged 恢复失败后直接断开，无重试机制

### 11.4 CORS 硬编码

CORS 允许来源硬编码为 `localhost:5173`，部署到生产环境时无法通过环境变量配置。

---

## 十二、日志与可观测性

### 12.1 log 和 tracing 混用

- 项目主体使用 `log::` 宏（warn!、error!、info!、debug!）
- 但 `runtime-execution/src/subrun.rs:286` 使用了 `tracing::warn!`，是唯一一处 tracing 调用
- 完全没有使用结构化日志（tracing::instrument、span），所有日志均为纯文本

### 12.2 日志级别使用不当

| 文件 | 问题 |
|------|------|
| `runtime/src/service/execution/mod.rs:232` | turn failed 用 `warn!` 而非 `error!` |
| `runtime-agent-loop/src/hook_runtime.rs:192` | hook call failed 用 `debug!` 而非 `error!` |
| `core/src/runtime/coordinator.rs:120` | 关键通信失败用 `debug!` 而非 `error!` |

### 12.3 错误被静默吞掉

**.ok() 忽略错误：**

| 文件 | 说明 |
|------|------|
| `storage/src/session/event_log.rs:254` | 文件操作失败被忽略 |
| `runtime-skill-loader/src/skill_loader.rs:293-294` | 时间解析失败被忽略 |
| `server/src/http/mapper.rs:443` | 解析失败被忽略 |

**let _ = ... 忽略结果：**

| 文件 | 说明 |
|------|------|
| `runtime/src/service/turn/orchestration.rs:64` | 取消操作失败被忽略 |
| `runtime-session/src/turn_runtime.rs:52,65` | 广播失败被忽略 |

### 12.4 生产代码中的 panic!/todo!

| 文件 | 行号 | 内容 |
|------|------|------|
| `storage/src/session/turn_lock.rs` | 376, 415, 463 | 锁相关 panic |
| `runtime-execution/src/subrun.rs` | 867, 883 | 状态机错误 panic |
| `runtime-agent-loop/src/context_window/prune_pass.rs` | 198, 258 | 消息类型错误 panic |
| `runtime/src/plugin_hook_adapter.rs` | 443, 463 | 未实现的 todo! |

### 12.5 关键操作缺少日志

- 会话创建成功无日志（只有冲突时有）
- 工具开始执行无日志（只有完成/失败时有）
- 子代理启动/完成的关键路径日志不足

### 12.6 println! 残留

- `runtime-config/src/loader.rs:92` — 用 println! 提示用户填写配置，应改为 log::warn!

---

## 十三、Trait 设计问题

### 13.1 Trait 过于庞大

`core/src/runtime/traits.rs` 中的 `ExecutionOrchestrationBoundary` 同时承担了 prompt 提交、会话中断、根代理执行、子代理调度四项职责，建议按职责拆分。

### 13.2 方法签名参数过多

`execute_root_agent` 有 6 个参数（含多个 Option），建议使用参数对象模式（`RootAgentExecutionParams`）。

### 13.3 不必要的动态分派

| 位置 | 类型 | 实现者数量 | 建议 |
|------|------|-----------|------|
| `core/src/runtime/coordinator.rs` | `Arc<dyn RuntimeHandle>` | 仅 1 个 | 改为泛型或枚举 |
| `core/src/store.rs` | `Box<dyn EventLogWriter>` | 仅 2 个 | 改为泛型或枚举 |

### 13.4 空 Trait

`core/src/store.rs` 中的 `SessionTurnLease` trait 没有定义任何方法，仅作为标记使用。如果不需要多态，可以直接用具体类型。

### 13.5 类型别名缺乏类型安全

`core/src/tool.rs` 中 `pub type SessionId = String;` 没有提供编译期类型安全，建议改为 newtype。

---

## 十四、存储层问题

### 14.1 写入错误恢复缺失

`storage/src/session/event_log.rs:179-190`：`sync_all()` 失败后无重试机制，系统崩溃可能导致部分事件丢失。

### 14.2 写入非原子性

`event_log.rs:167-170`：JSON 写入成功但换行符写入失败时，文件中会出现不完整的行，后续读取可能报错。

### 14.3 缺少数据完整性校验

- 没有文件校验和（checksum）验证
- 没有数据完整性检查机制
- 读取损坏行时缺少容错处理

### 14.4 缺少临时文件管理

没有临时文件创建和清理机制，潜在磁盘空间泄漏风险。

### 14.5 配置热重载竞态

`runtime/src/service/config_manager.rs`：配置重载时先读取新配置再重建 agent loop，这段时间内可能出现不一致状态。防抖机制（300ms）不完善，多次文件修改可能触发多次重载。

---

## 十五、优先级排序（最终版）

| 优先级 | 问题 | 理由 |
|--------|------|------|
| **P0** | `watch_ops.rs` 编译错误 | 代码无法通过编译 |
| **P0** | core 依赖 protocol | 违反架构规则 |
| **P0** | Poisoned lock panic（plugin/skill-loader） | 生产环境可能直接崩溃 |
| **P1** | Tokio 任务泄漏（4处 fire-and-forget） | 资源泄漏、无法优雅关闭 |
| **P1** | 错误类型各自为政 | 错误处理不可靠 |
| **P1** | service/mod.rs 过大（17K行） | 严重影响可维护性 |
| **P1** | Spec 与实现不同步 | 接口契约不可靠 |
| **P1** | turn_runner.rs 过度 clone | 性能瓶颈 |
| **P1** | 生产代码中的 panic!/todo! | 健壮性风险 |
| **P1** | 错误被静默吞掉（.ok()、let _ =） | 问题难以排查 |
| **P1** | 日志级别混乱 | 关键错误日志缺失 |
| **P2** | SessionState 9 把锁 | 死锁风险 |
| **P2** | 硬编码常量 | 维护风险 |
| **P2** | 前端类型分散 / 文件过大 | 前后端对齐风险 |
| **P2** | TODO 积压 | 功能缺口 |
| **P2** | dead_code 抑制 | 代码腐化 |
| **P2** | 依赖版本未统一到 workspace | 版本管理风险 |
| **P2** | API 路径命名不一致 | 接口规范性 |
| **P2** | 工具/Agent 路由缺少输入验证 | 安全风险 |
| **P2** | 存储层写入非原子 | 数据丢失风险 |
| **P2** | 配置热重载竞态 | 运行时一致性 |
| **P2** | Trait 过大 / 不必要的动态分派 | 性能和可维护性 |
| **P3** | 测试覆盖缺口 | 长期质量保障 |
| **P3** | SSE 流缺少速率限制和错误重试 | 高负载风险 |
| **P3** | CORS 硬编码 localhost | 部署灵活性 |
| **P3** | log/tracing 混用 | 可观测性 |
| **P3** | 存储层缺少完整性校验 | 数据可靠性 |
| **P3** | println! 残留 | 代码规范 |
| **P3** | CI 缺少 concurrency / artifact 配置 | CI 效率 |
| **P3** | 缺少 [profile.release] 优化配置 | 构建产物性能 |

---

## 十六、安全问题

### 16.1 命令执行风险

- `runtime-tool-loader/src/builtin_tools/shell.rs:441` — `Command::new(&spec.program).args(&spec.args)` 直接使用 LLM 返回的 shell 指令执行程序
- `runtime-config/src/editor.rs:20` — `Command::new(&open_command.program)` 使用配置中的编辑器命令

> 注：shell 工具本身就是让 AI Agent 执行命令的设计意图，但需要确保不在未授权场景下暴露。

### 16.2 文件路径工具无沙箱

- `read_file.rs`、`write_file.rs`、`edit_file.rs` — 工具接受 AI Agent 提供的路径直接操作文件系统
- `read_file.rs` 已有路径验证（验证通过），但写入/编辑工具的路径验证强度需确认

### 16.3 Tauri CSP 允许 unsafe-inline

- `src-tauri/tauri.conf.json` CSP 配置中 `style-src 'self' 'unsafe-inline'` 降低了 XSS 防护

### 16.4 事件反序列化无大小限制

- `protocol/src/http/event.rs` 的事件反序列化没有输入大小限制，理论上可被利用为 DoS 攻击向量

---

## 十七、错误处理补充

### 17.1 router.rs 9 处 .expect() 在生产代码中

`runtime-registry/src/router.rs` 中 9 处使用 `.expect()` 获取读写锁：

| 行号 | 代码 |
|------|------|
| 114, 131 | `.write().expect("capability router write lock")` |
| 161, 171, 191, 200, 223, 276, 281 | `.read().expect("capability router read lock")` |

### 17.2 supervisor.rs 持锁 await

`plugin/src/supervisor.rs:200`：
```rust
self.process.lock().await.shutdown().await
```
在持有 `Mutex` 的情况下调用 `.shutdown().await`，阻塞其他任务访问 process。

### 17.3 runtime-agent-control expect panic

`runtime-agent-control/src/lib.rs:608`：`.expect("waiter should finish before timeout")` — timeout 超时时直接 panic。

---

## 十八、构建与 CI 问题

### 18.1 CI 缺少并发控制

所有 `.github/workflows/*.yml` 均无 `concurrency` 配置，同一 PR 多次 push 会触发多个并行 CI 运行，浪费资源。

### 18.2 CI 缺少 artifact 上传

所有 workflow 都没有配置 `actions/upload-artifact`，构建产物无法缓存或共享给后续步骤。

### 18.3 缺少 [profile.release] 优化

根 `Cargo.toml` 无 `[profile.release]` 配置，未启用 LTO、strip 等优化，导致发布产物体积较大。

### 18.4 ESLint 禁用 no-undef

`frontend/eslint.config.js:24-26` 禁用了 `no-undef` 规则（注释称 TypeScript 已负责检查，合理但需确认）。

---

## 十九、文档与迁移不同步

### 19.1 migration.md 与实际代码不一致

`specs/001-runtime-boundary-refactor/migration.md` 记录已删除 `session_service.rs`、`execution_service.rs`、`replay.rs`，但这些文件仍在暂存区的变更列表中（标记为 `D` 已删除），说明迁移正在途中但文档和实际不同步。

### 19.2 protocol crate 新增 DTO 缺少文档

`protocol/src/http/event.rs` 中新增的子运行相关 DTO（`SubRunStorageModeDto`、`ForkModeDto`、`SubRunOutcomeDto` 等）缺少 `///` 文档注释。早期定义的类型（如 `PhaseDto`）有文档，但新类型没有。

---

## 二十、验证记录

以下发现经过验证后**修正或剔除**：

| 原始发现 | 验证结论 | 原因 |
|---------|---------|------|
| `read_file.rs` 无路径验证 | **否认** | 实际已有路径验证机制 |
| `grep.rs:1240-1245` 数组越界 | **否认** | 在测试代码中，非生产路径 |
| `query.rs:662` 数组越界 | **修正** | 在测试代码中，降级为 P3 |
| `protocol/event.rs` 所有 DTO 无文档 | **部分否认** | 早期类型有文档，仅新增类型缺失 |
| 生产代码有 `todo!` | **否认** | 搜索 crates/ 未找到 `todo!` 宏调用 |

---

## 二十一、优先级排序（最终完整版）

| 优先级 | 问题 | 理由 |
|--------|------|------|
| **P0** | `watch_ops.rs:332` 编译错误（测试代码） | CI 无法通过 |
| **P0** | core 依赖 protocol | 违反架构规则 |
| **P0** | Poisoned lock panic（plugin/skill-loader/router.rs 9处） | 生产环境可能直接崩溃 |
| **P0** | CSP `unsafe-inline` | 安全策略缺陷 |
| **P1** | Tokio 任务泄漏（4处 fire-and-forget） | 资源泄漏、无法优雅关闭 |
| **P1** | 错误类型各自为政（6个 crate 独立定义） | 错误处理不可靠 |
| **P1** | service/mod.rs 过大（17K行） | 严重影响可维护性 |
| **P1** | Spec 与实现不同步 | 接口契约不可靠 |
| **P1** | turn_runner.rs 过度 clone | 性能瓶颈 |
| **P1** | supervisor.rs 持锁 await | 死锁风险 |
| **P1** | agent-control expect panic | 健壮性风险 |
| **P1** | 错误被静默吞掉（.ok()、let _ =） | 问题难以排查 |
| **P1** | 日志级别混乱（3处关键错误用 debug!/warn!） | 关键错误日志缺失 |
| **P2** | SessionState 9 把锁 | 死锁风险 |
| **P2** | 硬编码常量（端口号 62000 重复等） | 维护风险 |
| **P2** | 前端类型分散 / 文件过大 | 前后端对齐风险 |
| **P2** | TODO 积压（5项关键 TODO） | 功能缺口 |
| **P2** | dead_code 抑制（7处） | 代码腐化 |
| **P2** | 依赖版本未统一到 workspace（4个依赖） | 版本管理风险 |
| **P2** | API 路径命名不一致（/api vs /api/v1） | 接口规范性 |
| **P2** | Agent 路由缺少输入验证 | 安全风险 |
| **P2** | 存储层写入非原子 | 数据丢失风险 |
| **P2** | 配置热重载竞态 | 运行时一致性 |
| **P2** | Trait 过大 / 不必要的动态分派 | 性能和可维护性 |
| **P2** | 事件反序列化无大小限制 | DoS 风险 |
| **P3** | 测试覆盖缺口 | 长期质量保障 |
| **P3** | SSE 流缺少速率限制和错误重试 | 高负载风险 |
| **P3** | CORS 硬编码 localhost | 部署灵活性 |
| **P3** | log/tracing 混用 | 可观测性 |
| **P3** | 存储层缺少完整性校验 | 数据可靠性 |
| **P3** | println! 残留 | 代码规范 |
| **P3** | CI 缺少 concurrency / artifact 配置 | CI 效率 |
| **P3** | 缺少 [profile.release] 优化配置 | 构建产物性能 |
| **P3** | migration.md 与实际代码不同步 | 文档准确性 |
| **P3** | 新增 DTO 缺少文档注释 | API 可用性 |
