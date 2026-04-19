## 1. Core 治理模型

- [ ] 1.1 在 `crates/core/src/mode/mod.rs` 定义开放式 `ModeId`、`GovernanceModeSpec`、`CapabilitySelector`（含 Name/Kind/SideEffect/Tag/AllTools 及组合操作）、`ActionPolicies`、`ChildPolicySpec` 与 `ResolvedTurnEnvelope`
- [ ] 1.2 在 `crates/core/src/lib.rs` 导出 mode 模块，并为序列化、校验与默认 builtin mode ID 补充单元测试
- [ ] 1.3 在 `crates/core/src/event/types.rs` 增加 `ModeChanged { from: ModeId, to: ModeId }` 事件载荷，并补充旧会话默认回退到 `execute` 的测试

## 2. Capability Selector 编译

- [ ] 2.1 在 `crates/application/src/mode/compiler.rs` 实现 `CapabilitySelector -> CapabilityRouter` 编译逻辑，从当前 `CapabilitySpec` / capability router 投影能力面
- [ ] 2.2 实现组合选择器（交集、并集、差集）的编译逻辑
- [ ] 2.3 确保 child capability router 从 parent mode child policy + SpawnCapabilityGrant 交集推导
- [ ] 2.4 为 execute/plan/review 三个 builtin mode 的 capability 选择编写等价测试

## 3. Application Catalog 与 Compiler

- [ ] 3.1 在 `crates/application/src/mode/catalog.rs` 实现 builtin mode catalog，定义 `execute`（code）、`plan`、`review` 三个首批 mode spec（含 CapabilitySelector、ActionPolicies、ChildPolicySpec、prompt program）
- [ ] 3.2 在 `crates/application/src/mode/compiler.rs` 实现 `GovernanceModeSpec -> ResolvedTurnEnvelope` 完整编译逻辑（capability router + prompt declarations + execution limits + action policies + child policy）
- [ ] 3.3 在 `crates/application/src/mode/validator.rs` 实现统一 transition / entry policy 校验入口，并补充非法切换、next-turn 生效等测试

## 4. Mode 执行限制

- [ ] 4.1 在 envelope 编译中实现 mode-specific max_steps 解析，与用户 ExecutionControl 取交集（更严格者生效）
- [ ] 4.2 在 envelope 编译中实现 mode-specific ForkMode 约束（如 restricted mode 强制 LastNTurns）
- [ ] 4.3 评估 SubmitBusyPolicy 是否需要由 mode 指定，如果是则在 envelope 编译中实现
- [ ] 4.4 在 envelope 编译中实现 AgentConfig 治理参数覆盖（max_subrun_depth、max_spawn_per_turn）

## 5. Mode Policy Engine 集成

- [ ] 5.1 将 `PolicyContext` 构建改为从治理包络派生，消除与治理包络字段的重复组装
- [ ] 5.2 实现 mode action policies 到 `PolicyEngine` 行为的映射（Allow/Deny 裁决）
- [ ] 5.3 确保 `decide_context_strategy` 能参考 mode 的上下文治理偏好
- [ ] 5.4 确保 builtin mode 使用 AllowAllPolicyEngine 等价行为，补充默认放行测试

## 6. Mode Prompt Program

- [ ] 6.1 为 execute/plan/review 三个 builtin mode 定义 prompt program（生成 PromptDeclarations）
- [ ] 6.2 确保 mode declarations 通过标准注入路径（`TurnRunRequest.prompt_declarations` -> `PromptDeclarationContributor`）进入 prompt 组装
- [ ] 6.3 重构 `WorkflowExamplesContributor`，让治理专属内容改为由 mode prompt program 提供
- [ ] 6.4 确保 `PromptFactsProvider` 的 metadata 和 declaration 过滤与 mode envelope 保持一致
- [ ] 6.5 验证 `CapabilityPromptContributor` 和 `AgentProfileSummaryContributor` 通过 PromptContext 自动响应 mode 能力面变化

