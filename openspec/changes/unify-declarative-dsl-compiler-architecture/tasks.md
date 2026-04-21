## 1. 文档与契约对齐

- [ ] 1.1 更新 `PROJECT_ARCHITECTURE.md` 与 `docs/architecture/declarative-dsl-compiler-target.md`，明确 `compile` / `bind` / `orchestrate` 术语、mode contract 边界与 plugin reload 一致性约束。验证：人工审阅文档；`git diff --check`.
- [ ] 1.2 盘点并更新相关 OpenSpec 主 spec 与实现注释中的旧术语，避免继续把 `ResolvedTurnEnvelope` 和 `ResolvedGovernanceSurface` 混称为同一层结果。验证：`rg -n "ResolvedTurnEnvelope|GovernanceSurfaceAssembler|compile_mode_envelope" openspec crates`.

## 2. 扩展 GovernanceModeSpec

- [ ] 2.1a 在 `crates/core/src/mode/mod.rs` 新增 `ModeArtifactDef` 结构体（artifact_type, file_template, schema_template, required_headings, actionable_sections），补充序列化与校验。验证：新增 `cargo test -p astrcode-core mode::mode_artifact_def`.
- [ ] 2.1b 新增 `ModeExitGateDef` 结构体（review_passes, review_checklist），补充序列化与校验。验证：新增 `cargo test -p astrcode-core mode::mode_exit_gate_def`.
- [ ] 2.1c 新增 `ModePromptHooks` 结构体（reentry_prompt, initial_template, exit_prompt, facts_template），补充序列化与校验。验证：新增 `cargo test -p astrcode-core mode::mode_prompt_hooks`.
- [ ] 2.1d 新增 `ModeWorkflowBinding` 结构体（workflow_id, phase_id, phase_role）与 `PhaseRole` 枚举，补充序列化与校验。验证：新增 `cargo test -p astrcode-core mode::mode_workflow_binding`.
- [ ] 2.1e 在 `GovernanceModeSpec` 上增加四个 `Option` 字段（artifact, exit_gate, prompt_hooks, workflow_binding），扩展 `validate()` 递归校验新字段。验证：`cargo test -p astrcode-core mode`.
- [ ] 2.2 调整 `crates/protocol/src/plugin/handshake.rs` 及其测试，确保插件通过 `InitializeResultData.modes` 声明扩展后的 mode contract 时仍保持纯 DTO 形状（字段可选，缺失时与旧行为等价）。验证：`cargo test -p astrcode-protocol plugin`.
- [ ] 2.3 让 builtin `plan` mode 在 `crates/application/src/mode/catalog.rs` 中以新 mode contract 字段表达当前 artifact / exit / prompt / workflow 语义，而不是只靠工具名约定。验证：新增/更新 `cargo test -p astrcode-application mode::catalog`，确认 plan mode 的新字段声明与现有行为等价。

## 3. 显式化治理 compile / bind 边界

- [ ] 3.1 重构 `crates/application/src/mode/compiler.rs`，把 selector 求值、mode contract 派生、child/grant 裁剪与 diagnostics 明确收敛到编译阶段产物中。验证：新增/更新 `cargo test -p astrcode-application mode::compiler`.
- [ ] 3.2 调整 `crates/application/src/governance_surface/assembler.rs` 与 `mod.rs`，把运行时/profile/session/control 绑定责任与 compile 责任分开；必要时仅做渐进命名收束，不强求一次性全量改名。验证：新增/更新 `cargo test -p astrcode-application governance_surface`.
- [ ] 3.3 收敛治理 prompt 来源，在 `crates/application/src/governance_surface/prompt.rs`、`crates/adapter-prompt/src/plan.rs`、`crates/adapter-prompt/src/block.rs` 之间保留单一 `PromptPlan` 结果模型，并补充来源 metadata。验证：`cargo test -p astrcode-adapter-prompt`.

## 4. workflow 轻量编译与 phase-mode 绑定

