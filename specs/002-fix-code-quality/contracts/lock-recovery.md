# Contract: Lock Recovery

**Feature**: 002-fix-code-quality  
**Date**: 2026-04-08  
**Status**: Binding

本契约定义锁获取的恢复机制规则。

---

## Purpose

确保生产代码中的锁获取不会因 panic 而崩溃：
1. 所有 `std::sync::Mutex` 锁获取使用恢复机制
2. 锁中毒时自动恢复，不 panic
3. 记录错误日志便于诊断

---

## Contract Rules

### Rule 1: 禁止 unwrap/expect

生产代码中的锁获取 **MUST NOT** 使用 `.unwrap()` 或 `.expect()`：

**❌ WRONG**:
```rust
let guard = self.state.lock().unwrap();
let guard = self.tokens.lock().expect("token lock poisoned");
```

**✅ CORRECT**:
```rust
// 使用 with_lock_recovery（自动恢复）
with_lock_recovery(&self.state, "session state", |state| {
    *state = State::Idle;
})

// 或使用 lock_anyhow（返回 Result）
let guard = lock_anyhow(&self.tokens, "auth tokens")?;
```

### Rule 2: 锁恢复机制

所有 `std::sync::Mutex` 锁获取 **MUST** 使用以下两种方式之一：

#### 方式 1: with_lock_recovery（自动恢复，不返回 Result）

```rust
use astrcode_core::support::with_lock_recovery;

with_lock_recovery(&self.phase, "session phase", |phase| {
    *phase = Phase::Idle;
})
```

**适用场景**：
- 不需要向上传播错误
- 可以自动恢复并继续执行
- 低竞争场景（如配置更新、状态切换）

#### 方式 2: lock_anyhow（返回 Result）

```rust
use astrcode_core::support::lock_anyhow;

let guard = lock_anyhow(&self.tokens, "auth tokens")?;
// 使用 guard
```

**适用场景**：
- 需要向上传播错误
- 关键路径（如认证、授权）
- 需要调用者决定如何处理锁中毒

### Rule 3: tokio Mutex 处理

`tokio::sync::Mutex` **MUST** 使用 `.lock().await` 直接获取：

```rust
let guard = self.async_state.lock().await;
```

**Rationale**：
- tokio Mutex 不会 poison（设计上不会因 panic 而毒化）
- 失败模式主要是任务被 cancel 或 runtime 关闭
- 这些情况下无法恢复，应该让错误传播

### Rule 4: 持锁 await 禁止

**MUST NOT** 在持有 `std::sync::Mutex` 守卫时跨越 `.await` 点：

**❌ WRONG**:
```rust
let guard = self.state.lock().unwrap();
some_async_operation().await; // 持锁 await，可能死锁
drop(guard);
```

**✅ CORRECT**:
```rust
// 先释放锁，再 await
{
    let guard = lock_anyhow(&self.state, "state")?;
    // 同步操作
}
some_async_operation().await; // 锁已释放
```

**或使用 tokio Mutex**:
```rust
let guard = self.async_state.lock().await;
some_async_operation().await; // tokio Mutex 支持持锁 await
drop(guard);
```

---

## Implementation

### Core Support Functions

**Location**: `crates/core/src/support.rs`

```rust
use std::sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard};
use crate::error::AstrError;

/// 自动恢复锁中毒，不返回 Result
pub fn with_lock_recovery<T, R>(
    mutex: &StdMutex<T>,
    name: &'static str,
    update: impl FnOnce(&mut T) -> R,
) -> R {
    match mutex.lock() {
        Ok(mut guard) => update(&mut guard),
        Err(poisoned) => {
            log::error!("mutex '{name}' was poisoned; recovering inner state");
            let mut guard = poisoned.into_inner();
            let result = update(&mut guard);
            mutex.clear_poison();
            result
        },
    }
}

/// 返回 Result，将锁中毒转换为 AstrError
pub fn lock_anyhow<'a, T>(
    mutex: &'a StdMutex<T>,
    name: &'static str,
) -> Result<StdMutexGuard<'a, T>, AstrError> {
    mutex.lock().map_err(|poisoned| {
        log::error!("mutex '{name}' was poisoned; recovering inner state");
        AstrError::LockPoisoned { name }
    }).or_else(|_| {
        // 尝试恢复
        mutex.lock().map_err(|_| AstrError::LockPoisoned { name })
    })
}
```

---

## Validation

### 编译时验证

