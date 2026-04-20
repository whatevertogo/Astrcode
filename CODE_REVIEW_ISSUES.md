# Code Review -- Astrcode Backend (master)

## Summary
Files reviewed: ~200+ (18 crates) | New issues: 10 (1 critical, 3 high, 4 medium, 2 low) | Perspectives: 4/4

---

## Security

| Sev | Issue | File:Line | Attack path |
|-----|-------|-----------|-------------|
| Critical | `/__astrcode__/run-info` 端点无认证保护，任何能访问 127.0.0.1 的人都能获取 bootstrap token | `crates/server/src/bootstrap/mod.rs:118-154` | 本地任意进程 -> `GET /__astrcode__/run-info` -> 获取 bootstrap token -> `POST /api/auth/exchange` -> 获取 API session token -> 完全控制所有 API |
| High | `delete_project` 接受未经验证的 `working_dir` 查询参数 | `crates/server/src/http/routes/sessions/mutation.rs:191-203` | 认证后 -> `DELETE /api/projects?working_dir=<arbitrary_path>` -> 删除任意项目的所有 session |
| Medium | Anthropic provider 日志泄露 API key 前 4 位和后 4 位 | `crates/adapter-llm/src/anthropic/provider.rs:170-181` | 日志文件被读取 -> 攻击者获取 API key 部分 -> 缩小暴力破解空间 |

### [SEC-001] Critical: `/__astrcode__/run-info` 无认证

`serve_run_info` 是所有路由中唯一不调用 `require_auth` 的端点（设计如此，因为前端 Vite dev server 在认证交换前需要获取 token）。但该端点直接返回明文 bootstrap token：

```rust
// crates/server/src/bootstrap/mod.rs:118
pub(crate) async fn serve_run_info(
    State(_state): State<AppState>,  // 注意：_state 未使用
) -> Result<Json<BrowserBootstrapResponse>, ApiError> {
    // ...读取 run.json...
    Ok(Json(BrowserBootstrapResponse {
        token: run_info.token,  // 明文返回
        server_origin: format!("http://127.0.0.1:{}", run_info.port),
    }))
}
```

由于 server 绑定在 `127.0.0.1:0`（随机端口），攻击面限于本机。但本机上的任意进程（包括浏览器中访问的恶意网页，通过 DNS rebinding 或 SSRF）都能获取 token，进而完全控制 API。

**缓解因素**: server 仅监听 `127.0.0.1`；token 有 24 小时过期；run.json 文件本身也有权限控制。

**建议**: 至少检查请求的 `Origin` 头是否匹配允许的 CORS 来源列表；或在非 dev 环境下关闭此端点。

### [SEC-002] High: `delete_project` 未验证 `working_dir`

```rust
// crates/server/src/http/routes/sessions/mutation.rs:191-203
pub(crate) async fn delete_project(
    ...
    Query(query): Query<DeleteProjectQuery>,  // query.working_dir 未做任何校验
) -> ... {
    require_auth(&state, &headers, None)?;
    let result = state.app.delete_project(&query.working_dir).await...
```

对比 `submit_prompt` 等路由会对 session_id 调用 `validate_session_path_id` 做字符白名单校验，`delete_project` 的 `working_dir` 直接传入后端，没有路径规范化或白名单检查。虽然后端存储层会用 `working_dir` 做项目目录映射而非直接拼路径（参见 `adapter-storage/src/session/paths.rs`），但缺少输入验证仍是不好的防御纵深。

### [SEC-003] Medium: API key 部分泄露到日志

```rust
// crates/adapter-llm/src/anthropic/provider.rs:170-181
let api_key_preview = if self.api_key.len() > 8 {
    format!("{}...{}", &self.api_key[..4], &self.api_key[self.api_key.len() - 4..])
} else {
    "****".to_string()
};
debug!("Anthropic request: url={}, api_key_preview={}, model={}", ...);
```

前 4 位 + 后 4 位组合（如 `sk-a...1234`）对于已知格式的 API key 可能显著缩小暴力搜索空间。此外，401 错误路径（同文件 214-220 行）也打印了同样的 preview。

**建议**: 仅显示 key 长度或 `****`，不泄露任何实际字符。

---

## Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| High | `AuthSessionManager` 使用 `Mutex<HashMap>` 而非 `DashMap`，每次 validate 都遍历全表 | `crates/server/src/http/auth.rs:89-121` | 高并发下锁竞争，且 token 数量增长后 validate 性能退化 |
| Medium | `LlmAccumulator::finish` 中 JSON 解析失败时静默降级为原始字符串 | `crates/adapter-llm/src/lib.rs:474-486` | 工具参数被错误地包装为 `Value::String` 传递到下游，可能导致下游工具收到意外格式 |
| Medium | `ToolSearchIndex::replace_from_specs` 写锁失败时静默丢弃数据 | `crates/adapter-tools/src/builtin_tools/tool_search.rs:49` | RwLock poison 后搜索索引永久失效，但不会报错 |

### [Quality-001] High: AuthSessionManager 锁竞争

