# Quickstart: 修复项目代码质量问题

**Feature**: 002-fix-code-quality  
**Date**: 2026-04-08  
**Status**: Complete

本文档提供快速验证和回归测试指南。

---

## 验证命令

### US1: 编译和 Clippy 检查

```bash
# 编译检查（无错误）
cargo check --workspace

# Clippy 检查（无警告）
cargo clippy --all-targets --all-features -- -D warnings
```

**预期结果**：
- `cargo check` 通过，无编译错误
- `cargo clippy` 通过，无警告

**失败处理**：
- 编译错误：检查 `watch_ops.rs:332` 的 Pattern trait bound 修复
- Clippy 警告：检查 `filter.rs:3` 的 unused import 移除

---

### US2: Core 不依赖 Protocol

```bash
# 检查 core/Cargo.toml 不包含 astrcode-protocol 依赖
grep -q "astrcode-protocol" crates/core/Cargo.toml && echo "FAIL: core still depends on protocol" || echo "PASS: core is independent"
```

**预期结果**：`PASS: core is independent`

**失败处理**：
- 检查 Plugin 类型是否已完全迁移到 core
- 检查 core 中是否还有 protocol re-export

---

### US3: 无 Panic 路径

```bash
# 搜索生产代码中的 .unwrap() 和 .expect()
rg '\.unwrap\(\)|\.expect\(' --type rust --glob '!tests/' --glob '!benches/' crates/

# 预期：无输出（或仅测试代码）
```

**预期结果**：无输出

**失败处理**：
- 检查锁获取是否使用 `with_lock_recovery()` 或 `lock_anyhow()`
- 检查数组索引是否使用 `.get()` 或前置断言
- 检查 timeout 等待是否使用 `match` 处理

---

### US4: 无 Fire-and-Forget Spawn

```bash
# 搜索未保存 JoinHandle 的 tokio::spawn
rg 'tokio::spawn' --type rust --glob '!tests/' crates/ | rg -v 'JoinHandle|let.*='

# 预期：无输出
```

**预期结果**：无输出

**失败处理**：
- 检查 `bootstrap.rs:238` 是否保存 PluginLoadHandle
- 检查 `root.rs:168` 和 `mod.rs:197` 是否使用 ExecutionTaskManager
- 检查 `subagent.rs:128` 是否使用 SubagentTaskManager
- 检查 `watch_manager.rs:28 & 46` 是否保存 JoinHandle

---

### US4: 无持锁 Await

```bash
# 搜索持锁 await 模式
rg '\.lock\(\)\.await\..*\.await' --type rust crates/

# 预期：无输出
```

**预期结果**：无输出

**失败处理**：
- 检查是否先释放锁再 await
- 或改用 tokio Mutex

---

### US5: 错误转换保留上下文

```bash
# 搜索 map_err(|_| 模式（丢弃原始错误）
rg 'map_err\(\|_\|' --type rust --glob '!tests/' crates/

# 预期：无输出
```

**预期结果**：无输出

**失败处理**：
- 检查 `src-tauri/src/main.rs:424` 是否保留原始错误
- 检查所有 `map_err` 是否使用 `|e|` 而非 `|_|`

---

### US6: 无 println/eprintln

```bash
# 搜索生产代码中的 println! 和 eprintln!
rg 'println!|eprintln!' --type rust --glob '!tests/' --glob '!examples/' crates/

# 预期：无输出
```

**预期结果**：无输出

**失败处理**：
- 检查 `runtime-config/src/loader.rs:92` 是否改为 `log::warn!`
- 检查所有 `println!` 是否替换为 `log::info!` 或 `log::debug!`

---

### US6: 日志级别正确

```bash
# 搜索关键操作使用 debug! 的情况（应该用 error!）
rg 'turn.*failed.*debug!' --type rust crates/
rg 'hook.*call.*failed.*debug!' --type rust crates/

# 预期：无输出
```

**预期结果**：无输出

**失败处理**：
- 检查 `runtime-agent-loop/src/hook_runtime.rs` 的日志级别
- 检查 `core/src/runtime/coordinator.rs` 的日志级别

---

### US7: 文件行数限制

