use std::time::{SystemTime, UNIX_EPOCH};

use astrcode_protocol::plugin::{
    CapabilityWireDescriptor, InitializeMessage, InitializeResultData, PROTOCOL_VERSION,
    PeerDescriptor, PeerRole, ProfileDescriptor,
};

/// `plugin-host` 持有的最小握手状态。
///
/// 第一阶段只固化：
/// - 宿主发给插件的 initialize 载荷
/// - 插件回传的 initialize 结果
/// - 最终协商出的协议版本
///
/// 这样后续迁入 `peer/supervisor` 时，不需要再把握手真相塞回旧 crate。
#[derive(Debug, Clone, PartialEq)]
pub struct PluginInitializeState {
    pub local_initialize: InitializeMessage,
    pub remote_initialize: Option<InitializeResultData>,
}

/// 宿主可直接消费的远端握手摘要。
///
/// 它不是协议原文，而是 `plugin-host` 对远端 initialize 结果的只读稳定视图，
/// 用于后续统一装配 active runtime surface。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePluginHandshakeSummary {
    pub protocol_version: String,
    pub peer_id: String,
    pub peer_name: String,
    pub capability_names: Vec<String>,
    pub profile_names: Vec<String>,
    pub skill_ids: Vec<String>,
    pub mode_ids: Vec<String>,
}

impl PluginInitializeState {
    pub fn new(local_initialize: InitializeMessage) -> Self {
        Self {
            local_initialize,
            remote_initialize: None,
        }
    }

    pub fn with_defaults(
        local_peer: PeerDescriptor,
        capabilities: Vec<CapabilityWireDescriptor>,
    ) -> Self {
        Self::new(default_initialize_message(
            local_peer,
            capabilities,
            default_profiles(),
        ))
    }

    pub fn record_remote_initialize(
        &mut self,
        remote_initialize: InitializeResultData,
    ) -> &InitializeResultData {
        self.remote_initialize = Some(remote_initialize);
        self.remote_initialize
            .as_ref()
            .expect("remote initialize should exist immediately after record")
    }

    pub fn negotiated_protocol_version(&self) -> &str {
        self.remote_initialize
            .as_ref()
            .map(|remote| remote.protocol_version.as_str())
            .unwrap_or_else(|| self.local_initialize.protocol_version.as_str())
    }

    pub fn remote_handshake_summary(&self) -> Option<RemotePluginHandshakeSummary> {
        self.remote_initialize
            .as_ref()
            .map(RemotePluginHandshakeSummary::from_remote_initialize)
    }
}

impl RemotePluginHandshakeSummary {
    pub fn from_remote_initialize(remote: &InitializeResultData) -> Self {
        Self {
            protocol_version: remote.protocol_version.clone(),
            peer_id: remote.peer.id.clone(),
            peer_name: remote.peer.name.clone(),
            capability_names: remote
                .capabilities
                .iter()
                .map(|capability| capability.name.to_string())
                .collect(),
            profile_names: remote
                .profiles
                .iter()
                .map(|profile| profile.name.clone())
                .collect(),
            skill_ids: remote
                .skills
                .iter()
                .map(|skill| skill.name.clone())
                .collect(),
            mode_ids: remote
                .modes
                .iter()
                .map(|mode| mode.id.to_string())
                .collect(),
        }
    }
}

/// 构建 `plugin-host` 默认使用的 initialize 载荷。
///
/// 这里只保留最小稳态：
/// - 单一协议版本入口
/// - 空 handlers
/// - `stdio` transport metadata
pub fn default_initialize_message(
    local_peer: PeerDescriptor,
    capabilities: Vec<CapabilityWireDescriptor>,
    profiles: Vec<ProfileDescriptor>,
) -> InitializeMessage {
    InitializeMessage {
        id: format!("plugin-host-init-{}", now_unix_ms()),
        protocol_version: PROTOCOL_VERSION.to_string(),
        supported_protocol_versions: vec![PROTOCOL_VERSION.to_string()],
        peer: local_peer,
        capabilities,
        handlers: Vec::new(),
        profiles,
        metadata: serde_json::json!({ "transport": "stdio" }),
    }
}