## 7. Session Runtime 集成

- [ ] 7.1 在 `crates/core/src/projection/agent_state.rs` 的 `AgentState` 增加 `mode_id` 字段，在 `AgentStateProjector::apply()` 增加 `ModeChanged` 事件处理
- [ ] 7.2 在 `crates/session-runtime/src/state/mod.rs` 的 `SessionState` 增加 `current_mode` per-field mutex
- [ ] 7.3 在 `crates/session-runtime/src/turn/submit.rs` 的 submit 边界解析当前 mode，并把 `ResolvedTurnEnvelope` 收口到 `AgentPromptSubmission` / `RunnerRequest`
- [ ] 7.4 修改 `crates/session-runtime/src/turn/runner.rs`，确保 turn 工具面从 envelope 的 capability router 读取
- [ ] 7.5 确保旧 session replay 时 ModeChanged 事件缺失回退到 `execute`

## 8. Delegation 与 Child Policy

- [ ] 8.1 实现 mode child policy 到 `DelegationMetadata` 的推导逻辑
- [ ] 8.2 实现 mode child policy 到 `SpawnCapabilityGrant` 的推导逻辑
- [ ] 8.3 修改 `crates/application/src/execution/subagent.rs` 和 `agent/mod.rs`，让 child 初始 mode 和 execution contract 来自 resolved child policy
- [ ] 8.4 确保 `AgentProfileSummaryContributor` 在 mode 禁止 delegation 时不渲染（通过现有 spawn 守卫条件自动生效）

## 9. /mode 命令

- [ ] 9.1 在 `crates/cli/src/command/mod.rs` 的 `Command` enum 增加 `Mode { query: Option<String> }` 变体
- [ ] 9.2 在 `parse_command` 中增加 `"/mode"` arm
- [ ] 9.3 实现 tab 补全，从 mode catalog 获取候选（集成到 slash_candidates 机制）
- [ ] 9.4 在 `coordinator.rs` 中实现 `/mode` 命令的执行调度
- [ ] 9.5 实现 mode 状态显示（当前 mode、可用 mode 列表、transition 拒绝反馈）

## 10. Bootstrap、Reload 与验证

- [ ] 10.1 在 `crates/server/src/bootstrap/governance.rs` 的 `GovernanceBuildInput` 中增加 mode catalog 参数
- [ ] 10.2 在 `ServerRuntimeReloader` 的 reload 编排中增加 mode catalog 替换步骤（与能力面替换原子性）
- [ ] 10.3 在 `crates/server/src/bootstrap/runtime.rs` 中装配 builtin mode catalog
- [ ] 10.4 确保插件 mode 在 bootstrap 握手阶段可注册到同一 catalog

## 11. 协作审计与可观测性

- [ ] 11.1 在 `AgentCollaborationFact` 增加可选的 `mode_id` 字段
- [ ] 11.2 确保审计事实记录当前 turn 开始时的 mode（不受 turn 内 mode 变更影响）
- [ ] 11.3 在 `ObservabilitySnapshotProvider` 快照中增加当前 mode 和变更时间戳
- [ ] 11.4 实现 envelope 编译的诊断信息记录（如空能力面警告）

## 12. 集成测试与验证

- [ ] 12.1 为 mode-aware collaboration guidance、delegation catalog 和 restricted child contract 增加回归测试
- [ ] 12.2 为 mode 切换的 next-turn 生效语义增加测试
- [ ] 12.3 为 CapabilitySelector 编译的等价性增加测试
- [ ] 12.4 为 PolicyEngine 与 mode action policies 的集成增加测试
- [ ] 12.5 运行 `cargo fmt --all`
- [ ] 12.6 运行 `cargo test --workspace --exclude astrcode`
- [ ] 12.7 运行 `node scripts/check-crate-boundaries.mjs`
- [ ] 12.8 手动验证：切换 builtin mode、确认下一 turn 生效、确认 child delegation surface 与 prompt guidance 随 mode 收敛