- [ ] 4.1 在 `crates/core/src/workflow.rs` 或 `crates/application/src/workflow/*` 中补充 workflow validate/compile 边界，使 workflow 在进入 orchestrator 前先完成显式校验。验证：新增/更新 `cargo test -p astrcode-application workflow`.
- [ ] 4.2 调整 `crates/application/src/workflow/orchestrator.rs`，让 phase -> mode 绑定显式引用 mode contract，而不是在 orchestrator 内重编码 plan artifact 或 exit 规则。验证：新增/更新 `cargo test -p astrcode-application workflow::orchestrator`.
- [ ] 4.3 保持当前 workflow 数据结构克制，不引入与现有规模不匹配的索引化结构，同时补充对应注释与测试断言。验证：人工审阅实现；相关 workflow 单测通过。

## 5. reload 一致性与回滚

- [ ] 5.1 重构 `crates/server/src/bootstrap/governance.rs`，把 mode catalog、capability surface、skill catalog 组织成统一候选治理快照，并在失败时完整回滚。验证：新增/更新 `cargo test -p astrcode-server bootstrap::governance`.
- [ ] 5.2 调整 `crates/server/src/bootstrap/capabilities.rs` 与相关组合根逻辑，保证 reload 后的新 turn 看到的是同一版本的治理输入，而执行中的 turn 继续使用旧快照。验证：新增/更新 `cargo test -p astrcode-server bootstrap::capabilities`.
- [ ] 5.3 为 reload 成功/失败路径补充 observability 或日志诊断，能够说明 mode catalog / capability surface / skill catalog 的快照切换边界。验证：自动化测试或手动检查日志输出。

## 6. 通用工具与 prompt 迁移

- [ ] 6.1 在 `crates/adapter-tools/src/builtin_tools/` 新增 `upsert_mode_artifact.rs`，实现通用 `upsertModeArtifact` 工具。该工具读取当前 mode 的 `ModeArtifactDef`，按 `artifact_type` / `file_template` 管理 CRUD lifecycle。`upsertSessionPlan` 改为内部委托新工具的兼容别名。验证：新增/更新 `cargo test -p astrcode-adapter-tools builtin_tools::upsert_mode_artifact`，确认等价于现有 `upsertSessionPlan` 行为。
- [ ] 6.2 新增 `exit_mode.rs`，实现通用 `exitMode` 工具。读取当前 mode 的 `ModeExitGateDef`：无 exit_gate 时直接执行 mode transition；有 exit_gate 时执行 heading 校验 + review checkpoint。`exitPlanMode` 改为内部委托的兼容别名。验证：新增/更新 `cargo test -p astrcode-adapter-tools builtin_tools::exit_mode`，确认 heading 校验和 2-pass review 行为与现有 `exitPlanMode` 等价。
- [ ] 6.3 调整 `crates/adapter-tools/src/builtin_tools/enter_plan_mode.rs`，让 workflow state 初始化读取 mode 的 `workflow_binding` 字段而不是硬编码 `workflow_id = "plan_execute"`。验证：更新 `cargo test -p astrcode-adapter-tools builtin_tools::enter_plan_mode`。
- [ ] 6.4 在 `crates/application/src/session_plan.rs` 中引入通用 `build_mode_prompt_declarations(spec, artifact_state)`，由 `ModePromptHooks` 驱动 facts / reentry / template 逻辑。`build_plan_prompt_declarations()` 改为委托新函数。验证：更新 `cargo test -p astrcode-application session_plan`。
- [ ] 6.5 将 `build_plan_exit_declaration()` 和 `build_execute_bridge_declaration()` 的核心逻辑迁移为由 `exit_prompt` 字段和 `workflow_binding` 驱动。验证：更新相关测试确认 plan mode exit/bridge prompt 不变。

## 7. 回归验证

- [ ] 7.1 增加 selector 稳定性、plugin mode 注册（含新 contract 字段）、通用工具行为等价、workflow compile、reload 回滚与 prompt 来源追踪的回归测试。验证：`cargo test --workspace --exclude astrcode --lib`.
- [ ] 7.2 清理其他已经无用的代码路径或测试断言，确认没有残留对旧术语或旧行为的依赖
- [ ] 7.3 运行仓库级边界检查，确认治理/工作流改造没有破坏 crate 依赖方向。验证：`node scripts/check-crate-boundaries.mjs`.