/// 构建 `plugin-host` 默认使用的本地 peer 描述。
///
/// 第一阶段先给新宿主一个稳定、可测试的身份，
/// 避免 external runtime handle 在 reload 后还没有本地握手上下文。
pub fn default_local_peer_descriptor() -> PeerDescriptor {
    PeerDescriptor {
        id: "plugin-host".to_string(),
        name: "plugin-host".to_string(),
        role: PeerRole::Supervisor,
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_profiles: vec!["coding".to_string()],
        metadata: serde_json::json!({
            "owner": "plugin-host",
            "transport": "stdio"
        }),
    }
}

/// `plugin-host` 当前默认暴露的 profile。
///
/// 暂时只保留 `coding`，和旧插件 supervisor 的最小默认值一致，
/// 但 owner 已经迁到 `plugin-host`。
pub fn default_profiles() -> Vec<ProfileDescriptor> {
    vec![ProfileDescriptor {
        name: "coding".to_string(),
        version: "1".to_string(),
        description: "Coding workflow profile".to_string(),
        context_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "workingDir": { "type": "string" },
                "repoRoot": { "type": "string" },
                "openFiles": { "type": "array", "items": { "type": "string" } },
                "activeFile": { "type": "string" },
                "selection": { "type": "object" },
                "approvalMode": { "type": "string" }
            }
        }),
        metadata: serde_json::Value::Null,
    }]
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use astrcode_protocol::plugin::{CapabilityKind, PeerRole};
    use serde_json::json;

    use super::{
        PluginInitializeState, default_initialize_message, default_local_peer_descriptor,
        default_profiles,
    };

    fn sample_peer() -> astrcode_protocol::plugin::PeerDescriptor {
        let mut peer = default_local_peer_descriptor();
        peer.id = "host-1".to_string();
        peer
    }

    #[test]
    fn default_initialize_message_uses_protocol_defaults() {
        let capability = astrcode_protocol::plugin::CapabilityWireDescriptor::builder(
            "tool.echo",
            CapabilityKind::tool(),
        )
        .description("Echo the input")
        .schema(json!({ "type": "object" }), json!({ "type": "object" }))
        .build()
        .expect("capability should build");

        let message =
            default_initialize_message(sample_peer(), vec![capability.clone()], default_profiles());

        assert!(message.id.starts_with("plugin-host-init-"));
        assert_eq!(message.protocol_version, "5");
        assert_eq!(message.supported_protocol_versions, vec!["5".to_string()]);
        assert_eq!(message.capabilities, vec![capability]);
        assert!(message.handlers.is_empty());
        assert_eq!(message.metadata["transport"], "stdio");
    }

    #[test]
    fn default_profiles_exposes_single_coding_profile() {
        let profiles = default_profiles();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "coding");
        assert_eq!(profiles[0].version, "1");
        assert_eq!(profiles[0].context_schema["type"], "object");
    }

    #[test]
    fn initialize_state_records_remote_handshake() {
        let local_peer = sample_peer();
        let local = default_initialize_message(local_peer.clone(), Vec::new(), default_profiles());
        let mut state = PluginInitializeState::new(local.clone());

        assert_eq!(state.negotiated_protocol_version(), "5");

        let recorded =
            state.record_remote_initialize(astrcode_protocol::plugin::InitializeResultData {
                protocol_version: "5".to_string(),
                peer: local_peer,
                capabilities: Vec::new(),
                handlers: Vec::new(),
                profiles: default_profiles(),
                skills: Vec::new(),
                modes: Vec::new(),
                metadata: serde_json::Value::Null,
            });

        assert_eq!(recorded.protocol_version, "5");
        assert_eq!(state.negotiated_protocol_version(), "5");
        assert!(state.remote_initialize.is_some());
        assert_eq!(state.local_initialize, local);
        let summary = state
            .remote_handshake_summary()
            .expect("remote handshake summary should exist");
        assert_eq!(summary.peer_id, "host-1");
        assert_eq!(summary.profile_names, vec!["coding".to_string()]);
    }

    #[test]
    fn default_local_peer_descriptor_marks_plugin_host_owner() {
        let peer = default_local_peer_descriptor();

        assert_eq!(peer.id, "plugin-host");
        assert_eq!(peer.name, "plugin-host");
        assert_eq!(peer.role, PeerRole::Supervisor);
        assert_eq!(peer.supported_profiles, vec!["coding".to_string()]);
        assert_eq!(peer.metadata["owner"], "plugin-host");
        assert_eq!(peer.metadata["transport"], "stdio");
    }
}
