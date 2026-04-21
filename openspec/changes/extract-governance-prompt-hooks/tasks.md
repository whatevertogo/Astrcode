## 1. 契约与架构文档

- [ ] 1.1 更新 `PROJECT_ARCHITECTURE.md` 或对应架构文档，明确 `core::HookHandler` 与 `governance prompt hooks` 的职责边界，并标注其都位于何层消费；验证：文档回读，确保术语与 `openspec/changes/extract-governance-prompt-hooks/*.md` 一致
- [ ] 1.2 审查并同步 `openspec/changes/unify-declarative-dsl-compiler-architecture/` 中与 prompt hook 迁移直接冲突的描述，改成依赖本 change，而不是在 mode change 内重复实现；验证：`rg -n "prompt hook|prompt_hooks|build_plan_prompt_declarations|build_plan_exit_declaration" openspec/changes/unify-declarative-dsl-compiler-architecture`

## 2. Governance Prompt Hooks 基础模块

- [ ] 2.1 在 `crates/application/src/` 下新增 `prompt_hooks/` 模块，定义 typed input、hook trait、resolver 和 builtin registration 入口；验证：`cargo check --workspace`
- [ ] 2.2 为 `ModeActive`、`ModeExit`、`WorkflowPhaseOverlay` 三类输入建立最小 typed context，确保 resolver 不依赖隐藏 I/O；验证：新增单元测试覆盖 hook 匹配与 resolver 顺序
- [ ] 2.3 为 resolver 增加稳定顺序与 diagnostics 结构，保证同一输入重复解析得到等价 declaration 顺序；验证：新增 resolver 单测

## 3. builtin plan / workflow prompt 迁移

- [ ] 3.1 将 `crates/application/src/session_plan.rs` 中的 `build_plan_prompt_declarations` 迁移为 builtin `ModeActive` hook provider，只保留 plan artifact / workflow truth 与 prompt context 计算；验证：原有 plan prompt 相关测试迁移后继续通过
- [ ] 3.2 将 `build_plan_exit_declaration` 迁移为 builtin `ModeExit` hook provider，保持 approved plan overlay 内容与字段等价；验证：新增或更新测试，断言 declaration `origin` 与内容字段保持预期
- [ ] 3.3 将 `build_execute_bridge_declaration` 迁移为 builtin `WorkflowPhaseOverlay` hook provider，确保 executing phase 的 bridge prompt 仍包含 approved plan 引用与 implementation steps；验证：新增或更新 bridge prompt 单测

## 4. Session 提交流程接线

- [ ] 4.1 重构 `crates/application/src/session_use_cases.rs`，把 mode-only 与 active-workflow 提交流程中的 prompt 拼装改为“准备 typed context -> 调用 resolver -> 注入 `extra_prompt_declarations`”；验证：`cargo test --workspace --exclude astrcode --lib session_use_cases`
- [ ] 4.2 保持 corrupted / semantically invalid workflow state 的既有降级行为，确保 fallback 发生在 hook 解析之前，而不是由 hook 决定；验证：现有 workflow 降级测试通过
- [ ] 4.3 清理提交流程中直接依赖 plan-specific prompt helper 的条件分支与死代码，保证主流程只保留 orchestration 与 state transition 责任；验证：`rg -n "build_plan_prompt_declarations|build_plan_exit_declaration|build_execute_bridge_declaration" crates/application/src`

## 5. 回归验证与后续衔接

- [ ] 5.1 补齐 `crates/application` 侧测试，覆盖 plan 初次进入、plan re-entry、approved exit、executing bridge、planning/executing phase 隔离、resolver 顺序与非匹配 hook 静默行为；验证：`cargo test -p astrcode-application prompt_hooks session_use_cases`
- [ ] 5.2 运行直接相关的整体校验，确认 hooks refactor 未破坏架构边界或编译；验证：`cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib`、`node scripts/check-crate-boundaries.mjs`
- [ ] 5.3 回读并清理遗留命名与注释，确保 `session_plan` 不再承担 prompt 组装职责，后续 mode change 可以直接依赖新 hook 模块；验证：人工审阅 `crates/application/src/session_plan.rs`、`crates/application/src/session_use_cases.rs`、`crates/application/src/prompt_hooks/`
