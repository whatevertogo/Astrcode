# Code Review — runtime-mcp (009-mcp-integration)

## 摘要
**文件审查**: 21 个 .rs 文件 | **新问题**: 8 (2 Critical, 3 High, 3 Medium) | **测试**: 79 passed ✅ | **Clippy**: 通过 ✅

---

## 🔒 Security

| 严重性 | 问题 | 文件:行 | 攻击路径 |
|--------|------|---------|----------|
| High | **环境变量展开未做安全校验** `loader.rs` 的 `expand_env_vars()` 函数直接读取环境变量值并拼接到配置中。如果攻击者能控制环境变量，可以注入恶意命令或 URL。 | `config/loader.rs` | 恶意环境变量 → 配置注入 → 执行非预期命令 |
| Medium | **MCP 服务器指令 (instructions) 未做 sanitization 直接注入 prompt** | `bridge/prompt_bridge.rs` | 恶意 MCP 服务器 → 注入恶意指令 → 影响 Agent 行为 |

### SEC-001: 环境变量注入风险
**文件**: `config/loader.rs`
```rust
// 当前实现
if let Ok(val) = env::var(&var_name) {
    result.push_str(&val);
}
```
**风险**: 如果 `.mcp.json` 中包含 `${PATH}` 或 `${HOME}` 等敏感变量，值会被直接注入到 command/args 中，可能导致命令注入。
**修复建议**: 对注入的值做白名单校验或转义，特别是用于 `command` 字段时。

---

## 📝 Code Quality

| 严重性 | 问题 | 文件:行 | 后果 |
|--------|------|---------|------|
| **Critical** | **`manager/mod.rs` 严重超过 800 行限制** (当前 996 行) | `manager/mod.rs` | 违反 AGENTS.md 宪法约束，可维护性差 |
| **Critical** | **`establish_connection_inner` 作为模块级函数而非方法** | `manager/mod.rs` | 需要传递大量参数，破坏封装性，容易遗漏状态更新 |
| High | **`refresh_tools_for_server` 未更新 CapabilityRouter** | `manager/mod.rs` | 工具列表刷新后，旧 invoker 仍被路由使用，新工具不可用 |
| High | **`McpConnectionManager::connect_one()` 未检查 `enabled` 标志** | `manager/mod.rs` | 被禁用的服务器仍会被连接 |
| Medium | **`create_transport` 函数暴露为 `pub(crate)` 但无文档说明使用场景** | `manager/mod.rs` | 调用方可能误用 |
| Medium | **`McpManagedConnection` 中的 `transport` 和 `client` 都是 `Arc<Mutex<>>`** | `manager/mod.rs` | 双重 Mutex 嵌套，可能导致性能问题和死锁风险 |

### CQ-001: manager/mod.rs 违反 800 行宪法约束 ⚠️
**当前状态**: 996 行 (超过 24.5%)
**包含职责**:
- McpConnectionManager struct 定义
- connect_all / connect_one 方法
- disconnect_one / wait_idle 方法
- 审批过滤逻辑
- 热加载集成
- 健康检查
- establish_connection_inner 模块级函数 (~100 行)
- refresh_tools_for_server 模块级函数 (~60 行)
- create_transport 工厂函数
- ManagedRuntimeComponent trait 实现
- 14 个单元测试

**修复建议**: 拆分为以下模块:
```
manager/
├── mod.rs              (仅声明子模块 + McpConnectionManager 公共接口)
├── connection.rs       (已有)
├── reconnect.rs        (已有)
├── hot_reload.rs       (已有)
├── connection_ops.rs   (connect_one, connect_all, disconnect_one)
├── connection_lifecycle.rs  (establish_connection, refresh_tools)
└── component.rs        (ManagedRuntimeComponent 实现)
```

### CQ-002: establish_connection_inner 参数爆炸
**文件**: `manager/mod.rs`
```rust
async fn establish_connection_inner(
    connections: &Arc<Mutex<HashMap<String, McpManagedConnection>>>,
    in_flight_count: &Arc<AtomicUsize>,
    reconnect_manager: &reconnect::McpReconnectManager,
    config: &McpServerConfig,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    // ...
}
```
**问题**: 5 个参数，其中 3 个是管理器内部状态。这说明该函数应该是 `McpConnectionManager` 的方法而非模块级函数。
**后果**: 如果调用方遗漏传递某个参数，会导致编译错误或状态不一致。

### CQ-003: refresh_tools_for_server 未通知 CapabilityRouter
**文件**: `manager/mod.rs`
```rust
async fn refresh_tools_for_server(...) {
    // 仅更新 managed.invokers
    managed.invokers = new_invokers;
}
```
**问题**: 刷新后的新 invoker 没有注册到 CapabilityRouter。Agent 仍会使用旧的工具定义。
**修复**: 应通过 channel 或 callback 通知上层更新路由。

---

## ✅ Tests

**运行结果**: 79 passed, 0 failed, 0 skipped ✅

| 严重性 | 未测试的场景 | 位置 |
|--------|-------------|------|
| High | **`stdio.rs` 传输实现完全无测试** | `transport/stdio.rs` |
| Medium | **工具调用取消流程未测试** (cancel → timeout → force disconnect) | `manager/mod.rs` |
| Medium | **list_changed 通知触发工具刷新未测试** | `manager/mod.rs` |
| Low | **热加载的文件变更事件未测试端到端** | `manager/hot_reload.rs` |

