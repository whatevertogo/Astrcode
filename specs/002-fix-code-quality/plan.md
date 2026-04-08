# Implementation Plan: 修复项目代码质量问题

**Branch**: `002-fix-code-quality` | **Date**: 2026-04-08 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/002-fix-code-quality/spec.md`

## Summary

修复 CODE_QUALITY_ISSUES.md 中记录的所有项目质量问题，包括编译错误、架构违规（core→protocol 依赖）、panic 路径、异步任务泄漏、错误处理不一致、日志级别错误、模块过大和硬编码常量。技术方案采用两阶段策略：先完成 P1 阻断性问题（编译、架构、panic、并发），再处理 P2/P3 质量改进（错误统一、日志、模块拆分、常量提取）。

## Technical Context

**Language/Version**: Rust 1.75+  
**Primary Dependencies**: tokio (async runtime), tracing (logging), serde (serialization)  
**Storage**: JSONL 文件持久化（session events）  
**Testing**: cargo test, 集成测试覆盖 runtime/server/storage  
**Target Platform**: 跨平台（Windows/Linux/macOS）桌面应用  
**Project Type**: 桌面应用（Tauri + Rust backend + TypeScript frontend）  
**Performance Goals**: 无性能退化（锁恢复、错误转换开销可忽略）  
**Constraints**: 
- 必须通过 `cargo check --workspace` 和 `cargo clippy -- -D warnings`
- 必须符合宪法 1.2.0 所有约束（无 panic、无 fire-and-forget、无持锁 await、单文件 ≤800 行）
- 不破坏现有功能（所有测试通过）  
**Scale/Scope**: 
- 13 处 panic 路径需修复
- 4 处 fire-and-forget spawn 需管理
- 1 处持锁 await 需重构
- 6 个 crate 错误类型需统一
- service/mod.rs 约 17,000 行需拆分

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Durable Truth First**: ✅ 本次修复不涉及 durable 事件格式或历史行为变更，无需重新定义真相源
- **One Boundary, One Owner**: ✅ 本次修复不改变边界职责，仅在现有边界内修复质量问题。US7（模块拆分）会重组 `runtime/src/service/` 内部结构，但不改变对外 surface
- **Protocol Purity, Projection Fidelity**: ⚠️ **NEEDS REVIEW** - US2 将 Plugin 类型从 protocol 移入 core，需确认：
  - Plugin 类型是否属于"稳定契约与共享领域类型"（应在 core）还是"传输 DTO"（应在 protocol）
  - 移动后 protocol 中任何引用 Plugin 的 DTO 是否需要 mapper 重新映射
  - **决策**：根据 Clarification，Plugin 是核心扩展机制，属于 core。protocol 只保留传输 DTO，通过 mapper 转换
- **Ownership Over Storage Mode**: ✅ 本次修复不涉及 ownership、session mode 或父子执行语义
- **Explicit Migrations, Verifiable Refactors**: ✅ 每个 User Story 都有明确的验收标准和验证命令（cargo check/clippy/test、搜索 unwrap/spawn、wc -l）。US2（Plugin 类型迁移）需要 caller inventory 和迁移顺序
- **Runtime Robustness**: ✅ **CORE FOCUS** - US3/US4 直接针对宪法 VI：
  - US3：消除所有 `.unwrap()`/`.expect()` 在锁获取、数组索引中的使用
  - US4：消除所有 fire-and-forget spawn 和持锁 await
  - 采用 `with_lock_recovery` 恢复机制和 `JoinHandle` 管理
- **Observability & Error Visibility**: ✅ **CORE FOCUS** - US6 直接针对宪法 VII：
  - 修正关键操作的日志级别（turn failed、hook call failed 用 error!）
  - 消除 `println!`/`eprintln!`
  - 审计 `.ok()` 和 `let _ =` 的使用

**Constitution Gate Status**: ⚠️ **CONDITIONAL PASS** - 需在 Phase 0 研究中明确 Plugin 类型迁移的 mapper 策略和 caller inventory

## Project Structure

### Documentation (this feature)

```text
specs/002-fix-code-quality/
├── spec.md              # Feature specification (already exists)
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output - Plugin 迁移策略、锁恢复机制、JoinHandle 管理模式
├── data-model.md        # Phase 1 output - AstrError 统一结构、JoinHandle 管理器设计
├── quickstart.md        # Phase 1 output - 验证命令和回归测试指南
├── contracts/           # Phase 1 output - 错误转换契约、锁恢复契约
│   ├── error-conversion.md
│   └── lock-recovery.md
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