```bash
# 检查 service 目录下所有文件不超过 800 行
find crates/runtime/src/service -name '*.rs' -exec wc -l {} \; | awk '$1 > 800 {print "FAIL: " $2 " has " $1 " lines"; exit 1}'

# 预期：无输出（或 PASS）
```

**预期结果**：无输出

**失败处理**：
- 检查 `service/mod.rs` 是否已拆分
- 检查 `execution/mod.rs` 是否已拆分

---

### US8: Workspace 依赖统一

```bash
# 检查是否有依赖未使用 workspace = true
rg 'toml.*=.*\{.*version' crates/*/Cargo.toml | rg -v 'workspace.*=.*true'

# 预期：仅合理例外（如特定 crate 需要不同版本）
```

**预期结果**：无输出或仅合理例外

**失败处理**：
- 检查 toml、tracing、async-stream、tower 依赖是否统一
- 检查根 Cargo.toml 的 workspace.dependencies 是否完整

---

### 回归测试

```bash
# 运行所有测试
cargo test --workspace --exclude astrcode

# 前端类型检查
cd frontend && npm run typecheck
```

**预期结果**：
- 所有测试通过
- 前端类型检查通过

**失败处理**：
- 检查测试失败的具体原因
- 检查是否有测试依赖修改后的行为

---

## 回归测试场景

### 场景 1: 锁恢复后功能正常

**测试步骤**：
1. 启动应用
2. 创建新 session
3. 执行工具调用
4. 触发配置热重载
5. 验证所有功能正常

**验证点**：
- Session 创建成功
- 工具执行成功
- 配置热重载成功
- 无 panic 或崩溃

---

### 场景 2: 错误转换后错误信息完整

**测试步骤**：
1. 触发 storage 错误（如文件不存在）
2. 触发 plugin 错误（如能力未找到）
3. 触发 config 错误（如配置解析失败）
4. 检查错误日志

**验证点**：
- 错误消息包含原始错误信息
- 错误链可追溯（`error.source()` 不为 None）
- 错误日志包含完整上下文

---

### 场景 3: JoinHandle 管理后任务正常取消

**测试步骤**：
1. 启动 turn 执行
2. 启动子 Agent 执行
3. 启动配置监听
4. 触发取消操作
5. 验证任务正常停止

**验证点**：
- Turn 执行可以取消
- 子 Agent 执行可以取消
- 配置监听可以停止
- 无资源泄漏（JoinHandle 正确清理）

---

### 场景 4: 模块拆分后外部 API 行为不变

**测试步骤**：
1. 运行集成测试
2. 验证所有公共 API 行为一致
3. 检查模块间调用关系

**验证点**：
- 所有集成测试通过
- 公共 API 行为不变
- 无循环依赖

---

## 性能验证

### 锁恢复开销

```bash
# 运行性能测试（如果有）
cargo bench --bench lock_recovery

# 预期：锁恢复开销可忽略（< 1%）
```

### 错误转换开销

```bash
# 运行性能测试（如果有）
cargo bench --bench error_conversion

# 预期：错误转换开销可忽略（< 1%）
```

---

## 手动验证清单

### US1: 编译阻断问题

- [ ] `cargo check --workspace` 通过
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 通过
- [ ] 无编译错误
- [ ] 无 clippy 警告

### US2: Core 对 Protocol 的依赖

- [ ] `core/Cargo.toml` 不包含 `astrcode-protocol` 依赖
- [ ] Plugin 类型已移入 core
- [ ] Protocol 只保留传输 DTO
- [ ] 所有功能正常

### US3: Panic 路径

- [ ] 生产代码无 `.unwrap()` 或 `.expect()`
- [ ] 锁获取使用恢复机制
- [ ] 数组索引使用安全变体
- [ ] Timeout 等待使用 `match` 处理

### US4: 异步任务泄漏和持锁 Await

- [ ] 所有 `tokio::spawn` 保存 JoinHandle
- [ ] 无 fire-and-forget spawn
- [ ] 无持锁 await 模式
- [ ] 任务可以正常取消

### US5: 错误处理统一

- [ ] 所有自定义错误类型实现 `Into<AstrError>`
- [ ] 错误转换保留上下文
- [ ] 无 `map_err(|_| ...)` 用法
- [ ] 错误链可追溯

### US6: 日志级别和静默错误