```bash
# 确保所有锁获取都使用恢复机制
cargo check --workspace
```

### 运行时验证

```bash
# 搜索 .lock().unwrap() 和 .lock().expect()（不应存在）
rg '\.lock\(\)\.unwrap\(\)|\.lock\(\)\.expect\(' --type rust --glob '!tests/' crates/

# 预期：无输出

# 搜索持锁 await 模式（不应存在）
rg '\.lock\(\)\.await\..*\.await' --type rust crates/

# 预期：无输出
```

### 测试验证

```rust
#[test]
fn test_lock_recovery() {
    use std::sync::{Arc, Mutex};
    use std::thread;
    
    let mutex = Arc::new(Mutex::new(0));
    let mutex_clone = Arc::clone(&mutex);
    
    // 模拟 panic 导致锁中毒
    let _ = thread::spawn(move || {
        let _guard = mutex_clone.lock().unwrap();
        panic!("intentional panic");
    }).join();
    
    // 验证恢复机制
    with_lock_recovery(&mutex, "test mutex", |value| {
        *value = 42;
    });
    
    assert_eq!(*mutex.lock().unwrap(), 42);
}
```

---

## Affected Locations

### 生产代码（优先级 P1）

| 文件 | 行号 | 当前代码 | 修复方案 |
|------|------|--------|--------|
| `plugin/src/peer.rs` | 300 | `.lock().unwrap()` | `with_lock_recovery()` |
| `server/src/http/auth.rs` | 99 | `.lock().expect()` | `lock_anyhow()?` |

### 测试代码（优先级 P3）

- `runtime-agent-loop/src/agent_loop/tests/` - 26 处
- `runtime-agent-tool/src/lib.rs` - 2 处
- `runtime-llm/src/` - 5 处
- `runtime/src/` - 8 处
- `sdk/src/tests.rs` - 2 处
- `core/src/runtime/coordinator.rs` - 1 处
- `runtime-agent-loop/src/context_pipeline.rs` - 2 处

---

## Migration Checklist

- [ ] 在 `core/src/support.rs` 中定义 `with_lock_recovery` 和 `lock_anyhow`
- [ ] 修复 `plugin/src/peer.rs:300`
- [ ] 修复 `server/src/http/auth.rs:99`
- [ ] 逐步修复其他 crate（runtime-agent-tool、runtime-agent-loop、runtime-llm）
- [ ] 添加锁恢复测试
- [ ] 验证无持锁 await 模式

---

## Examples

### Example 1: Session State Update

```rust
// runtime-session/src/turn_runtime.rs

pub fn set_phase(&self, new_phase: Phase) {
    with_lock_recovery(&self.phase, "session phase", |phase| {
        *phase = new_phase;
    })
}
```

### Example 2: Token Validation

```rust
// server/src/http/auth.rs

pub fn validate_token(&self, token: &str) -> Result<UserId> {
    let tokens = lock_anyhow(&self.tokens, "auth tokens")?;
    tokens.get(token)
        .cloned()
        .ok_or(AstrError::Unauthorized)
}
```

### Example 3: Plugin Read Loop Handle

```rust
// plugin/src/peer.rs

pub fn set_read_loop_handle(&self, handle: JoinHandle<()>) {
    with_lock_recovery(&self.inner.read_loop_handle, "read loop handle", |h| {
        *h = Some(handle);
    })
}
```

---

## Rationale

**为什么使用 poisoned.into_inner()？**
- 恢复数据：从中毒的 Mutex 中提取数据
- 不 panic：确保程序继续运行
- 记录日志：便于诊断问题

**为什么需要两种方式？**
- `with_lock_recovery`：适合不需要错误传播的场景，代码更简洁
- `lock_anyhow`：适合需要错误传播的场景，调用者可以决定如何处理

**为什么 tokio Mutex 不需要恢复？**
- 设计差异：tokio Mutex 不会 poison
- 失败模式：主要是任务取消，无法恢复
- 最佳实践：直接使用 `.lock().await`

**为什么禁止持锁 await？**
- 死锁风险：持锁期间 await 可能导致其他任务无法获取锁
- 性能问题：持锁时间过长影响并发性能
- 最佳实践：先释放锁，再 await

---

## Compliance

本契约是 **强制性** 的，所有涉及锁获取的代码 **MUST** 遵守。

违反本契约的代码 **MUST** 在 code review 中被拒绝。

---

## Version

**Version**: 1.0  
**Last Updated**: 2026-04-08  
**Status**: Binding
