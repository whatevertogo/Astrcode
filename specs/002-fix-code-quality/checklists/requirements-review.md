# Checklist: Requirements Review - Code Quality Fix

**Purpose**: 实施前需求审查 - 验证 spec 是否完整、清晰、可测，可以开始实施

**Created**: 2026-04-08

**Feature**: 002-fix-code-quality

**Scope**: 所有优先级（P1-P3），覆盖需求完整性、验收标准可测性、架构约束一致性、实施风险

---

## Requirement Completeness

- [ ] CHK001 - 是否所有 CODE_QUALITY_ISSUES.md 中记录的编译错误都有对应的需求和验收场景？ [Completeness, Gap]
- [ ] CHK002 - 是否所有 clippy 警告类型（unused import、dead code 等）都有明确的修复需求？ [Completeness, Spec §FR-002]
- [ ] CHK003 - Plugin 类型从 protocol 移入 core 后，是否定义了所有受影响的传输 DTO 的 mapper 重构需求？ [Completeness, Edge Case]
- [ ] CHK004 - 是否明确定义了 13 处 panic 路径的具体位置和替代方案？ [Completeness, Spec §US3]
- [ ] CHK005 - 是否所有 4 处 fire-and-forget spawn 和 1 处持锁 await 的位置都有明确的修复需求？ [Completeness, Spec §US4]
- [ ] CHK006 - 6 个 crate 的自定义错误类型是否都有明确的统一策略？ [Completeness, Spec §US5]
- [ ] CHK007 - 是否定义了所有需要修正日志级别的具体位置（hook_runtime.rs、coordinator.rs 等）？ [Completeness, Spec §US6]
- [ ] CHK008 - service/mod.rs（17,000 行）和 execution/mod.rs（14,000 行）的拆分策略是否定义了具体的职责边界？ [Completeness, Spec §US7]
- [ ] CHK009 - 是否列出了所有需要提取为常量的硬编码值（62000、128000、20000、200000）的位置？ [Completeness, Spec §US8]
- [ ] CHK010 - 是否定义了所有需要统一到 workspace 的依赖（toml、tracing、async-stream、tower）？ [Completeness, Spec §US8]

## Requirement Clarity

- [ ] CHK011 - "锁获取恢复机制"是否明确定义了 StdMutex 和 tokio Mutex 的具体恢复策略？ [Clarity, Spec §FR-005, Clarification]
- [ ] CHK012 - "安全数组索引"是否明确定义了使用 `.get()` 还是前置断言的选择标准？ [Clarity, Spec §FR-006]
- [ ] CHK013 - "JoinHandle 管理和取消机制"是否明确定义了保存位置（结构体字段）和取消触发方式（CancelToken）？ [Clarity, Spec §FR-007, Clarification]
- [ ] CHK014 - "持锁 await"的识别模式是否明确（`.lock().await.*.await`）？ [Clarity, Spec §FR-008]
- [ ] CHK015 - "错误类型兼容"是否明确定义了使用 `Into<AstrError>` 还是 `#[source]` 的选择标准？ [Clarity, Spec §FR-009, Clarification]
- [ ] CHK016 - "关键操作"的定义是否明确（turn failed、hook call failed 等）？ [Clarity, Spec §FR-011]
- [ ] CHK017 - "两阶段拆分"的阶段边界是否明确（US1-US4 完成并稳定后执行 US7）？ [Clarity, Spec §US7]
- [ ] CHK018 - 硬编码常量的提取位置是否明确（`core/src/env.rs` vs `runtime-config/src/constants.rs`）？ [Clarity, Spec §FR-015]

## Requirement Consistency

- [ ] CHK019 - FR-005（锁获取恢复机制）与 Clarification 中的 `poisoned.into_inner()` 策略是否一致？ [Consistency, Spec §FR-005]
- [ ] CHK020 - US3 的验收场景与 FR-005/FR-006 的要求是否一致（锁获取、数组索引）？ [Consistency, Spec §US3]
- [ ] CHK021 - US4 的验收场景与 FR-007/FR-008 的要求是否一致（JoinHandle 管理、持锁 await）？ [Consistency, Spec §US4]
- [ ] CHK022 - US5 的错误统一策略与 Clarification 中的 `#[source]` 方案是否一致？ [Consistency, Spec §US5]
- [ ] CHK023 - US7 的两阶段拆分与 Edge Cases 中的"避免同时做两件大事"是否一致？ [Consistency, Spec §US7]
- [ ] CHK024 - FR-003（core 不依赖 protocol）与 FR-004（Plugin 类型移入 core）是否一致？ [Consistency, Spec §FR-003, §FR-004]

## Acceptance Criteria Quality

