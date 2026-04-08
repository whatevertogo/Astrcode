# Contract: Error Conversion

**Feature**: 002-fix-code-quality  
**Date**: 2026-04-08  
**Status**: Binding

本契约定义各 crate 错误类型与 `AstrError` 的转换规则。

---

## Purpose

统一错误处理链路，确保：
1. 所有 crate 的错误最终可转换为 `AstrError`
2. 错误转换保留原始错误信息（不丢失上下文）
3. 上层调用者只需处理 `AstrError` 一种错误类型

---

## Contract Rules

### Rule 1: 错误类型转换

每个 crate 的自定义错误类型 **MUST** 实现 `Into<AstrError>`：

```rust
impl From<ProtocolError> for AstrError {
    fn from(err: ProtocolError) -> Self {
        AstrError::Protocol { inner: err }
    }
}

impl From<StorageError> for AstrError {
    fn from(err: StorageError) -> Self {
        AstrError::Storage { inner: err }
    }
}

// ... 其他 crate 的错误类型
```

### Rule 2: 错误链保留

转换 **MUST** 保留原始错误信息：

**✅ CORRECT**:
```rust
// 使用 #[source] 保留错误链
#[error("storage error")]
Storage {
    #[source]
    inner: StorageError,
}

// 或在 map_err 中保留上下文
.map_err(|e| AstrError::Storage { inner: e })
```

**❌ WRONG**:
```rust
// 丢弃原始错误
.map_err(|_| AstrError::Storage { inner: StorageError::Unknown })

// 只保留字符串，丢失类型信息
.map_err(|e| AstrError::Generic(e.to_string()))
```

### Rule 3: map_err 使用规范

`map_err` **MUST NOT** 使用 `|_|` 丢弃原始错误：

**✅ CORRECT**:
```rust
// 保留原始错误
storage.load_session(id)
    .map_err(|e| AstrError::Storage { inner: e })?

// 或使用 ? 自动转换（如果实现了 From）
storage.load_session(id)?
```

**❌ WRONG**:
```rust
// 丢弃原始错误
storage.load_session(id)
    .map_err(|_| AstrError::Storage { inner: StorageError::Unknown })?
```

### Rule 4: 错误追溯

所有错误 **MUST** 支持 `error.source()` 追溯：

```rust
let err: AstrError = storage_error.into();
assert!(err.source().is_some()); // 必须能追溯到原始错误
```

---

## Validation

### 编译时验证

```bash
# 确保所有自定义错误类型都实现了 Into<AstrError>
cargo check --workspace
```

### 运行时验证

```bash
# 搜索 map_err(|_| 模式（不应存在）
rg 'map_err\(\|_\|' --type rust --glob '!tests/' crates/

# 预期：无输出
```

### 测试验证

```rust
#[test]
fn test_error_conversion_preserves_context() {
    let storage_err = StorageError::FileNotFound("session.jsonl".to_string());
    let astr_err: AstrError = storage_err.into();
    
    // 验证错误链
    assert!(astr_err.source().is_some());
    
    // 验证错误消息包含原始信息
    let msg = astr_err.to_string();
    assert!(msg.contains("session.jsonl"));
}
```

---

## Affected Crates

| Crate | 错误类型 | AstrError Variant | 实现位置 |
|-------|---------|------------------|---------|
| `protocol` | `ProtocolError` | `AstrError::Protocol` | `core/src/error.rs` |
| `storage` | `StorageError` | `AstrError::Storage` | `core/src/error.rs` |
| `plugin` | `PluginError` | `AstrError::Plugin` | `core/src/error.rs` |
| `runtime-config` | `ConfigError` | `AstrError::Config` | `core/src/error.rs` |
| `runtime-registry` | `RegistryError` | `AstrError::Registry` | `core/src/error.rs` |
| `runtime-agent-loop` | `AgentLoopError` | `AstrError::AgentLoop` | `core/src/error.rs` |

---

## Migration Checklist

- [ ] 在 `core/src/error.rs` 中新增 6 个 AstrError variants
- [ ] 为每个自定义错误类型实现 `From<T> for AstrError`
- [ ] 搜索并修复所有 `map_err(|_| ...)` 用法
- [ ] 添加错误转换测试
- [ ] 验证错误链可追溯

---

## Examples

### Example 1: Storage Error Conversion

```rust
// storage/src/session/event_log.rs

pub fn load_events(&self, session_id: &str) -> Result<Vec<StorageEvent>> {
    let path = self.session_path(session_id);
    let file = std::fs::File::open(&path)
        .map_err(|e| StorageError::FileNotFound(path.display().to_string()))?;
    
    // ... 读取逻辑
}

// 上层调用（runtime）
let events = storage.load_events(session_id)?; // 自动转换为 AstrError
```

### Example 2: Plugin Error Conversion

```rust
// plugin/src/invoker.rs

pub async fn invoke(&self, request: InvokeRequest) -> Result<InvokeResult> {
    let capability = self.find_capability(&request.capability_id)
        .ok_or_else(|| PluginError::CapabilityNotFound(request.capability_id.clone()))?;
    
    // ... 调用逻辑
}

// 上层调用（runtime）
let result = invoker.invoke(request).await?; // 自动转换为 AstrError
```

### Example 3: Config Error Conversion

```rust
// runtime-config/src/loader.rs

pub fn load_config(&self) -> Result<RuntimeConfig> {
    let content = std::fs::read_to_string(&self.config_path)
        .map_err(|e| ConfigError::FileReadError {
            path: self.config_path.clone(),
            source: e,
        })?;
    
    toml::from_str(&content)
        .map_err(|e| ConfigError::ParseError {
            path: self.config_path.clone(),
            source: e,
        })
}

// 上层调用（runtime）
let config = loader.load_config()?; // 自动转换为 AstrError
```

---

## Rationale

**为什么保留各 crate 自有错误类型？**
- 领域特定：每个 crate 的错误有其特定的上下文和字段
- 类型安全：在 crate 内部可以精确匹配错误类型
- 可扩展：各 crate 可以独立扩展其错误类型

**为什么使用 #[source]？**
- 标准做法：符合 Rust 错误处理最佳实践
- 工具支持：`anyhow`、`thiserror` 等库自动支持
- 可追溯：支持 `error.source()` 链式追溯

**为什么禁止 map_err(|_|)？**
- 丢失上下文：无法追溯原始错误
- 调试困难：错误消息不完整
- 违反契约：破坏错误链

---

## Compliance

本契约是 **强制性** 的，所有涉及错误处理的代码 **MUST** 遵守。

违反本契约的代码 **MUST** 在 code review 中被拒绝。

---

## Version

**Version**: 1.0  
**Last Updated**: 2026-04-08  
**Status**: Binding