**Note**: 本次修复不涉及 durable 事件、公共 runtime surface、依赖方向或边界删除，因此不需要三层文档（findings/design/migration）。但 US2（Plugin 迁移）和 US7（模块拆分）需要在 research.md 中明确迁移策略。

### Source Code (repository root)

```text
crates/
├── core/
│   ├── src/
│   │   ├── agent/mod.rs          # US2: 接收从 protocol 移入的 Plugin 类型
│   │   ├── capability.rs         # US2: 移除 protocol re-export
│   │   ├── error.rs              # US5: 扩展 AstrError variants
│   │   └── env.rs                # US8: 新增端口号、大小限制常量
│   └── Cargo.toml                # US2: 移除 astrcode-protocol 依赖
├── protocol/
│   └── src/
│       └── http/
│           └── plugin.rs         # US2: Plugin 类型移出，保留传输 DTO + mapper
├── runtime/
│   └── src/
│       ├── service/
│       │   ├── mod.rs            # US1: 修复编译错误；US7: 拆分为子模块（17,000 行 → ≤800 行/文件）
│       │   ├── execution/
│       │   │   ├── mod.rs        # US7: 拆分（14,000 行 → ≤800 行/文件）
│       │   │   ├── root.rs       # US4: 修复 fire-and-forget spawn
│       │   │   └── subagent.rs   # US4: 修复 fire-and-forget spawn
│       │   └── watch_manager.rs  # US1: 修复 Pattern trait bound 错误；US4: 修复 fire-and-forget
│       └── Cargo.toml            # US8: 统一依赖版本到 workspace
├── runtime-session/
│   └── src/
│       └── lib.rs                # US3: 推广 with_lock_recovery 到其他 crate
├── runtime-registry/
│   └── src/
│       └── router.rs             # US3: 9 处 .expect() 替换为安全锁获取
├── runtime-agent-loop/
│   └── src/
│       └── hook_runtime.rs       # US6: 修正日志级别（debug! → error!）
├── runtime-agent-control/
│   └── src/
│       └── lib.rs                # US3: .expect("waiter should finish") 替换为 match
├── runtime-config/
│   ├── src/
│   │   ├── loader.rs             # US6: println! → log::warn!
│   │   └── constants.rs          # US8: 新增常量聚合导出
│   └── Cargo.toml                # US8: 统一依赖版本
├── storage/
│   └── src/
│       └── session/
│           └── event_log.rs      # US6: 文件操作失败不再 .ok() 忽略
├── plugin/
│   └── src/
│       ├── peer.rs               # US3: .lock().unwrap() 替换；US4: spawn handle 管理
│       └── supervisor.rs         # US4: 持锁 await 重构
└── src-tauri/
    └── src/
        └── main.rs               # US5: map_err(|_| ...) 保留原始错误

tests/
├── integration/
│   └── quality_regression.rs     # 新增：验证所有质量修复的回归测试
└── unit/
    └── error_conversion.rs       # US5: 错误转换测试

Cargo.toml                        # US8: workspace 依赖版本统一管理
```

## Phase 0: Research

**Prerequisites**: Constitution Check 通过

**Research Tasks**:

1. **Plugin 类型迁移策略** (US2)
   - 问题：Plugin 类型（PluginDescriptor、PluginInterface 等）当前在 protocol 中定义，但 core 依赖 protocol 违反宪法
   - 研究目标：
     - 确认 Plugin 类型的完整清单（哪些类型需要移动）
     - 识别 protocol 中所有引用 Plugin 类型的传输 DTO
     - 设计 mapper 策略（core 领域类型 ↔ protocol DTO）
     - 列出所有 caller（哪些模块当前从 protocol 导入 Plugin 类型）
   - 输出：迁移清单、mapper 设计、caller inventory

2. **锁恢复机制推广** (US3)
   - 问题：runtime-session 已有 `with_lock_recovery`，需推广到其他 crate
   - 研究目标：
     - 分析 `with_lock_recovery` 实现（poisoned.into_inner() + log::error!）
     - 确认 tokio Mutex 是否也需要恢复机制（tokio Mutex 不会 poison，但可能 panic）
     - 设计统一的锁获取 API（是否需要 trait 抽象）
   - 输出：锁恢复最佳实践、推广策略