- [ ] CHK025 - US1 的验收标准是否可客观验证（`cargo check` 和 `cargo clippy` 通过）？ [Measurability, Spec §US1]
- [ ] CHK026 - US2 的验收标准是否可客观验证（检查 `core/Cargo.toml` 不包含 `astrcode-protocol`）？ [Measurability, Spec §US2]
- [ ] CHK027 - US3 的验收标准是否可客观验证（搜索 `.unwrap()` / `.expect()` 数量为 0）？ [Measurability, Spec §US3]
- [ ] CHK028 - US4 的验收标准是否可客观验证（搜索 fire-and-forget spawn 数量为 0）？ [Measurability, Spec §US4]
- [ ] CHK029 - US5 的验收标准是否可客观验证（错误类型实现 `Into<AstrError>` 或 `#[source]`）？ [Measurability, Spec §US5]
- [ ] CHK030 - US6 的验收标准是否可客观验证（搜索 `debug!` 在关键操作中的使用）？ [Measurability, Spec §US6]
- [ ] CHK031 - US7 的验收标准是否可客观验证（`wc -l` 统计所有文件不超过 800 行）？ [Measurability, Spec §US7]
- [ ] CHK032 - US8 的验收标准是否可客观验证（搜索硬编码常量、检查 workspace 依赖）？ [Measurability, Spec §US8]
- [ ] CHK033 - SC-001 到 SC-007 是否都有对应的自动化验证命令？ [Measurability, Success Criteria]

## Scenario Coverage

- [ ] CHK034 - 是否定义了编译错误修复后的回归测试需求（确保修复不引入新错误）？ [Coverage, Primary Flow]
- [ ] CHK035 - 是否定义了 Plugin 类型迁移后的兼容性测试需求（protocol DTO mapper 正确性）？ [Coverage, Alternate Flow]
- [ ] CHK036 - 是否定义了锁获取恢复机制的异常场景需求（锁中毒、恢复失败）？ [Coverage, Exception Flow]
- [ ] CHK037 - 是否定义了 JoinHandle 取消机制的异常场景需求（任务已完成、取消失败）？ [Coverage, Exception Flow]
- [ ] CHK038 - 是否定义了错误转换链路的异常场景需求（source chain 断裂、上下文丢失）？ [Coverage, Exception Flow]
- [ ] CHK039 - 是否定义了 service 模块拆分后的集成测试需求（模块间调用关系正确）？ [Coverage, Primary Flow]
- [ ] CHK040 - 是否定义了所有修复后的性能影响评估需求（锁恢复、错误转换开销）？ [Coverage, Non-Functional]

## Edge Case Coverage

- [ ] CHK041 - 是否定义了 Plugin 类型迁移时 protocol 中任何引用这些类型的 DTO 的处理需求？ [Edge Case, Spec Edge Cases]
- [ ] CHK042 - 是否定义了 service/mod.rs 拆分时避免循环依赖的检查需求？ [Edge Case, Spec Edge Cases]
- [ ] CHK043 - 是否定义了 `with_lock_recovery` 在所有场景下的正确性验证需求？ [Edge Case, Spec Edge Cases]
- [ ] CHK044 - 是否定义了锁获取失败时的降级策略（如果恢复也失败）？ [Edge Case, Gap]
- [ ] CHK045 - 是否定义了异步任务取消时的资源清理需求（未完成的 I/O、持有的锁）？ [Edge Case, Gap]
- [ ] CHK046 - 是否定义了错误转换时的循环引用检测需求（AstrError 包含自身）？ [Edge Case, Gap]
- [ ] CHK047 - 是否定义了日志级别修正后的日志量影响评估需求（从 debug 到 error 可能增加日志量）？ [Edge Case, Gap]
- [ ] CHK048 - 是否定义了常量提取后的跨 crate 引用一致性需求（避免重复定义）？ [Edge Case, Gap]

## Non-Functional Requirements

- [ ] CHK049 - 是否定义了所有修复的性能影响阈值（锁恢复、错误转换不应显著降低性能）？ [Non-Functional, Gap]
- [ ] CHK050 - 是否定义了锁恢复机制的并发安全性验证需求？ [Non-Functional, Security]
- [ ] CHK051 - 是否定义了错误日志的敏感信息过滤需求（避免泄露内部状态）？ [Non-Functional, Security]
- [ ] CHK052 - 是否定义了 service 模块拆分后的编译时间影响评估需求？ [Non-Functional, Gap]
- [ ] CHK053 - 是否定义了所有修复的内存影响评估需求（JoinHandle 保存、错误 source chain）？ [Non-Functional, Gap]

## Dependencies & Assumptions

- [ ] CHK054 - 是否验证了 `with_lock_recovery` 机制在所有 crate 中都可用的假设？ [Assumption, Spec Assumptions]
- [ ] CHK055 - 是否验证了 service 模块拆分不改变外部 API 行为的假设？ [Assumption, Spec Assumptions]
- [ ] CHK056 - 是否验证了 800 行限制可以临时放宽的假设（是否有 CI 检查）？ [Assumption, Spec Assumptions]
- [ ] CHK057 - 是否明确了 US7（模块拆分）对 US1-US4 完成的依赖关系？ [Dependency, Spec §US7]
- [ ] CHK058 - 是否明确了 Plugin 类型迁移对现有 protocol DTO 的影响范围？ [Dependency, Gap]
- [ ] CHK059 - 是否明确了错误统一对现有错误处理代码的影响范围？ [Dependency, Gap]
- [ ] CHK060 - 是否明确了 JoinHandle 管理对现有异步任务生命周期的影响？ [Dependency, Gap]

