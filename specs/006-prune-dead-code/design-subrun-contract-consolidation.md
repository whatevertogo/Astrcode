# Design: Subrun 核心模型与服务契约收口

## 1. 目标

这份设计处理“仍在使用、但语义重复或归属错误”的 subrun 核心模型与服务契约：

1. 单一 subrun 状态模型
2. 单一 lineage 真相层
3. 单一 execution receipt
4. 正确的 trait owner
5. 清晰的 child reference / navigation 分层
6. 强类型 protocol 状态
7. 单一 prompt metrics payload
8. 集中的 compaction 原因映射

## 2. Canonical Model Targets

| Current redundancy | Canonical target |
|--------------------|------------------|
| `SubRunOutcome` + `AgentStatus` | `AgentStatus` |
| `SubRunDescriptor` + `SubRunHandle` | `SubRunHandle` |
| optional `parent_turn_id` | required `SubRunHandle.parent_turn_id` |
| `PromptAccepted` / `RootExecutionAccepted` / runtime duplicates | `ExecutionAccepted` |
| 手工 subrun event context 拼装 | `From<&SubRunHandle> for AgentEventContext` |
| `ChildAgentRef.openable` | 删除，UI 基于 canonical open target 判断 |
| `ChildSessionNotification.open_session_id` + `child_ref.open_session_id` | `child_ref.open_session_id` |
| protocol `status: String` | protocol `AgentStatusDto`（或等价命名） |
| `PromptMetrics` 三层重复 variant 载荷 | 共享 `PromptMetricsPayload` |
| `HookCompactionReason` / `CompactTrigger` 的散落映射 | 单一 compaction reason mapping owner |

## 3. 状态模型收口

### 3.1 设计

- 在 `AgentStatus` 中新增 `TokenExceeded`
- `is_final()` 覆盖 `TokenExceeded`
- `SubRunResult.status` 改用 `AgentStatus`
- 删除 `status_from_result()`、`map_outcome_status()`、`to_subrun_outcome_dto()` 这类纯映射函数

### 3.2 约束

- protocol 可以保留 DTO-only 镜像枚举，但不能再维护第二套业务状态语义
- `Cancelled` 成为唯一取消终态，`Aborted` 退出主线

## 4. Lineage 模型收口

### 4.1 设计

- 删除 `SubRunDescriptor`
- `SubRunHandle.parent_turn_id` 改为必填
- `SubRunHandle::descriptor()` 删除
- `SubRunStatusSnapshot`、storage events、server mapper、frontend reader 全部直接从 handle / durable node 读取字段

### 4.2 约束

- 缺少 lineage 核心字段的旧输入不再得到 downgrade 视图
- storage mode 不能决定 lineage 是否“看起来完整”

## 5. Execution receipt 收口

### 5.1 设计

- 在 `core` 引入单一 `ExecutionAccepted`
- `ExecutionOrchestrationBoundary::submit_prompt` 与 `execute_root_agent` 都返回该模型
- `runtime::service_contract` 删除本地重复 receipt
- 旧 root execute HTTP route 既然本次删除，就不需要再为它保留独立 receipt 壳

### 5.2 约束

- HTTP 路由可以按需要做 DTO 投影，但内部 contract 不能再分裂

## 6. Trait owner 收口

### 6.1 设计

- `submit_prompt` / `interrupt_session` / `execute_root_agent` 继续属于 `ExecutionOrchestrationBoundary`
- `launch_subagent` 迁入 `LiveSubRunControlBoundary`

### 6.2 原因

`launch_subagent` 依赖当前 live child ownership、tool context 和 active control tree，而不是纯 root orchestration。

## 7. Child reference 与 navigation 分层

### 7.1 设计

- `ChildAgentRef` 保留 identity / lineage / status / canonical `open_session_id`
- `openable` 删除；“是否可打开”由是否存在 canonical open target 决定
- `ChildSessionNotification` 与 protocol DTO 不再额外重复 `open_session_id`
- child navigation 依赖 `child_ref.open_session_id` 或 durable child session fact，而不是 duplicated bool 或外层重复字段

### 7.2 原因

这能把 open target 作为单一正式事实保留，同时把纯 UI 便利字段留在 projection 层，而不是在 core/protocol 里双写。

## 8. Protocol 状态枚举与共享 payload

### 8.1 设计

- protocol 为 child/subrun 相关状态补齐独立强类型枚举
- server mapper 负责 core `AgentStatus` 到 protocol 枚举的显式映射
- `PromptMetrics` 提取共享 payload，由 storage/domain/protocol 三层引用

### 8.2 原因

protocol 可以保持 DTO-only，但不能把核心状态退化成字符串，也不应长期维护三套 100% 同构的 metrics 字段清单。

## 9. Compaction 原因归一

- `HookCompactionReason` 保留 `Reactive`
- durable `CompactTrigger` 仍只表达正式落盘 trigger
- `Reactive -> durable trigger` 的映射收敛到单一 owner，不允许在多个调用点手写

## 10. 注释契约

- `ToolExecutionResult::model_content()` 必须明确说明为什么不能被绕过
- `split_assistant_content()` 必须说明为什么 `to_ascii_lowercase()` 的索引仍然能安全映射到原字符串

## 11. Guardrails

- 不通过 adapter 保留旧模型名字
- 不通过 optional 字段继续维护 downgrade 语义
- 不把 UI 派生字段重新塞回 core
- 不在 notification 外层重复存放 child open target
- 不把 protocol 状态重新退化成字符串
- 不让共享 metrics payload 再裂成多份手工字段清单
- 不因“调用点很多”而放弃 canonical owner