```rust
// crates/server/src/http/auth.rs:89-91
pub(crate) struct AuthSessionManager {
    tokens: Mutex<HashMap<String, i64>>,
}
```

每次 `validate` 调用（即每个需要认证的请求）都会：
1. 获取 Mutex 锁
2. 遍历整个 HashMap 做过期清理
3. 再遍历一遍做 token 匹配

这意味着：a) 每个 API 请求都持有全局锁；b) cleanup 是 O(n) 操作；c) 随着签发的 token 数量增长，性能持续退化。

**建议**: 使用 `DashMap` 或至少将 cleanup 与 lookup 分离（如每隔 N 次 validate 才做一次 cleanup）。

### [Quality-002] Medium: 工具参数静默降级

```rust
// crates/adapter-llm/src/lib.rs:474-486
let args = match serde_json::from_str(&call.arguments) {
    Ok(value) => value,
    Err(error) => {
        warn!("failed to parse tool call '{}' arguments as JSON: {}, falling back to raw string", ...);
        Value::String(call.arguments)  // 静默降级
    },
};
```

当 LLM 返回的 tool_call arguments 不是合法 JSON 时，整个参数被包装为 `Value::String`。下游工具收到的不是预期的 object 而是 `"{\"query\": ...}"` 这样的字符串。虽然有 warn 日志，但没有错误传播或结构化通知。

### [Quality-003] Medium: ToolSearchIndex 静默丢弃

```rust
// crates/adapter-tools/src/builtin_tools/tool_search.rs:49
if let Ok(mut guard) = self.specs.write() {
    *guard = external;
}
// 写锁失败时静默跳过
```

RwLock poison 后搜索索引永久失效但不会报错。项目中 `AuthSessionManager` 使用 `expect("lock poisoned")` 策略，而这里静默忽略，两种 poison 处理策略不一致。

---

## Tests

**概况**: 项目测试覆盖较好，228 个文件中 954 处 `#[test]` / `#[cfg(test)]` 标记。关键模块如 `core`、`adapter-llm`、`adapter-storage`、`session-runtime` 都有对应测试。

**缺失的测试场景**:

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| Medium | `serve_run_info` 端点无测试（过期 token / 文件不存在 / 格式错误等分支） | `crates/server/src/bootstrap/mod.rs:118` |
| Medium | `delete_project` 路由无测试（working_dir 参数验证） | `crates/server/src/http/routes/sessions/mutation.rs:191` |
| Low | `AuthSessionManager` 无并发测试（多线程同时 issue + validate） | `crates/server/src/http/auth.rs:89` |
| Low | MCP stdio transport 的 `send_request` 无超时测试（MCP server 卡死不返回时会永久阻塞） | `crates/adapter-mcp/src/transport/stdio.rs:118-163` |

### MCP stdio transport 阻塞风险

```rust
// crates/adapter-mcp/src/transport/stdio.rs:148-163
loop {
    let line = stdout.next_line().await...;  // 无超时
    if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&line) {
        return Ok(response);
    }
    // 非 JSON-RPC 行被跳过
}
```

这个循环没有超时机制，也没有检查 `cancel` token。如果 MCP server 持续输出非 JSON-RPC 行（如大量日志），该调用会永久阻塞。

---

## Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| High | `adapter-mcp` 依赖 `adapter-prompt`，违反 `adapter-* -> core` 的单向依赖规则 | `crates/adapter-mcp/Cargo.toml:10` |
| Medium | `server` 直接依赖 `session-runtime` 构造 `ForkPoint` enum，绕过 application 层 | `crates/server/src/http/routes/sessions/mutation.rs:137-140` |
| Low | `core` 依赖 `reqwest`（网络 HTTP 客户端），但 core 的职责是领域模型和端口 | `crates/core/Cargo.toml:18` |

### [Arch-001] High: adapter-mcp 依赖 adapter-prompt

```
# crates/adapter-mcp/Cargo.toml
astrcode-adapter-prompt = { path = "../adapter-prompt" }
```

`PROJECT_ARCHITECTURE.md` 明确规定：`adapter-* -> core`。`adapter-mcp` 依赖 `adapter-prompt` 打破了这一规则，意味着 adapter 层之间存在耦合。如果 `adapter-prompt` 的 API 发生变更，`adapter-mcp` 也需要同步修改，而 crate boundary checker (`check-crate-boundaries.mjs --strict`) 当前未检测到这种横向依赖。

**注意**: `check-crate-boundaries.mjs --strict` 输出了 `crate boundary check passed`，说明脚本可能只检查了 `application -> adapter-*` 的禁止规则，而未检查 `adapter-* -> adapter-*` 的禁止规则。

### [Arch-002] Medium: server 直接使用 session-runtime 类型

```rust
// crates/server/src/http/routes/sessions/mutation.rs:137-140
let fork_point = match (request.turn_id, request.storage_seq) {
    (Some(turn_id), None) => astrcode_session_runtime::ForkPoint::TurnEnd(turn_id),
    (None, Some(storage_seq)) => astrcode_session_runtime::ForkPoint::StorageSeq(storage_seq),
    (None, None) => astrcode_session_runtime::ForkPoint::Latest,
    ...
};
```