### TST-001: stdio.rs 缺少测试
**风险**: stdio 是最常用的传输方式（本地 MCP 服务器），但完全没有测试覆盖。
**建议**: 至少测试:
- 进程启动失败
- 进程优雅关闭 (SIGINT → SIGTERM → SIGKILL)
- stdin/stdout 消息收发

### TST-002: 缺少真实 MCP 服务器集成测试
**当前**: 所有测试使用 mock 传输
**建议**: 添加至少一个真实 MCP 服务器的集成测试 (例如 `mcp-server-filesystem`)

---

## 🏗️ Architecture

| 严重性 | 不一致 | 文件 |
|--------|--------|------|
| High | **`server` 直接依赖 `astrcode-runtime-mcp`**，可能绕过 `runtime` 门面 | `crates/server/Cargo.toml` |
| High | **`runtime-mcp` 依赖 `astrcode-runtime-prompt`**，违反了 "仅依赖 core" 的宪法约束 | `crates/runtime-mcp/Cargo.toml` |
| Medium | **协议常量定义在 `types.rs` 而非 `core/env.rs`** | `protocol/types.rs` |
| Medium | **`McpManagedConnection` 持有 `Arc<Mutex<dyn McpTransport>>` 和 `Arc<Mutex<McpClient>>`** | `manager/mod.rs` |

### ARC-001: server 直接依赖 runtime-mcp
**问题**: `server` 层直接依赖 `astrcode-runtime-mcp`，可以绕过 `runtime` 门面直接调用 MCP 功能。
**当前用途**: MCP 审批 API 路由 (`/api/mcp/approve`, `/api/mcp/reject`)
**修复建议**: 审批 API 应通过 `RuntimeService` 暴露，server 不应直接持有 `McpConnectionManager` 引用。

### ARC-002: runtime-mcp 依赖 runtime-prompt
**问题**: 根据 AGENTS.md "runtime-...-loader 仅依赖 core，不依赖 runtime"，`runtime-prompt` 属于 runtime 家族，不应被 runtime-mcp 直接依赖。
**当前用途**: `PromptDeclaration` 类型定义
**修复建议**: 将 `PromptDeclaration` 移到 `core` 或 `protocol` crate。

### ARC-003: 双重 Mutex 嵌套
**问题**:
```rust
pub(crate) struct McpManagedConnection {
    pub(crate) transport: Arc<Mutex<dyn McpTransport>>,
    pub(crate) client: Arc<Mutex<McpClient>>,
}
```
`McpClient` 内部已经持有 `Arc<Mutex<dyn McpTransport>>`。再包装一层 `Mutex` 会导致:
1. 两次锁竞争
2. 潜在的锁顺序问题
**修复建议**: `McpManagedConnection` 只持有 `Arc<Mutex<McpClient>>`，通过 client 访问 transport。

---

## 🚨 Must Fix Before Merge

1. **[CQ-001]** `manager/mod.rs` 996 行，超过 800 行宪法约束
   - **影响**: 违反架构约束，代码可维护性差
   - **修复**: 拆分为 `connection_ops.rs` + `connection_lifecycle.rs` + `component.rs`

2. **[ARC-002]** `runtime-mcp` 依赖 `astrcode-runtime-prompt`
   - **影响**: 违反宪法 "仅依赖 core" 约束
   - **修复**: 将 `PromptDeclaration` 移到 `protocol` crate

3. **[CQ-003]** `refresh_tools_for_server` 未更新 CapabilityRouter
   - **影响**: 工具刷新后 Agent 仍使用旧定义
   - **修复**: 添加回调/channel 通知上层路由更新

---

## 📎 Pre-Existing Issues (not blocking)

- 协议常量 (`MCP_PROTOCOL_VERSION`, `DEFAULT_TOOL_TIMEOUT_SECS` 等) 定义在 `types.rs` 而非 `core/env.rs`
- `McpConnectionManager` 的 `connect_one()` 未检查 `config.enabled` 标志
- `establish_connection_inner` 作为模块级函数而非方法

---

## 🤔 Low-Confidence Observations

- `hot_reload.rs` 的 `events()` 方法返回 `&mut Receiver`，调用方需要小心管理借用关系
- `McpToolBridge` 的 `execute()` 方法在工具调用失败时返回的 `AstrError` 可能缺少上下文信息
- `SseTransport` 和 `StreamableHttpTransport` 的错误处理路径可能有重叠逻辑，考虑提取公共函数

---

## 📊 审查清单

- [x] 上下文收集: 完整了解代码结构、依赖、测试状态
- [x] 4 个视角审查: Security ✅ Code Quality ✅ Tests ✅ Architecture ✅
- [x] 置信度过滤: 仅报告确定问题
- [x] 新 vs 现有问题分离
- [x] 测试验证: 79 passed, clippy 通过

## 总结

**runtime-mcp crate 整体架构清晰，职责分离合理，测试覆盖率良好 (79 tests)**。主要问题集中在:

1. **宪法约束违反**: manager/mod.rs 超过 800 行、依赖 runtime-prompt
2. **功能缺陷**: refresh_tools 未通知路由、connect_one 未检查 enabled
3. **测试缺失**: stdio 传输无测试、取消流程无测试

建议在 merge 前优先修复 Critical/High 问题，特别是文件拆分和依赖调整。