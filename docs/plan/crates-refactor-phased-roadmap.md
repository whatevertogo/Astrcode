# Crates 分阶段重构方案

## 目标

1. 恢复并固化分层边界，避免耦合继续扩散。
2. 拆解高复杂度模块，降低理解与维护成本。
3. 让 runtime 回归门面职责，子域实现可独立演进。
4. 用自动化规则防止架构回退。

## 周期建议

1. 快速版：2 周（P0 + 核心 P1）
2. 标准版：4 周（完整 P0/P1/P2）

---

## [x] 阶段 0：基线与护栏（2-3 天）

### 阶段目标

在开始重构前锁定行为基线，避免重构中出现“无感回归”。

### 主要动作

1. 产出当前 crates 依赖图（crate 级别）。
2. 补齐关键集成回归用例（会话执行、插件能力、prompt 构建）。
3. 在 CI 增加依赖边界检查脚本（先软失败，后硬失败）。

### 影响范围

1. 顶层工作流与脚本。
2. 集成测试目录与文档。

### 验收标准

1. 主流程回归全绿。
2. 每个 PR 可见依赖边界检查结果。

---

## [x] 阶段 1：修复 P0 编译隔离（3-5 天）

### 阶段目标

消除 runtime-prompt 对 runtime-skill-loader 的直接依赖，恢复 prompt 层编译隔离。

### 主要动作

1. 将 skill 元数据改为由上层注入（上下文输入），而不是 prompt 层直接读取 loader。
2. 调整 runtime 组装链路，统一注入 skill catalog / fingerprint。
3. 删除 runtime-prompt 对 runtime-skill-loader 的直接依赖声明。

### 影响范围

1. crates/runtime-prompt
2. crates/runtime
3. crates/runtime-execution

### 验收标准

1. runtime-prompt 可独立编译通过。
2. prompt 行为与现有产物保持一致。
3. 无新增循环依赖。

---

## [x]阶段 2：core 去实现化（4-6 天）

### 阶段目标

将 core 收敛为“契约与模型层”，移除具体路由实现。

### 主要动作

1. 在 core 仅保留 trait 与 DTO。
2. 将 CapabilityRouter / ToolRegistry 等具体实现迁移到 runtime 侧（建议独立 runtime-registry 子层或 crate）。
3. 在 runtime 层提供重导出，降低调用方迁移成本。

### 影响范围

1. crates/core/src/registry
2. crates/core/src/lib.rs
3. crates/runtime（新增或调整 registry 承载位置）

### 验收标准

1. core 不再包含路由具体实现。
2. runtime 侧承载路由实现并可独立演进。
3. 外部 crate 不出现大规模断裂式 API 变更。

---

## [ ] 阶段 3：RuntimeService 拆职责（1-1.5 周）

### 阶段目标

把 RuntimeService 从“全能服务”收敛为“门面编排”。

### 主要动作

1. 拆出 SessionService：会话生命周期与持久化。
2. 拆出 ExecutionService：turn 执行与重放。
3. 拆出 CapabilityManager：能力面替换、热重载。
4. 拆出 ConfigManager：配置 watcher 与重建触发。
5. RuntimeService 仅做编排与对外统一入口。

### 影响范围

1. crates/runtime/src/service/mod.rs
2. crates/runtime/src/service/session.rs
3. crates/runtime/src/service/execution.rs
4. crates/runtime/src/service/watch_ops.rs
5. crates/runtime/src/service/config_ops.rs

### 验收标准

1. RuntimeService 字段和公开方法显著减少。
2. 子服务具备独立单测。
3. 变更影响范围可被局部化。

---

## [ ]阶段 4：runtime-execution 与 agent-loop 降耦（1 周）

### 阶段目标

按子域拆分执行链路，降低依赖扇出与认知负担。

### 主要动作

1. runtime-execution 收敛为执行编排层，避免承担过多跨层依赖。
2. context pipeline / context window / compaction 独立为明确子域（可独立 crate 或强约束子模块）。
3. agent-loop 聚焦 orchestration，减少混合职责。

### 影响范围

1. crates/runtime-execution
2. crates/runtime-agent-loop

### 验收标准

1. runtime-execution 直接依赖数下降。
2. agent-loop 模块边界更清晰。
3. 上下文与压缩子域可独立测试。

---

## [ ]阶段 5：删除冗余、统一命名、文档固化（3-4 天）

### 阶段目标

将架构优化结果沉淀为长期可执行规则。

### 主要动作

1. 删除重复语义导出与历史兼容壳（确认无引用后）。
2. 统一模块命名语义（assembler / manager / service / executor）。
3. 更新架构文档与 crate 职责清单。
4. 将依赖边界检查从“提示”升级为“强阻断”。

### 影响范围

1. docs/architecture
2. docs/plan
3. AGENTS.md
4. CLAUDE.md

### 验收标准

1. 新增代码可快速判断归属 crate。
2. 越界依赖在 CI 被自动拦截。
3. 文档与实现保持一致。

---

## 每阶段统一验证命令

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
cargo check --workspace
```

## 风险与控制

1. 风险：阶段 2-4 容易引入 API 断裂。
   控制：引入兼容层，分批迁移调用点，再移除旧接口。
2. 风险：行为回归难发现。
   控制：阶段 0 先建立关键集成回归，后续阶段强制回归。
3. 风险：重构与需求并行导致分支漂移。
   控制：采用短分支、小步提交、阶段性合并。

## 推荐执行顺序

1. 先做阶段 1（收益高、风险低）。
2. 再做阶段 3（快速降低复杂度）。
3. 然后做阶段 2 与阶段 4（结构性清理）。
4. 最后阶段 5（收口与固化）。
