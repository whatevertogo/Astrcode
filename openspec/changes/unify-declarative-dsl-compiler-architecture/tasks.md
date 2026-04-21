## 1. 文档与契约对齐

- [x] 1.1 更新 `PROJECT_ARCHITECTURE.md` 与 `docs/architecture/declarative-dsl-compiler-target.md`，明确 `compile` / `bind` / `orchestrate` 术语、mode contract 边界、workflow artifact owner 与 governance snapshot 一致性约束。验证：人工审阅文档；`git diff --check`.
- [x] 1.2 盘点并更新相关 OpenSpec 主 spec 与实现注释中的旧术语，删除 `workflow_binding` 与 mixed-snapshot 的过时表述，避免继续把 `ResolvedTurnEnvelope` 和 `ResolvedGovernanceSurface` 混称为同一层结果。验证：`rg -n "ResolvedTurnEnvelope|ResolvedGovernanceSurface|workflow_binding|running turn.*old snapshot" openspec crates`.

## 2. 扩展 GovernanceModeSpec

- [x] 2.1a 在 `crates/core/src/mode/mod.rs` 新增 `ModeArtifactDef` 结构体（artifact_type, file_template, schema_template, required_headings, actionable_sections），补充序列化与校验。验证：新增 `cargo test -p astrcode-core mode::mode_artifact_def`.
- [x] 2.1b 新增 `ModeExitGateDef` 结构体（review_passes, review_checklist），补充序列化与校验。验证：新增 `cargo test -p astrcode-core mode::mode_exit_gate_def`.
- [x] 2.1c 新增 `ModePromptHooks` 结构体（reentry_prompt, initial_template, exit_prompt, facts_template），补充序列化与校验。验证：新增 `cargo test -p astrcode-core mode::mode_prompt_hooks`.
- [x] 2.1d 在 `GovernanceModeSpec` 上增加三个 `Option` 字段（artifact, exit_gate, prompt_hooks），扩展 `validate()` 递归校验新字段。验证：`cargo test -p astrcode-core mode`.
- [x] 2.2 调整 `crates/protocol/src/plugin/handshake.rs` 及其测试，确保插件通过 `InitializeResultData.modes` 声明扩展后的 mode contract 时仍保持纯 DTO 形状（字段可选，缺失时与旧行为等价）。验证：`cargo test -p astrcode-protocol plugin`.
- [x] 2.3 让 builtin `plan` mode 在 `crates/application/src/mode/catalog.rs` 中以新 mode contract 字段表达当前 artifact / exit / prompt 语义，而不是只靠工具名约定。验证：新增/更新 `cargo test -p astrcode-application mode::catalog`.
- [x] 2.4 为 `ModeCatalog` 增加 duplicate `mode_id` 检测，拒绝 plugin 覆盖 builtin mode 或多个 plugin 共享同一 `mode_id`。验证：新增/更新 `cargo test -p astrcode-application mode::catalog`.

## 3. 显式化治理 compile / bind 边界

- [x] 3.1 重构 `crates/application/src/mode/compiler.rs`，把 selector 求值、mode contract 派生、child/grant 裁剪与 diagnostics 明确收敛到编译阶段产物中。验证：新增/更新 `cargo test -p astrcode-application mode::compiler`.
- [x] 3.2 调整 `crates/application/src/governance_surface/assembler.rs` 与 `mod.rs`，把 runtime/profile/session/control 绑定责任与 compile 责任分开；必要时仅做渐进命名收束，不强求一次性全量改名。验证：新增/更新 `cargo test -p astrcode-application governance_surface`.
- [x] 3.3 为工具执行新增 pure-data `BoundModeToolContractSnapshot`（命名可渐进演化），并沿 `ResolvedGovernanceSurface -> AgentPromptSubmission -> ToolContext / CapabilityContext` 传递，禁止 `adapter-tools` 依赖 application 内部类型。验证：新增/更新 `cargo test -p astrcode-core tool`, `cargo test -p astrcode-kernel registry::tool`, `cargo test -p astrcode-session-runtime turn::submit`.
- [x] 3.4 收敛治理 prompt 来源，在 `crates/application/src/governance_surface/prompt.rs`、`crates/adapter-prompt/src/plan.rs`、`crates/adapter-prompt/src/block.rs` 之间保留单一 `PromptPlan` 结果模型，并补充来源 metadata。验证：`cargo test -p astrcode-adapter-prompt`.

