# Contract: McpToolBridge

**Feature**: 009-mcp-integration
**Purpose**: MCP 工具到 Astrcode CapabilityInvoker 的桥接层，负责协议转换、结果映射和取消信号传递

## 接口签名

```rust
/// MCP 工具桥接适配器。
///
/// 将单个 MCP 工具转换为 Astrcode 的 CapabilityInvoker，
/// 负责 JSON-RPC 请求构造、响应解析和结果大小控制。
///
/// 桥接路径: McpToolBridge (impl CapabilityInvoker)
///           → 通过 McpConnectionManager 统一注册到 RuntimeSurfaceContribution
pub struct McpToolBridge {
    server_name: String,
    tool_name: String,
    fully_qualified_name: String,  // "mcp__{server}__{tool}"
    description: String,
    input_schema: Value,
    annotations: McpToolAnnotations,
    client: Arc<McpClient>,
}

#[async_trait]
impl CapabilityInvoker for McpToolBridge {
    fn descriptor(&self) -> CapabilityDescriptor {
        // 从 fully_qualified_name + description + input_schema 构建 CapabilityDescriptor
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        // 1. 从 ctx.cancel 提取取消信号，传递到 McpClient::call_tool
        // 2. 调用 MCP tools/call
        // 3. 将 MCP 响应映射为 CapabilityExecutionResult（见映射表）
    }
}
```

## CancelToken 流转路径

```
Agent Loop
  → CapabilityRouter::invoke(payload, CapabilityContext { cancel, ... })
    → McpToolBridge::invoke(payload, ctx)
      → tokio::select! {
          // 正常调用路径
          result = self.client.call_tool(tool_name, payload, ctx.cancel.clone()) => { ... }
          // 取消监听路径：cancel token 被触发
          _ = ctx.cancel.cancelled() => {
              self.client.send_cancel(request_id, Some("cancelled by user")).await;
              // 等待服务器响应（30 秒超时），超时则强制断开
              return Err(AstrError::Cancelled);
          }
        }
```

**关键约束**:
- `ctx.cancel` (来自 `CapabilityContext`) 直接 clone 后传入 `McpClient::call_tool`
- MCP 协议的 `notifications/cancelled` 在 cancel token 触发后由客户端自动发送
- 若服务器 30 秒内未响应取消通知，McpTransport 强制关闭连接

## MCP 结果 → CapabilityExecutionResult 映射表

MCP `tools/call` 返回结果结构:

```json
{
  "content": [
    { "type": "text", "text": "..." },
    { "type": "image", "data": "...", "mimeType": "image/png" },
    { "type": "resource", "resource": { "uri": "...", "mimeType": "...", "text": "..." } }
  ],
  "isError": false
}
```

### 映射规则

| MCP content 类型 | 映射目标 | 说明 |
|-----------------|---------|------|
| `text` | `CapabilityExecutionResult.output` | 文本内容序列化为 JSON string |
| 多个 `text` | `CapabilityExecutionResult.output` | 拼接为 JSON 数组 `["text1", "text2"]` |
| `image` | `CapabilityExecutionResult.output` | 持久化到磁盘，output 中返回文件路径 JSON `{"type":"image","path":"...","mimeType":"..."}` |
| `resource` | `CapabilityExecutionResult.output` | 嵌入为 JSON 对象 `{"type":"resource","uri":"...","content":"..."}` |
| `isError: true` | `CapabilityExecutionResult { success: false, error: Some(...) }` | content 中的文本作为 error 消息 |
| `isError: false` | `CapabilityExecutionResult { success: true }` | 正常结果 |

### 输出大小控制

复用已有的 `TOOL_RESULT_INLINE_LIMIT` 机制:

1. 所有 `text` content 拼接后检查总大小
2. 超过 `TOOL_RESULT_INLINE_LIMIT` 时:
   - 将完整结果持久化到磁盘（session 工具结果目录）
   - `output` 中返回读取指引 `{"type":"file","path":"...","truncated":true}`
   - `truncated = true`
3. `image` 和 `resource` 始终按需持久化，不计入 inline limit

### 错误映射

| MCP 场景 | CapabilityExecutionResult |
|---------|--------------------------|
| `isError: true` + 有 content | `success: false`, `error: content文本` |
| `isError: true` + 无 content | `success: false`, `error: "MCP tool returned error without message"` |
| 网络超时 | `success: false`, `error: "MCP server timeout: {server_name}"` |
| 连接断开 | `success: false`, `error: "MCP server disconnected: {server_name}"` |
| 取消 | 返回 `Err(AstrError::Cancelled)` 而非 CapabilityExecutionResult |
| 协议错误（非 JSON 响应） | `success: false`, `error: "MCP protocol error: {detail}"` |

## Annotations 映射

| MCP Annotation | Astrcode 字段 | 说明 |
|---------------|--------------|------|
| `readOnlyHint: true` | `ToolCapabilityMetadata.concurrency_safe = true` | 只读工具可并行调用 |
| `destructiveHint: true` | `ToolCapabilityMetadata.side_effect = SideEffectLevel::High` | 破坏性操作需要用户确认 |
| `openWorldHint: true` | `ToolCapabilityMetadata.permissions.push(PermissionHint::Network)` | 访问外部资源的提示 |

**默认值**: 未提供 annotations 时，所有字段使用保守默认值（`concurrency_safe: false`, `side_effect: Medium`, 无额外 permissions）。
