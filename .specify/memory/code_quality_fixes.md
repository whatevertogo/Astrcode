---
name: code_quality_fixes
description: 002-fix-code-quality 规范的核心质量约束和修复模式
type: feedback
---

## 核心质量约束

### 1. 锁获取恢复机制（强制）

**规则**：生产代码禁止 `.lock().unwrap()` / `.lock().expect()`

**Why**：宪法 VI Runtime Robustness 禁止 panic 路径，锁中毒必须恢复而非崩溃

**How to apply**：
- `std::sync::Mutex`：使用 `with_lock_recovery()` 或 `lock_anyhow()?`
- `tokio::sync::Mutex`：直接 `.lock().await`（不会 poison）
- 禁止持锁 await：先释放 std Mutex 再 await，或改用 tokio Mutex

```rust
// ✅ 自动恢复（不需要错误传播）
with_lock_recovery(&self.state, "session state", |state| {
    *state = State::Idle;
})

// ✅ 返回 Result（需要错误传播）
let guard = lock_anyhow(&self.tokens, "auth tokens")?;
```

### 2. 错误转换保留上下文（强制）

**规则**：禁止 `map_err(|_| ...)` 丢弃原始错误

**Why**：错误链断裂导致无法追溯根因，违反可观测性原则

**How to apply**：
- 使用 `#[source]` 保留错误链
- `map_err` 必须保留原始错误：`map_err(|e| ...)`
- 各 crate 错误类型实现 `From<T> for AstrError`

```rust
// ✅ 保留错误链
#[error("storage error")]
Storage {
    #[source]
    inner: StorageError,
}

// ❌ 丢弃上下文
.map_err(|_| AstrError::Unknown)
```

### 3. 异步任务生命周期管理（强制）

**规则**：所有 `tokio::spawn` 必须保存 `JoinHandle` 并提供取消机制

**Why**：fire-and-forget 导致资源泄漏和优雅关闭失败

**How to apply**：
- 在结构体中保存 `Vec<JoinHandle<()>>`
- shutdown 时批量 `abort()` 或 `await`
- 各模块自管理任务（不集中管理）

```rust
pub struct ServiceManager {
    tasks: Vec<JoinHandle<()>>,
}

impl ServiceManager {
    pub async fn shutdown(&mut self) {
        for handle in self.tasks.drain(..) {
            handle.abort();
        }
    }
}
```

### 4. 日志级别和静默错误（强制）

**规则**：
- 关键操作失败使用 `error!`（不是 `warn!` 或 `debug!`）
- 禁止 `.ok()` 和 `let _ =` 静默吞掉错误（除非有注释说明）
- 生产代码禁止 `println!` / `eprintln!`（使用 `log::*!`）

**Why**：宪法 VII Observability 要求，影响问题诊断效率

**How to apply**：
- turn failed / hook call failed → `error!`
- 文件操作失败 → 记录日志或返回错误
- `let _ =` 必须添加 `// 故意忽略：<原因>` 注释

### 5. Plugin 类型归属（架构约束）

**规则**：Plugin 相关类型属于 `core`，不属于 `protocol`

**Why**：宪法 1.2.0 要求 core 和 protocol 双向独立，Plugin 是核心扩展机制

**How to apply**：
- Descriptor 类型族（11 个）→ `core/src/plugin/`
- 元数据类型（4 个）→ `core/src/plugin/`
- 调用上下文（4 个）→ `core/src/plugin/`
- 传输 DTO（InitializeMessage 等）→ 保留在 `protocol`
- 在 `plugin` crate 实现 Mapper（protocol DTO ↔ core 领域类型）

### 6. 模块大小约束（架构约束）

**规则**：单文件不超过 800 行（宪法 1.2.0）

**Why**：保持代码可维护性，避免单文件承担过多职责

**How to apply**：
- 按职责拆分：状态管理 / 事件处理 / 生命周期管理
- 优先拆分 service 模块（当前最大）
- 拆分在边界重构稳定后执行（避免同时做两件大事）

## 工具安全增强（已完成）

### 1. 文件操作安全（P0）
- 设备文件黑名单：禁止读取 `/dev/zero` 等
- UNC 路径检查：防止 Windows NTLM 泄漏
- 文件大小限制：edit_file 限制 1 GiB
- 符号链接检测：防止绕过路径沙箱

### 2. 用户体验改进（P1）
- 引号规范化：自动转换智能引号（""）为标准引号（""）
- Grep 默认限制：250 条结果（防止 context window 爆炸）

### 3. Agent 友好性（P2）
- ListDir 按大小排序：`sortBy: "size"`（降序）

## 迁移策略

### Plugin 类型迁移（5 阶段）
1. 准备：core 创建 plugin 模块，protocol 创建 re-export 层
2. SDK/Plugin 层：更新导入，创建 Mapper
3. Runtime 层：更新导入
4. Server 层：更新导入
5. 清理：移除 re-export，更新依赖

### 模块拆分（两阶段）
1. 先拆分 service 模块（当前最大）
2. 再拆分 agent 模块（如需要）

## 验证命令

```bash
# 编译检查
cargo check --workspace
cargo clippy --all-targets --all-features -- -D warnings

# 锁获取检查
rg '\.lock\(\)\.unwrap\(\)|\.lock\(\)\.expect\(' --type rust --glob '!tests/' crates/

# 持锁 await 检查
rg '\.lock\(\)\.await\..*\.await' --type rust crates/

# 错误丢弃检查
rg 'map_err\(\|_\|' --type rust --glob '!tests/' crates/

# 静默错误检查
rg '\.ok\(\);|let _ =' --type rust --glob '!tests/' crates/

# 模块大小检查
fd -e rs -x wc -l {} \; | awk '$1 > 800 {print}'
```
