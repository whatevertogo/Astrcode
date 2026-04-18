## 1. Core 类型基础

- [ ] 1.1 在 `crates/core/src/mode/mod.rs` 创建 mode 模块，定义 `CollaborationMode` 枚举（Plan/Execute/Review）、`ModeEntryPolicy`、`ModeTransition`、`ArtifactStatus`、`RenderLevel`、`ModeTransitionSource`
- [ ] 1.2 在 `crates/core/src/mode/mod.rs` 定义 `ToolGrantRule` 枚举（Named/SideEffect/All）
- [ ] 1.3 在 `crates/core/src/mode/mod.rs` 定义 `ModeSpec` 结构体
- [ ] 1.4 在 `crates/core/src/mode/mod.rs` 定义 `PlanContent`、`ReviewContent`、`ModeArtifactBody`、`ModeArtifactRef` 结构体
- [ ] 1.5 在 `crates/core/src/mode/mod.rs` 定义 `ArtifactRenderer` trait 和 `ModeCatalog` trait
- [ ] 1.6 在 `crates/core/src/lib.rs` 注册 mode 模块并 re-export 公共类型
- [ ] 1.7 在 `crates/core/src/policy/engine.rs` 的 `StorageEventPayload` 中新增 `ModeChanged`、`ModeArtifactCreated`、`ModeArtifactStatusChanged` 变体

**验证:** `cargo check -p astrcode-core` 通过

## 2. Session-Runtime 模式真相

- [ ] 2.1 在 `crates/session-runtime/src/state/mod.rs` 的 `SessionState` 中新增 `session_mode: StdMutex<CollaborationMode>` 字段和 `current_mode()` / `set_mode()` 方法
- [ ] 2.2 在 `crates/session-runtime/src/state/mod.rs` 的 `SessionState` 中新增 `active_artifacts: StdMutex<Vec<ModeArtifactRef>>` 字段和相关查询方法
- [ ] 2.3 更新 `SessionState::new()` 初始化新字段为默认值（Execute / 空 vec）
- [ ] 2.4 在 `EventTranslator` 中处理 `ModeChanged` 和 `ModeArtifactCreated` / `ModeArtifactStatusChanged` 事件的投影逻辑

**验证:** `cargo check -p astrcode-session-runtime` 通过

## 3. 模式编译与工具授予

- [ ] 3.1 在 `crates/session-runtime/src/turn/` 下新增 `mode_compile.rs`，实现 `compile_mode_spec()` 函数：从 ModeSpec + CapabilitySpec 列表编译出 visible tools + prompt declarations
- [ ] 3.2 实现 `ToolGrantRule` 到 `Vec<String>` 的解析逻辑，处理 Named/SideEffect/All 三种规则
- [ ] 3.3 实现 `ModeMap` prompt block 生成（从 ModeCatalog 生成 SemiStable 层 declaration）
- [ ] 3.4 实现 `CurrentMode` prompt block 生成（Dynamic 层 declaration，包含当前约束）
- [ ] 3.5 修改 `crates/session-runtime/src/turn/runner.rs` 的 `TurnExecutionResources::new()`，从 session_mode 编译 visible_tools 替换直接读 gateway 的工具列表
- [ ] 3.6 在 `AssemblePromptRequest` 中新增 mode 相关字段，在 `assemble_prompt_request()` 中注入 ModeMap 和 CurrentMode prompt declarations
- [ ] 3.7 为 compile_mode_spec 编写单元测试：Plan 模式只拿只读工具、Execute 拿全部、Review 拿只读

**验证:** `cargo test -p astrcode-session-runtime -- mode_compile` 通过

## 4. 统一模式切换入口

- [ ] 4.1 在 `crates/session-runtime/src/turn/` 下新增 `mode_transition.rs`，实现 `apply_mode_transition()` 函数
- [ ] 4.2 实现转换合法性验证（检查 target_mode 是否在当前 mode 的 transitions 列表中）
- [ ] 4.3 实现 entry_policy 检查逻辑（LlmCanEnter/UserOnly/LlmSuggestWithConfirmation）
- [ ] 4.4 实现 transition requires_confirmation 检查逻辑
- [ ] 4.5 实现切换执行：更新 session_mode + 广播 ModeChanged StorageEvent
- [ ] 4.6 为 apply_mode_transition 编写单元测试：合法切换、非法切换、entry_policy 拒绝、User 绕过

