//! # MCP 工具桥接层
//!
//! 将 MCP 服务器提供的工具、prompt、资源桥接到 Astrcode 的能力路由系统。
//! 桥接路径: McpToolBridge (impl CapabilityInvoker) → CapabilityRouter

pub mod prompt_bridge;
pub mod prompt_tool;
pub mod resource_tool;
pub mod tool_bridge;

/// 生成 MCP 工具的全限定名称。
///
/// 格式: `mcp__{server_name}__{tool_name}`
pub fn build_mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    format!("mcp__{}__{}", server_name, tool_name)
}
