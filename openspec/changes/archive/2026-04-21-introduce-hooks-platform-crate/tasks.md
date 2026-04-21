## 1. 文档与范围收口

- [ ] 1.1 更新 `PROJECT_ARCHITECTURE.md`，把 `astrcode-hooks` 声明为独立平台 crate，并明确它与 `core`、`application`、`server`、plugin 协议层的职责边界；验证：文档回读，确认术语与本 change 的 proposal/design/specs 一致
- [ ] 1.2 收口并吸收 `openspec/changes/extract-governance-prompt-hooks/`，将其标记为被本 change 覆盖或迁入，避免双轨推进两套 hook 系统；验证：`rg -n "governance prompt hooks|extract-governance-prompt-hooks|BeforeTurnSubmit" openspec/changes`

## 2. `astrcode-hooks` crate 基础搭建

- [ ] 2.1 新建 `crates/hooks`，定义 `HookEvent`、`HookInput`、`HookEffect`、`HookMatcher`、`HookHandler`、`HookRegistry`、`HookRunner`、`HookExecutionReport` 等核心类型与模块结构；验证：`cargo check --workspace`
- [ ] 2.2 将当前 `crates/core/src/hook.rs` 收缩为极小兼容语义面，只保留必要的共享类型或再导出；完整的 registry、runner、matcher、report、schema 与执行语义全部落到 `crates/hooks`；验证：相关 crate 仍能编译，`rg -n "pub mod hook|HookHandler|HookOutcome" crates/core crates/hooks`
- [ ] 2.3 为第一阶段事件集实现 typed inputs：`SessionStart`、`SessionEnd`、`BeforeTurnSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`PermissionDenied`、`PreCompact`、`PostCompact`、`SubagentStart`、`SubagentStop`；验证：新增单元测试覆盖各事件序列化/反序列化
- [ ] 2.4 实现 hook point 分类与 event-scoped effect gating，明确 `observe`、`guard`、`augment` 三类 hook point 的 effect 边界，且默认不开放任意状态突变 effect；验证：新增单元测试覆盖 `PermissionRequest` 无法覆写 hard deny、`PreToolUse` 只能改写当前工具输入、`PostToolUse` 不能直接突变 workflow/session 真相等约束

## 3. Handler、Schema 与可观测性

- [ ] 3.1 在 `crates/hooks` 中实现第一阶段 handler 类型：`inline`、`command`、`http`，统一返回平台级执行结果与 diagnostics；验证：新增单元测试覆盖三类 handler 的成功、失败继续、失败中止语义
- [ ] 3.2 为 hooks 输入/输出补齐 schema 或等价稳定 wire shape，供 plugin/command/http handler 使用；验证：schema fixture 或序列化快照测试通过
- [ ] 3.3 实现结构化 `HookExecutionReport` 与 runner 报告聚合，记录事件名、handler 来源、handler 类型、effect 摘要、耗时与结果状态；验证：新增 runner 测试断言报告内容与顺序稳定

## 4. Application 生命周期接线

- [ ] 4.1 在 `crates/application` 中接入 hooks runner，并为 turn 提交建立统一 `BeforeTurnSubmit` 触发路径，使 root/session/subagent 入口都经由同一 turn-level hooks 解析；验证：`cargo test -p astrcode-application` 中新增或更新提交流程测试
- [ ] 4.2 将现有工具调用与 compact 流程改为消费 `astrcode-hooks` 的事件与 effect，而不是继续直接依赖 `core::hook` 窄版语义；验证：相关单元测试和回归测试通过
- [ ] 4.3 在权限裁决链路中接入 `PermissionRequest` / `PermissionDenied` hooks，并保证 effect 解释严格服从 governance / policy / capability surface 的硬边界；验证：新增权限集成测试，覆盖 ask/deny/continue 路径
- [ ] 4.4 在 subagent 生命周期边界接入 `SubagentStart` / `SubagentStop` hooks，并确保生命周期上下文不泄漏为 workflow 或 mode 真相；验证：新增子代理生命周期测试

## 5. Builtin hooks 迁移

- [ ] 5.1 将 `session_plan.rs` / `session_use_cases.rs` 中与 plan/workflow prompt overlay 相关的硬编码 helper 迁移为 builtin `BeforeTurnSubmit` hooks，统一走 `PromptDeclaration` 注入路径；验证：plan 初次进入、re-entry、approved exit、execute bridge 测试通过
- [ ] 5.2 调整 `governance_surface` 组装逻辑，消费 hook 产出的 prompt declarations / system messages，同时拒绝任何越过治理边界的 effect；验证：新增 governance surface 集成测试
- [ ] 5.3 确保 workflow 只向 hooks 提供已解析的 phase truth 和 bridge context，hooks 不直接决定 signal、transition 或恢复策略；验证：workflow 损坏降级测试与 phase overlay 测试通过

## 6. Plugin 与 Reload 集成

- [ ] 6.1 更新 plugin hook 物化路径，使 plugin 声明的 hooks 不再直接适配 `core::HookHandler`，而是注册为 hooks 平台 external handlers；验证：相关 plugin 集成测试通过
- [ ] 6.2 扩展 `server` bootstrap / reload，使 hooks registry 与 capability surface、skill catalog、mode catalog 一起参与候选快照、提交与回滚；验证：新增 reload 失败回滚测试，确认不会出现半刷新状态
- [ ] 6.3 为 builtin hooks 与 plugin hooks 的统一注册、冲突处理和 diagnostics 暴露增加可观测性输出；验证：人工检查日志/collector 输出或新增测试断言

## 7. 清理与验证

- [ ] 7.1 清理旧的 `core::hook` 专属调用点、废弃 helper 与重复抽象，确保 `core` 不再拥有 hooks 平台运行时，只保留最小兼容壳层并标明退出路径；验证：`rg -n "core::hook|build_plan_prompt_declarations|build_plan_exit_declaration|build_execute_bridge_declaration" crates`
- [ ] 7.2 运行直接相关的编译、测试与架构边界校验，确认新 crate 与依赖方向正确；验证：`cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib`、`node scripts/check-crate-boundaries.mjs`
- [ ] 7.3 回读 `openspec/changes/introduce-hooks-platform-crate/` 全部 artifacts，确认 proposal/design/specs/tasks 与最终代码边界一致，并为后续 mode change 预留干净依赖点；验证：人工审阅并补充必要注释/文档