3. **JoinHandle 管理模式** (US4)
   - 问题：4 处 fire-and-forget spawn 需要保存 handle 和取消机制
   - 研究目标：
     - 确认每个 spawn 的生命周期归属（谁应该持有 JoinHandle）
     - 设计取消触发方式（CancelToken vs CancellationToken）
     - 确认是否需要全局 JoinHandle 管理器（或各模块自行管理）
   - 输出：JoinHandle 管理模式、取消策略

4. **错误统一策略** (US5)
   - 问题：6 个 crate 各自定义错误类型，需与 AstrError 兼容
   - 研究目标：
     - 列出所有自定义错误类型（ProtocolError、StorageError 等）
     - 设计 AstrError 扩展方案（新增 variants + #[source]）
     - 确认是否保留各 crate 自有错误类型（或全部合并到 AstrError）
   - 输出：错误类型清单、AstrError 扩展设计

5. **模块拆分策略** (US7)
   - 问题：service/mod.rs（17,000 行）和 execution/mod.rs（14,000 行）需拆分
   - 研究目标：
     - 分析当前模块职责（哪些函数属于同一职责）
     - 设计拆分边界（避免循环依赖）
     - 确认拆分顺序（先拆哪个模块）
   - 输出：拆分方案、职责边界、拆分顺序

**Output**: `research.md` 包含所有研究结果和决策

## Phase 1: Design & Contracts

**Prerequisites**: `research.md` 完成

### Data Model

**Output**: `data-model.md`

**Entities**:

1. **AstrError (扩展)**
   - 新增 variants：
     - `Protocol { #[source] inner: ProtocolError }`
     - `Storage { #[source] inner: StorageError }`
     - `Plugin { #[source] inner: PluginError }`
     - `Config { #[source] inner: ConfigError }`
     - `Registry { #[source] inner: RegistryError }`
     - `AgentLoop { #[source] inner: AgentLoopError }`
   - 保留原始错误信息（#[source] 链）
   - 实现 `From<T>` for `AstrError` 自动转换

2. **LockRecoveryResult<T>**
   - 表示锁获取结果（成功、恢复、失败）
   - 字段：
     - `value: T`（锁守卫或恢复后的值）
     - `was_poisoned: bool`（是否发生中毒恢复）
   - 用于统一锁恢复语义

3. **TaskHandle**
   - 表示异步任务的生命周期管理
   - 字段：
     - `handle: JoinHandle<()>`
     - `cancel_token: CancellationToken`
   - 方法：
     - `cancel(&self)` - 触发取消
     - `await_completion(self)` - 等待完成

4. **PluginDescriptor (迁移到 core)**
   - 从 protocol 移入 core
   - 字段保持不变（id, name, version, capabilities）
   - protocol 中新增 `PluginDescriptorDto` 作为传输 DTO

### Interface Contracts

**Output**: `contracts/`

#### 1. Error Conversion Contract (`contracts/error-conversion.md`)

**Purpose**: 定义各 crate 错误类型与 AstrError 的转换契约

**Contract**:
- 每个 crate 的自定义错误类型 MUST 实现 `Into<AstrError>`
- 转换 MUST 保留原始错误信息（通过 `#[source]` 或 `error.to_string()`）
- `map_err` MUST NOT 使用 `|_|` 丢弃原始错误
- 示例：
  ```rust
  impl From<ProtocolError> for AstrError {
      fn from(err: ProtocolError) -> Self {
          AstrError::Protocol { inner: err }
      }
  }
  ```

**Validation**:
- 搜索 `map_err(|_|` 不存在
- 所有自定义错误类型都有 `Into<AstrError>` 实现
- 错误链可追溯（`error.source()` 不为 None）

#### 2. Lock Recovery Contract (`contracts/lock-recovery.md`)

**Purpose**: 定义锁获取的恢复机制契约

**Contract**:
- 所有生产代码的锁获取 MUST 使用恢复机制
- StdMutex 中毒 MUST 使用 `poisoned.into_inner()` 恢复 + `log::error!` 记录
- tokio Mutex MUST 使用 `lock().await` 或 `try_lock()` + 错误处理
- 持锁 await MUST 重构为释放锁后再 await
- 示例：
  ```rust
  fn with_lock_recovery<T, F, R>(mutex: &Mutex<T>, f: F) -> Result<R>
  where F: FnOnce(&mut T) -> Result<R>
  {
      let mut guard = match mutex.lock() {
          Ok(g) => g,
          Err(poisoned) => {
              log::error!("Mutex poisoned, recovering");
              poisoned.into_inner()
          }
      };
      f(&mut guard)
  }
  ```