**验证:** `cargo test -p astrcode-session-runtime -- mode_transition` 通过

## 5. switchMode Builtin Tool

- [ ] 5.1 在 `crates/adapter-tools/src/builtin_tools/` 下新增 `switch_mode.rs`，实现 switchMode tool 的参数解析和执行逻辑
- [ ] 5.2 switchMode 执行体调用 `apply_mode_transition(source=Tool)`，返回切换结果
- [ ] 5.3 在 `crates/adapter-tools/src/builtin_tools/mod.rs` 注册 switchMode tool
- [ ] 5.4 switchMode 的 CapabilitySpec 标注为 `side_effect: None`（所有模式都可见）
- [ ] 5.5 编写 switchMode tool 的单元测试：成功切换、拒绝切换、未知模式

**验证:** `cargo test -p astrcode-adapter-tools -- switch_mode` 通过

## 6. BuiltinModeCatalog 注册

- [ ] 6.1 在 `crates/application/src/execution/` 下新增 `mode_catalog.rs`，实现 `BuiltinModeCatalog` 结构体
- [ ] 6.2 定义 Plan/Execute/Review 三个 ModeSpec 实例（含 tool_grants、system_directive、entry_policy、transitions）
- [ ] 6.3 在 `crates/application/src/lib.rs` 注册 mode_catalog 模块
- [ ] 6.4 在 `crates/server/src/bootstrap/runtime.rs` 的 bootstrap 阶段创建 BuiltinModeCatalog 并注入到 PromptFactsProvider
- [ ] 6.5 编写 BuiltinModeCatalog 的单元测试：list_modes 返回 3 个、resolve_mode 正确

**验证:** `cargo check --workspace` 通过

## 7. /mode Command 入口

- [ ] 7.1 在 session-runtime 的 command 处理中新增 `/mode` 命令解析
- [ ] 7.2 `/mode <name>` 调用 `apply_mode_transition(source=User)`，绕过 entry_policy
- [ ] 7.3 `/mode` 不带参数返回当前模式名称和描述
- [ ] 7.4 编写 /mode command 的单元测试

**验证:** `cargo test -p astrcode-session-runtime -- mode_command` 通过

## 8. ModeArtifact 集成

- [ ] 8.1 在 `crates/core/src/mode/mod.rs` 实现 Builtin 的 `PlanArtifactRenderer`（Summary/Compact/Full 三级渲染）
- [ ] 8.2 在 `crates/session-runtime/src/state/` 下新增 artifact 管理方法：create_artifact、accept_artifact、reject_artifact、supersede_artifact
- [ ] 8.3 在 `crates/session-runtime/src/turn/mode_compile.rs` 的 Execute 模式编译中，查找 accepted plan artifact 并注入 Full 级渲染的 PromptDeclaration
- [ ] 8.4 编写 artifact 管理的单元测试：创建、接受、supersede 流程
- [ ] 8.5 编写 artifact prompt injection 的单元测试

**验证:** `cargo test -p astrcode-session-runtime -- artifact` 通过

## 9. PromptFacts 集成

- [ ] 9.1 在 `crates/server/src/bootstrap/prompt_facts.rs` 中集成 ModeCatalog，使 PromptFacts 包含 mode 信息
- [ ] 9.2 在 `crates/adapter-prompt/src/contributors/` 下新增 mode 相关 contributor（生成 ModeMap block）
- [ ] 9.3 确保 ModeMap block 和 CurrentMode block 的缓存层正确（SemiStable / Dynamic）

**验证:** `cargo check --workspace` 通过

## 10. 集成验证

- [ ] 10.1 运行 `cargo fmt --all` 格式化代码
- [ ] 10.2 运行 `cargo clippy --all-targets --all-features -- -D warnings` 修复所有警告
- [ ] 10.3 运行 `cargo test --workspace --exclude astrcode` 确保所有测试通过
- [ ] 10.4 运行 `node scripts/check-crate-boundaries.mjs` 验证 crate 依赖边界
- [ ] 10.5 端到端手动验证：启动 dev server，使用 /mode plan 切换模式，确认 LLM 只使用只读工具