## Ambiguities & Conflicts

- [ ] CHK061 - "生产代码"的定义是否明确（是否包括 examples、benches）？ [Ambiguity, Spec §FR-005]
- [ ] CHK062 - "关键操作失败"与"非关键操作失败"的边界是否明确？ [Ambiguity, Spec §FR-011]
- [ ] CHK063 - `.ok()` 和 `let _ =` 的"注释说明原因"是否有具体的注释格式要求？ [Ambiguity, Spec §FR-013]
- [ ] CHK064 - "统一使用 workspace 继承"是否有例外情况（如特定 crate 需要不同版本）？ [Ambiguity, Spec §FR-016]
- [ ] CHK065 - US7 的"稳定"标准是否明确（多少天无 bug、多少测试通过）？ [Ambiguity, Spec §US7]
- [ ] CHK066 - FR-005（锁获取恢复）与 FR-008（持锁 await）是否有冲突（恢复后仍持锁 await）？ [Conflict, Gap]
- [ ] CHK067 - FR-014（800 行限制）与 Assumptions 中的"临时放宽"是否有冲突？ [Conflict, Spec §FR-014]

## Architecture Constraints Alignment

- [ ] CHK068 - FR-003（core 不依赖 protocol）是否与宪法 1.2.0 Architecture Constraints 的双向独立约束对齐？ [Consistency, Constitution]
- [ ] CHK069 - FR-005/FR-006（消除 panic）是否与宪法 VI Runtime Robustness 的禁止 panic 约束对齐？ [Consistency, Constitution]
- [ ] CHK070 - FR-007/FR-008（异步任务管理）是否与宪法 VI 的并发安全约束对齐？ [Consistency, Constitution]
- [ ] CHK071 - FR-011/FR-012（日志级别）是否与宪法 VII Observability 的日志要求对齐？ [Consistency, Constitution]
- [ ] CHK072 - FR-014（800 行限制）是否与宪法 1.2.0 Architecture Constraints 的量化约束对齐？ [Consistency, Constitution]
- [ ] CHK073 - 是否所有宪法约束都有对应的需求（是否有遗漏的宪法约束）？ [Completeness, Constitution]

## Implementation Risk

- [ ] CHK074 - Plugin 类型迁移是否可能破坏现有的 protocol 序列化/反序列化？ [Risk, Spec §US2]
- [ ] CHK075 - 锁恢复机制是否可能引入新的并发安全问题（恢复后状态不一致）？ [Risk, Spec §US3]
- [ ] CHK076 - JoinHandle 保存是否可能引入内存泄漏（handle 未正确清理）？ [Risk, Spec §US4]
- [ ] CHK077 - 错误统一是否可能破坏现有的错误处理逻辑（错误类型不匹配）？ [Risk, Spec §US5]
- [ ] CHK078 - 日志级别修正是否可能引入日志洪水（大量 error 日志）？ [Risk, Spec §US6]
- [ ] CHK079 - service 模块拆分是否可能引入新的循环依赖？ [Risk, Spec §US7, Edge Case]
- [ ] CHK080 - 常量提取是否可能引入跨 crate 的编译依赖问题？ [Risk, Spec §US8]
- [ ] CHK081 - 是否定义了所有修复的回滚策略（如果修复引入新问题）？ [Risk, Gap]

## Traceability

- [ ] CHK082 - 是否所有 User Story 都有明确的 FR 映射？ [Traceability, Gap]
- [ ] CHK083 - 是否所有 FR 都有明确的 SC 映射？ [Traceability, Gap]
- [ ] CHK084 - 是否所有 Edge Cases 都有对应的需求或验收场景？ [Traceability, Spec Edge Cases]
- [ ] CHK085 - 是否所有 Clarifications 都已反映到对应的需求中？ [Traceability, Spec Clarifications]
- [ ] CHK086 - 是否建立了需求 ID 与 CODE_QUALITY_ISSUES.md 中问题的映射？ [Traceability, Gap]

---

**Total Items**: 86

**Coverage**:
- Requirement Completeness: 10 items
- Requirement Clarity: 8 items
- Requirement Consistency: 6 items
- Acceptance Criteria Quality: 9 items
- Scenario Coverage: 7 items
- Edge Case Coverage: 8 items
- Non-Functional Requirements: 5 items
- Dependencies & Assumptions: 7 items
- Ambiguities & Conflicts: 7 items
- Architecture Constraints Alignment: 6 items
- Implementation Risk: 8 items
- Traceability: 5 items

**Focus Areas**: 需求完整性、验收标准可测性、架构约束一致性、实施风险

**Depth**: 标准深度，覆盖所有质量维度

**Audience**: 实施前审查（作者 + 评审者）

**Timing**: 开始实施前