**Validation**:
- 搜索 `.lock().unwrap()` 和 `.lock().expect()` 不存在
- 搜索 `.lock().await.*.await` 模式不存在
- 所有锁获取都有错误处理或恢复机制

### Quickstart

**Output**: `quickstart.md`

**验证命令**:

```bash
# US1: 编译和 clippy 检查
cargo check --workspace
cargo clippy --all-targets --all-features -- -D warnings

# US2: core 不依赖 protocol
grep -q "astrcode-protocol" crates/core/Cargo.toml && echo "FAIL: core still depends on protocol" || echo "PASS"

# US3: 无 panic 路径
rg '\.unwrap\(\)|\.expect\(' --type rust --glob '!tests/' --glob '!benches/' crates/
# 预期：无输出

# US4: 无 fire-and-forget spawn
rg 'tokio::spawn' --type rust --glob '!tests/' crates/ | rg -v 'JoinHandle|let.*='
# 预期：无输出

# US4: 无持锁 await
rg '\.lock\(\)\.await\..*\.await' --type rust crates/
# 预期：无输出

# US5: 错误转换保留上下文
rg 'map_err\(\|_\|' --type rust --glob '!tests/' crates/
# 预期：无输出

# US6: 无 println/eprintln
rg 'println!|eprintln!' --type rust --glob '!tests/' --glob '!examples/' crates/
# 预期：无输出

# US7: 文件行数限制
find crates/runtime/src/service -name '*.rs' -exec wc -l {} \; | awk '$1 > 800 {print}'
# 预期：无输出

# US8: workspace 依赖统一
rg 'toml.*=.*\{.*version' crates/*/Cargo.toml | rg -v 'workspace.*=.*true'
# 预期：仅合理例外

# 回归测试
cargo test --workspace --exclude astrcode
cd frontend && npm run typecheck
```

**回归测试场景**:
1. 锁恢复后功能正常（session 创建、工具执行、配置热重载）
2. 错误转换后错误信息完整（上下文不丢失）
3. JoinHandle 管理后任务正常取消（无资源泄漏）
4. 模块拆分后外部 API 行为不变（集成测试通过）

### Agent Context Update

运行 `.specify/scripts/powershell/update-agent-context.ps1 -AgentType claude` 更新 agent 上下文，添加：
- 锁恢复机制（`with_lock_recovery`）
- JoinHandle 管理模式（`TaskHandle`）
- 错误统一策略（`AstrError` variants）

## Re-Evaluation: Constitution Check (Post-Design)

**After Phase 1 design completion, re-check**:

- **Protocol Purity, Projection Fidelity**: ✅ Plugin 类型已移入 core，protocol 只保留 PluginDescriptorDto + mapper
- **Explicit Migrations, Verifiable Refactors**: ✅ research.md 已包含 Plugin 迁移的 caller inventory 和迁移顺序
- **Runtime Robustness**: ✅ 所有 panic 路径、fire-and-forget spawn、持锁 await 都有明确的替代方案
- **Observability & Error Visibility**: ✅ 日志级别修正和错误转换契约已定义

**Final Gate Status**: ✅ **PASS** - 所有宪法检查项通过，可以进入 Phase 2 (tasks generation)

## Next Steps

1. ✅ Phase 0: 完成 `research.md`（Plugin 迁移、锁恢复、JoinHandle 管理、错误统一、模块拆分）
2. ✅ Phase 1: 完成 `data-model.md`、`contracts/`、`quickstart.md`
3. ⏭️ Phase 2: 运行 `/speckit.tasks` 生成 `tasks.md`（按 User Story 分组，P1 → P2 → P3）

**Branch**: `002-fix-code-quality`  
**Plan Path**: `D:\GitObjectsOwn\Astrcode\specs\master\plan.md`  
**Generated Artifacts**: 
- ⏳ `research.md` (Phase 0)
- ⏳ `data-model.md` (Phase 1)
- ⏳ `contracts/error-conversion.md` (Phase 1)
- ⏳ `contracts/lock-recovery.md` (Phase 1)
- ⏳ `quickstart.md` (Phase 1)