`server` 层本应只依赖 `application` 的公共 API，但这里直接引用了 `session-runtime` 的 `ForkPoint` 类型。按照架构约束 `server -> application + protocol`，对 `session-runtime` 的直接类型依赖应通过 `application` 层的端口或类型重导出来间接访问。

### [Arch-003] Low: core 依赖 reqwest

`core` 的定位是 "领域模型、强类型 ID、端口 trait"，但它直接依赖了 `reqwest`（HTTP 客户端）。这个依赖可能来源于 `core` 中定义了某些与 HTTP 相关的类型，但从架构角度看，网络相关类型应属于 `protocol` 或 `adapter-*` 层。

---

## Must Fix Before Merge (Critical/High)

1. **[SEC-001]** `/__astrcode__/run-info` 无认证 -- `crates/server/src/bootstrap/mod.rs:118`
   - Impact: 本地任意进程可获取 bootstrap token 并完全控制 API
   - Fix: 至少验证 Origin 头匹配允许列表，或在生产构建中关闭此端点
   - **注意**: 这是设计意图（dev-only），但需要显式的安全缓解措施

2. **[SEC-002]** `delete_project` 未验证 `working_dir` -- `crates/server/src/http/routes/sessions/mutation.rs:191`
   - Impact: 认证后可传入任意 working_dir 删除对应项目的所有 session
   - Fix: 添加路径验证（类似 `validate_session_path_id` 的模式）

3. **[Arch-001]** `adapter-mcp` 依赖 `adapter-prompt` 违反架构约束 -- `crates/adapter-mcp/Cargo.toml:10`
   - Impact: adapter 层横向耦合，降低可替换性
   - Fix: 将共享抽象抽取到 `core` 的端口中，或更新 crate boundary checker 规则以显式禁止 adapter 间依赖

4. **[Quality-001]** `AuthSessionManager` 全局 Mutex 锁竞争 -- `crates/server/src/http/auth.rs:89`
   - Impact: 高并发场景下每个 API 请求都争抢同一把锁
   - Fix: 替换为 `DashMap`；将 cleanup 操作与 lookup 分离

---

## Pre-Existing Issues (not blocking)

- `core` 依赖 `reqwest`（`crates/core/Cargo.toml:18`）-- 架构上不够干净，但影响有限
- `server` 直接使用 `session-runtime::ForkPoint` 类型 -- 绕过了 application 层的抽象
- MCP stdio transport 的 `send_request` 无超时（`crates/adapter-mcp/src/transport/stdio.rs:148`）-- MCP server 卡死时会永久阻塞
- Poison 处理策略不一致：`AuthSessionManager` 用 `expect`，`ToolSearchIndex` 静默忽略

---

## Low-Confidence Observations

- **`secure_token_eq` 的常量时间声称**: 函数注释说"确保比较时间与输入内容无关"，但长度不同的字符串会在 `left.len() ^ right.len()` 后设置一个非零 `diff`，后续循环仍然遍历最大长度。这在数学上是正确的常量时间实现，但编译器优化可能引入提前退出。实际风险极低（localhost-only）。
- **`LlmAccumulator` 中 `tool_calls` 使用 `HashMap<usize, AccToolCall>`** (`crates/adapter-llm/src/lib.rs:427`): 如果 LLM 返回的 `ToolCallDelta` 中 index 不连续（如 0, 2, 5），中间的空位不会被填充。这不太可能是实际 bug（因为 `finish` 时排序后直接 map），但值得注意。

---

## Positive Findings (worth preserving)

1. **路径穿越防御**: session ID 有严格的字符白名单（`is_valid_session_id`），HTTP 路由层也有对应的 `validate_session_path_id`，且存储层每处入口都调用 `validated_session_id`。三层防御做得很好。
2. **认证安全**: `secure_token_eq` 使用常量时间比较防止时序攻击；token 有过期时间；bootstrap token 与 API session token 分离。
3. **错误类型层次**: `AstrError`（core）-> `ApplicationError`（application）-> `ApiError`（server）的三层错误转换设计清晰，`From` 实现完备。
4. **组合根设计**: `bootstrap/runtime.rs` 是唯一的业务组装点，所有依赖通过构造函数注入，handler 只依赖 `App` trait。架构意图明确且实际落地一致。
5. **端口契约**: `core/ports.rs` 中的 trait 设计干净（`LlmProvider`、`EventStore`、`PromptProvider` 等），依赖倒置做得彻底 -- `adapter-llm`、`adapter-storage` 等生产代码中无 `unwrap()`。
6. **配置安全**: API key 支持环境变量引用（`env:NAME`）和字面值（`literal:value`），避免了在配置文件中存储明文密钥。`Debug` 实现中 API key 已 redacted 为 `<redacted>`。
7. **文件工具安全**: `fs_common.rs` 有 UNC 路径检查（防止 NTLM 凭据泄露）和符号链接检测，防御意识到位。
