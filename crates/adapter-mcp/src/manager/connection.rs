//! # MCP 连接状态机
//!
//! 定义与单个 MCP 服务器的连接状态和状态转换逻辑。
//! McpConnection 是 McpConnectionManager 的内部依赖。

use serde::{Deserialize, Serialize};

use crate::protocol::types::McpServerCapabilities;

/// MCP 连接状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpConnectionState {
    /// 初始状态，等待连接。
    Pending,
    /// 正在握手。
    Connecting,
    /// 已连接，可调用。
    Connected,
    /// 连接或调用失败，含错误信息。
    Failed(String),
    /// 远程服务器需要认证。
    NeedsAuth,
    /// 用户手动禁用。
    Disabled,
}

impl std::fmt::Display for McpConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Connecting => write!(f, "connecting"),
            Self::Connected => write!(f, "connected"),
            Self::Failed(e) => write!(f, "failed: {}", e),
            Self::NeedsAuth => write!(f, "needs_auth"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

/// 与 MCP 服务器的活动连接信息。
pub struct McpConnection {
    /// 服务器名称。
    pub name: String,
    /// 当前状态。
    pub state: McpConnectionState,
    /// 握手后获取的服务器能力。
    pub capabilities: Option<McpServerCapabilities>,
    /// 服务器提供的 instructions。
    pub instructions: Option<String>,
    /// 当前重连尝试次数。
    pub reconnect_attempt: u32,
}

impl McpConnection {
    /// 创建新的连接（初始状态为 Pending）。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: McpConnectionState::Pending,
            capabilities: None,
            instructions: None,
            reconnect_attempt: 0,
        }
    }

    /// 转换到 Connecting 状态。
    pub fn start_connecting(&mut self) {
        self.state = McpConnectionState::Connecting;
    }

    /// 连接成功，记录握手结果。
    pub fn mark_connected(
        &mut self,
        capabilities: McpServerCapabilities,
        instructions: Option<String>,
    ) {
        self.state = McpConnectionState::Connected;
        self.capabilities = Some(capabilities);
        self.instructions = instructions;
        self.reconnect_attempt = 0;
    }

    /// 连接失败。
    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.state = McpConnectionState::Failed(reason.into());
    }

    /// 需要认证。
    pub fn mark_needs_auth(&mut self) {
        self.state = McpConnectionState::NeedsAuth;
    }

    /// 用户禁用。
    pub fn mark_disabled(&mut self) {
        self.state = McpConnectionState::Disabled;
    }

    /// 准备重连（回到 Pending）。
    pub fn prepare_reconnect(&mut self) {
        self.reconnect_attempt += 1;
        self.state = McpConnectionState::Pending;
    }

    /// 是否处于可调用状态。
    pub fn is_connected(&self) -> bool {
        matches!(self.state, McpConnectionState::Connected)
    }

    /// 是否已被禁用。
    pub fn is_disabled(&self) -> bool {
        matches!(self.state, McpConnectionState::Disabled)
    }

    /// 是否需要重连（仅远程传输且未禁用）。
    pub fn should_reconnect(&self, is_remote: bool) -> bool {
        is_remote
            && !self.is_disabled()
            && matches!(self.state, McpConnectionState::Failed(_))
            && self.reconnect_attempt < 5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::{McpServerCapabilities, ToolsCapability};

    #[test]
    fn test_state_transitions() {
        let mut conn = McpConnection::new("test-server");

        assert_eq!(conn.state, McpConnectionState::Pending);
        assert!(!conn.is_connected());

        // Pending → Connecting → Connected
        conn.start_connecting();
        assert_eq!(conn.state, McpConnectionState::Connecting);

        let caps = McpServerCapabilities {
            tools: Some(ToolsCapability { list_changed: true }),
            prompts: None,
            resources: None,
            logging: None,
            experimental: None,
        };
        conn.mark_connected(caps, Some("test instructions".to_string()));
        assert!(conn.is_connected());
        assert_eq!(conn.instructions.as_deref(), Some("test instructions"));
        assert_eq!(conn.reconnect_attempt, 0);

        // Connected → Failed → Pending (reconnect)
        conn.mark_failed("connection lost");
        assert!(matches!(conn.state, McpConnectionState::Failed(_)));
        assert!(conn.should_reconnect(true));

        conn.prepare_reconnect();
        assert_eq!(conn.state, McpConnectionState::Pending);
        assert_eq!(conn.reconnect_attempt, 1);
    }

    #[test]
    fn test_should_not_reconnect_stdio() {
        let mut conn = McpConnection::new("stdio-server");
        conn.mark_failed("process crashed");
        // stdio 不重连
        assert!(!conn.should_reconnect(false));
    }

    #[test]
    fn test_should_not_reconnect_disabled() {
        let mut conn = McpConnection::new("remote-server");
        conn.mark_disabled();
        assert!(!conn.should_reconnect(true));
    }

    #[test]
    fn test_max_reconnect_attempts() {
        let mut conn = McpConnection::new("remote-server");
        for _ in 0..5 {
            conn.mark_failed("error");
            conn.prepare_reconnect();
        }
        // 超过 5 次不再重连
        conn.mark_failed("error");
        assert!(!conn.should_reconnect(true));
    }

    #[test]
    fn test_display() {
        assert_eq!(McpConnectionState::Pending.to_string(), "pending");
        assert_eq!(McpConnectionState::Connected.to_string(), "connected");
        assert_eq!(McpConnectionState::Disabled.to_string(), "disabled");
        assert_eq!(
            McpConnectionState::Failed("timeout".to_string()).to_string(),
            "failed: timeout"
        );
    }
}