- [ ] 关键操作使用 `error!` 日志级别
- [ ] 无 `println!` 或 `eprintln!`
- [ ] `.ok()` 和 `let _ =` 有注释说明
- [ ] 文件操作失败不被忽略

### US7: 模块拆分

- [ ] `service/mod.rs` 所有文件 ≤800 行
- [ ] `execution/mod.rs` 所有文件 ≤800 行
- [ ] 无循环依赖
- [ ] 外部 API 行为不变

### US8: 硬编码常量和依赖版本

- [ ] 端口号、大小限制提取为常量
- [ ] 所有依赖使用 `workspace = true`
- [ ] 常量定义在 `core/src/env.rs` 或 `runtime-config/src/constants.rs`
- [ ] 根 Cargo.toml 集中管理依赖版本

---

## 故障排查

### 编译错误

**症状**：`cargo check` 失败

**可能原因**：
- Pattern trait bound 未修复
- Plugin 类型迁移不完整
- 模块拆分后路径错误

**排查步骤**：
1. 查看具体错误消息
2. 检查相关文件是否修改正确
3. 检查导入路径是否正确

---

### Clippy 警告

**症状**：`cargo clippy` 报告警告

**可能原因**：
- Unused import 未移除
- Dead code 未删除
- 不符合 clippy 规则

**排查步骤**：
1. 查看具体警告消息
2. 根据警告修复代码
3. 重新运行 clippy

---

### 测试失败

**症状**：`cargo test` 失败

**可能原因**：
- 锁恢复机制改变行为
- 错误类型转换不兼容
- 模块拆分后路径错误

**排查步骤**：
1. 查看失败的测试
2. 检查测试是否依赖修改后的行为
3. 更新测试或修复代码

---

### 运行时错误

**症状**：应用运行时崩溃或行为异常

**可能原因**：
- 锁恢复后状态不一致
- JoinHandle 管理不正确
- 错误转换丢失上下文

**排查步骤**：
1. 查看错误日志
2. 检查相关代码是否正确实现
3. 添加调试日志定位问题

---

## 完整验证流程

```bash
#!/bin/bash
# 完整验证脚本

set -e

echo "=== US1: 编译和 Clippy 检查 ==="
cargo check --workspace
cargo clippy --all-targets --all-features -- -D warnings

echo "=== US2: Core 不依赖 Protocol ==="
! grep -q "astrcode-protocol" crates/core/Cargo.toml || (echo "FAIL: core still depends on protocol" && exit 1)

echo "=== US3: 无 Panic 路径 ==="
! rg '\.unwrap\(\)|\.expect\(' --type rust --glob '!tests/' --glob '!benches/' crates/ || (echo "FAIL: found unwrap/expect in production code" && exit 1)

echo "=== US4: 无 Fire-and-Forget Spawn ==="
! rg 'tokio::spawn' --type rust --glob '!tests/' crates/ | rg -v 'JoinHandle|let.*=' || (echo "FAIL: found fire-and-forget spawn" && exit 1)

echo "=== US4: 无持锁 Await ==="
! rg '\.lock\(\)\.await\..*\.await' --type rust crates/ || (echo "FAIL: found lock-then-await pattern" && exit 1)

echo "=== US5: 错误转换保留上下文 ==="
! rg 'map_err\(\|_\|' --type rust --glob '!tests/' crates/ || (echo "FAIL: found map_err discarding error" && exit 1)

echo "=== US6: 无 println/eprintln ==="
! rg 'println!|eprintln!' --type rust --glob '!tests/' --glob '!examples/' crates/ || (echo "FAIL: found println/eprintln in production code" && exit 1)

echo "=== US7: 文件行数限制 ==="
find crates/runtime/src/service -name '*.rs' -exec wc -l {} \; | awk '$1 > 800 {print "FAIL: " $2 " has " $1 " lines"; exit 1}'

echo "=== 回归测试 ==="
cargo test --workspace --exclude astrcode

echo "=== 前端类型检查 ==="
cd frontend && npm run typecheck

echo "=== 所有验证通过 ==="
```

---

## 总结

本文档提供了完整的验证和回归测试指南，涵盖：
- 8 个 User Story 的验证命令
- 4 个回归测试场景
- 性能验证方法
- 手动验证清单
- 故障排查指南
- 完整验证脚本

使用这些验证方法可以确保所有质量问题都已修复且不引入新问题。
