## 1. 统一治理包络模型

- [ ] 1.1 在 `crates/core` 或 `crates/application` 中引入统一治理包络类型（如 `ResolvedGovernanceSurface`），覆盖 scoped router、prompt declarations、resolved limits、overrides、injected messages、policy context 与 collaboration audit context
- [ ] 1.2 梳理 `crates/session-runtime/src/turn/submit.rs` 中 `AgentPromptSubmission` 的职责，决定是替换还是瘦身为治理包络的 transport 形状
- [ ] 1.3 为统一治理包络补充字段校验与基础单元测试，确保不同入口可复用同一输出形状

## 2. 收口入口装配路径

- [ ] 2.1 在 `crates/application` 新增治理装配服务，让 `execution/root.rs`、`execution/subagent.rs` 与 `session_use_cases.rs` 统一调用
- [ ] 2.2 清理 root / session / subagent 三条路径里手工拼接 scoped router、prompt declarations、limits 的重复逻辑
- [ ] 2.3 为 root、普通 session submit、fresh child、resumed child 四类入口补充一致性测试

## 3. Capability Router 统一装配

- [ ] 3.1 将 `execution/root.rs:71-85` 的 root capability router 构建迁入治理装配器
- [ ] 3.2 将 `execution/subagent.rs:141-172` 的 child capability router 构建（parent_allowed_tools ∩ SpawnCapabilityGrant 交集）迁入治理装配器
- [ ] 3.3 将 `agent/routing.rs:571-722` 的 resumed child scoped router 构建迁入治理装配器
- [ ] 3.4 确保三条路径统一后各自保留必要的入口类型差异化参数，补充回归测试

## 4. 执行限制与控制收口

- [ ] 4.1 将 `ResolvedExecutionLimitsSnapshot` 的构建逻辑从各入口迁入治理装配器
- [ ] 4.2 将 `ExecutionControl`（max_steps、manual_compact）作为治理装配器的输入参数，不再在 session_use_cases.rs 中直接覆写 runtime config
- [ ] 4.3 将 `ForkMode` 和上下文继承策略（`select_inherited_recent_tail`）作为治理包络的一部分
- [ ] 4.4 评估 `SubmitBusyPolicy` 是否需要成为治理包络的可配置字段，还是保持固定策略
- [ ] 4.5 将 `AgentConfig` 中 max_subrun_depth、max_spawn_per_turn 等治理参数改为治理装配器的输入源，不再被消费方直接读取

## 5. 策略引擎接入管线

- [ ] 5.1 将 `PolicyContext` 的构建改为从治理包络派生，消除与治理包络字段的重复组装
- [ ] 5.2 确保 `PolicyEngine` 的三个检查点能读取治理包络中的 capability surface 和 execution limits
- [ ] 5.3 建立 `ApprovalRequest` / `ApprovalResolution` / `ApprovalPending` 的管线骨架，但默认不触发
- [ ] 5.4 保持 `AllowAllPolicyEngine` 作为默认实现，补充管线存在但默认放行的测试

## 6. 委派策略元数据收口

- [ ] 6.1 将 `build_delegation_metadata`（agent/mod.rs:287-312）迁入治理装配器
- [ ] 6.2 将 `SpawnCapabilityGrant` 的解析从 spawn 参数迁入治理包络
- [ ] 6.3 将 `AgentCollaborationPolicyContext` 的构建改为从治理包络获取参数
- [ ] 6.4 将 `enforce_spawn_budget_for_turn` 改为使用治理包络中的限制参数
- [ ] 6.5 确保 `persist_delegation_for_handle` 持久化的数据与治理包络一致

## 7. Prompt 与 Delegation 真相收口

- [ ] 7.1 将 `crates/application/src/agent/mod.rs` 中 fresh/resumed child contract 生成逻辑迁入统一治理装配链路
- [ ] 7.2 收口 `crates/adapter-prompt/src/contributors/workflow_examples.rs` 中 authoritative 协作 guidance，使 adapter 仅渲染上游声明
- [ ] 7.3 确保 delegation catalog、child contract 与协作 guidance 使用同一治理事实源，并补充回归测试

## 8. Prompt 事实治理联动显式化

- [ ] 8.1 将 `prompt_declaration_is_visible`（prompt_facts.rs:200-213）的过滤逻辑上移到治理装配层
- [ ] 8.2 将 `PromptFacts.metadata` 中 `agentMaxSubrunDepth` / `agentMaxSpawnPerTurn` 改为从治理包络获取，消除 vars dict 传递
- [ ] 8.3 确保 `build_profile_context` 中的 approvalMode 与治理包络中的策略配置一致
- [ ] 8.4 重构 `PromptFactsProvider` 为治理包络的消费者，不再独立实现治理过滤

## 9. Bootstrap/Runtime 治理生命周期

- [ ] 9.1 在 `GovernanceBuildInput`（server/bootstrap/governance.rs）中预留 mode catalog 参数（Option 类型）
- [ ] 9.2 在 `AppGovernance.reload()` 编排中预留 mode catalog 替换步骤（本轮为空操作）
- [ ] 9.3 确保 `RuntimeCoordinator.replace_runtime_surface` 后续 turn 使用更新后的治理包络
- [ ] 9.4 确保 `CapabilitySurfaceSync` 能力面变更后通知治理装配器刷新缓存

## 10. 协作审计事实关联

- [ ] 10.1 为 `AgentCollaborationFact` 增加可选的治理包络标识字段（governance_revision 或 envelope_hash）
- [ ] 10.2 将 `CollaborationFactRecord` 的构建参数改为从治理包络获取
- [ ] 10.3 确保 `AGENT_COLLABORATION_POLICY_REVISION` 与治理包络中的策略版本一致

## 11. 清理与验证

- [ ] 11.1 清理旧 helper、重复命名与临时桥接代码，保持模块职责与文件结构清晰一致
- [ ] 11.2 运行 `cargo fmt --all`
- [ ] 11.3 运行 `cargo test --workspace --exclude astrcode`
- [ ] 11.4 运行 `node scripts/check-crate-boundaries.mjs`
- [ ] 11.5 手动验证 root/session/subagent 提交路径的默认行为等价，且治理声明来源已统一
- [ ] 11.6 验证 PolicyEngine 管线存在但默认行为与当前等价
- [ ] 11.7 验证 PromptFactsProvider 退化后 prompt 输出与当前等价