## 4. workflow 轻量编译与 owner 收敛

- [x] 4.1 在 `crates/core/src/workflow.rs` 或 `crates/application/src/workflow/*` 中补充 workflow validate/compile 边界，使 workflow 在进入 orchestrator 前先完成显式校验。验证：新增/更新 `cargo test -p astrcode-application workflow`.
- [x] 4.2 调整 `crates/application/src/workflow/orchestrator.rs`，让 phase -> mode 绑定继续由 workflow artifact 的 `phase.mode_id` 持有，而不是反向从 mode spec 查 workflow binding。验证：新增/更新 `cargo test -p astrcode-application workflow::orchestrator`.
- [x] 4.3 把 plan workflow 的 bootstrap / approval / archive / bridge / reconcile 副作用从 `session_plan.rs`、`session_use_cases.rs` 的散落逻辑中收回到 `crates/application/src/workflow/*` 的统一 helper / service。验证：新增/更新 `cargo test -p astrcode-application workflow`.
- [x] 4.4 简化 `crates/adapter-tools/src/builtin_tools/enter_plan_mode.rs`，使其只负责 mode transition；workflow bootstrap 改由 application workflow orchestration 统一触发。验证：更新 `cargo test -p astrcode-adapter-tools builtin_tools::enter_plan_mode`。
- [x] 4.5 保持当前 workflow 数据结构克制，不引入与现有规模不匹配的索引化结构，同时补充对应注释与测试断言。验证：人工审阅实现；相关 workflow 单测通过。

## 5. reload 一致性与回滚

- [x] 5.1 重构 `crates/server/src/bootstrap/governance.rs`，把 mode catalog、capability surface、skill catalog 组织成统一候选治理快照，并在失败时完整回滚。验证：新增/更新 `cargo test -p astrcode-server bootstrap::governance`.
- [x] 5.2 调整 `crates/server/src/bootstrap/capabilities.rs` 与相关组合根逻辑，继续保持“存在 running session 时拒绝 reload”的治理合同，并删除 mixed-snapshot 假设。验证：新增/更新 `cargo test -p astrcode-server bootstrap::capabilities`.
- [x] 5.3 为 reload 成功/失败路径补充 observability 或日志诊断，能够说明 mode catalog / capability surface / skill catalog 的快照切换边界。验证：自动化测试或手动检查日志输出。

## 6. plan 合同清理

- [x] 6.1 在 `crates/application/src/session_plan.rs` 中引入 `build_mode_prompt_declarations(spec, artifact_state, workflow_facts)`，由 `ModePromptHooks` 驱动 facts / reentry / template / exit prompt 逻辑；`build_plan_prompt_declarations()` 改为委托新函数。验证：更新 `cargo test -p astrcode-application session_plan`.
- [x] 6.2 调整 `crates/adapter-tools/src/builtin_tools/upsert_session_plan.rs` 与 `exit_plan_mode.rs`，让它们通过 `BoundModeToolContractSnapshot` 读取 artifact / exit 合同，而不是继续硬编码 heading / checklist / writer 约束。验证：更新 `cargo test -p astrcode-adapter-tools builtin_tools::upsert_session_plan`, `cargo test -p astrcode-adapter-tools builtin_tools::exit_plan_mode`.
- [x] 6.3 更新 builtin `plan` mode prompt 与相关说明文案，移除对 `workflow_binding`、generic mode tool 和 mixed-snapshot 的错误假设。验证：新增/更新相关单测；`rg -n "workflow_binding|upsertModeArtifact|exitMode|running turn.*old snapshot" openspec crates`.

## 7. 回归验证

- [x] 7.1 增加 selector 稳定性、duplicate mode id 拒绝、plugin mode 注册（含新 contract 字段）、workflow compile / reconcile、reload 回滚、tool-contract bridge 与 prompt 来源追踪的回归测试。验证：`cargo test --workspace --exclude astrcode --lib`.
- [x] 7.2 清理其他已经无用的代码路径或测试断言，确认没有残留对旧术语、旧 owner 或旧假设的依赖。
- [x] 7.3 运行仓库级边界检查，确认治理 / 工作流改造没有破坏 crate 依赖方向。验证：`node scripts/check-crate-boundaries.mjs`.
