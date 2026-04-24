use astrcode_core::{CapabilityKind, CapabilitySpec, InvocationMode, SideEffect, Stability};

use crate::PluginDescriptor;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolContributionCatalog {
    pub tool_names: Vec<String>,
}

pub fn builtin_tools_descriptor(
    plugin_id: impl Into<String>,
    display_name: impl Into<String>,
    tools: Vec<CapabilitySpec>,
) -> PluginDescriptor {
    let mut descriptor = PluginDescriptor::builtin(plugin_id, display_name);
    descriptor.tools = tools;
    descriptor
}

pub fn builtin_collaboration_tools_descriptor() -> PluginDescriptor {
    builtin_tools_descriptor(
        "builtin-collaboration-tools",
        "Builtin Collaboration Tools",
        vec![
            host_session_tool(
                "spawn_agent",
                "Spawn a child session and record parent/child lineage through host-session.",
            ),
            host_session_tool(
                "send_to_child",
                "Deliver an input from a parent session to a direct child session.",
            ),
            host_session_tool(
                "send_to_parent",
                "Deliver a typed result from a child session back to its direct parent.",
            ),
            host_session_tool(
                "observe_subtree",
                "Read the host-session collaboration subtree projection.",
            ),
            host_session_tool(
                "terminate_subtree",
                "Terminate a session subtree through the host-session owner.",
            ),
        ],
    )
}

fn host_session_tool(name: &str, description: &str) -> CapabilitySpec {
    CapabilitySpec {
        name: name.into(),
        kind: CapabilityKind::Tool,
        description: description.to_string(),
        input_schema: Default::default(),
        output_schema: Default::default(),
        invocation_mode: InvocationMode::Unary,
        concurrency_safe: false,
        compact_clearable: true,
        profiles: vec!["coding".to_string()],
        tags: vec!["collaboration".to_string(), "host-session".to_string()],
        permissions: Vec::new(),
        side_effect: SideEffect::Workspace,
        stability: Stability::Experimental,
        metadata: Default::default(),
        max_result_inline_size: None,
    }
}

pub fn tool_contribution_catalog(descriptors: &[PluginDescriptor]) -> ToolContributionCatalog {
    ToolContributionCatalog {
        tool_names: descriptors
            .iter()
            .flat_map(|descriptor| descriptor.tools.iter())
            .map(|tool| tool.name.to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilityKind, CapabilitySpec, InvocationMode, SideEffect, Stability};

    use super::{
        builtin_collaboration_tools_descriptor, builtin_tools_descriptor, tool_contribution_catalog,
    };

    fn capability(name: &str) -> CapabilitySpec {
        CapabilitySpec {
            name: name.into(),
            kind: CapabilityKind::Tool,
            description: format!("{name} capability"),
            input_schema: Default::default(),
            output_schema: Default::default(),
            invocation_mode: InvocationMode::Unary,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffect::None,
            stability: Stability::Stable,
            metadata: Default::default(),
            max_result_inline_size: None,
        }
    }

    #[test]
    fn builtin_tools_are_represented_as_plugin_descriptor_tools() {
        let descriptor = builtin_tools_descriptor(
            "builtin-core-tools",
            "Builtin Core Tools",
            vec![capability("readFile"), capability("writeFile")],
        );

        assert_eq!(descriptor.plugin_id, "builtin-core-tools");
        assert_eq!(
            descriptor
                .tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect::<Vec<_>>(),
            vec!["readFile".to_string(), "writeFile".to_string()]
        );
    }

    #[test]
    fn tool_catalog_flattens_mcp_and_builtin_descriptor_tools() {
        let builtin = builtin_tools_descriptor(
            "builtin-core-tools",
            "Builtin Core Tools",
            vec![capability("readFile")],
        );
        let mcp = builtin_tools_descriptor("mcp-tools", "MCP Tools", vec![capability("mcp.echo")]);

        let catalog = tool_contribution_catalog(&[builtin, mcp]);

        assert_eq!(
            catalog.tool_names,
            vec!["readFile".to_string(), "mcp.echo".to_string()]
        );
    }

    #[test]
    fn collaboration_entrypoints_are_declared_as_builtin_plugin_tools() {
        let descriptor = builtin_collaboration_tools_descriptor();

        assert_eq!(descriptor.plugin_id, "builtin-collaboration-tools");
        assert_eq!(
            descriptor
                .tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect::<Vec<_>>(),
            vec![
                "spawn_agent".to_string(),
                "send_to_child".to_string(),
                "send_to_parent".to_string(),
                "observe_subtree".to_string(),
                "terminate_subtree".to_string(),
            ]
        );
        assert!(
            descriptor
                .tools
                .iter()
                .all(|tool| tool.tags.iter().any(|tag| tag == "host-session"))
        );
    }
}
