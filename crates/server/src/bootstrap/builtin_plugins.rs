//! # 内置插件描述符共享定义
//!
//! runtime.rs（初始引导）和 governance.rs（reload）共用常量与 descriptor builder，
//! 消除两份相同代码的并行维护。

use astrcode_core::mode::GovernanceModeSpec;
use astrcode_plugin_host::{
    CommandDescriptor, HookDescriptor, HookDispatchMode, HookFailurePolicy, HookStage,
    PluginDescriptor, builtin_tools_descriptor,
};

// -- Plugin IDs --

pub const BUILTIN_GOVERNANCE_MODES_PLUGIN_ID: &str = "builtin-governance-modes";
pub const BUILTIN_PERMISSION_PLUGIN_ID: &str = "builtin-permission-policy";
pub const BUILTIN_PLANNING_PLUGIN_ID: &str = "builtin-planning";
pub const EXTERNAL_PLUGIN_MODES_PLUGIN_ID: &str = "external-plugin-modes";

// -- Hook IDs --

pub const PERMISSION_PROVIDER_REQUEST_HOOK_ID: &str =
    "builtin-permission-policy.before-provider-request";
pub const PERMISSION_TOOL_CALL_HOOK_ID: &str = "builtin-permission-policy.tool-call";
pub const PLANNING_INPUT_HOOK_ID: &str = "builtin-planning.input-mode-prefix";

// -- Descriptor builders --

pub fn builtin_composer_plugin_descriptor() -> PluginDescriptor {
    let mut descriptor =
        PluginDescriptor::builtin("builtin-composer-resources", "Builtin Composer Resources");
    descriptor.commands.push(CommandDescriptor {
        command_id: "compact".to_string(),
        entry_ref: "builtin://commands/compact".to_string(),
    });
    descriptor
}

pub fn builtin_permission_descriptor() -> PluginDescriptor {
    let mut descriptor =
        PluginDescriptor::builtin(BUILTIN_PERMISSION_PLUGIN_ID, "Builtin Permission Policy");
    descriptor.hooks.push(HookDescriptor {
        hook_id: PERMISSION_TOOL_CALL_HOOK_ID.to_string(),
        event: "tool_call".to_string(),
        stage: HookStage::Runtime,
        dispatch_mode: HookDispatchMode::Cancellable,
        failure_policy: HookFailurePolicy::FailClosed,
        priority: 500,
        entry_ref: format!("builtin://hooks/{PERMISSION_TOOL_CALL_HOOK_ID}"),
        input_schema: Some("astrcode.hooks.tool-call.v1".to_string()),
        effect_schema: Some("astrcode.hooks.tool-call.effects.v1".to_string()),
    });
    descriptor.hooks.push(HookDescriptor {
        hook_id: PERMISSION_PROVIDER_REQUEST_HOOK_ID.to_string(),
        event: "before_provider_request".to_string(),
        stage: HookStage::Runtime,
        dispatch_mode: HookDispatchMode::Cancellable,
        failure_policy: HookFailurePolicy::FailClosed,
        priority: 500,
        entry_ref: format!("builtin://hooks/{PERMISSION_PROVIDER_REQUEST_HOOK_ID}"),
        input_schema: Some("astrcode.hooks.before-provider-request.v1".to_string()),
        effect_schema: Some("astrcode.hooks.before-provider-request.effects.v1".to_string()),
    });
    descriptor
}

pub fn builtin_planning_descriptor(
    tools: Vec<astrcode_core::CapabilitySpec>,
    modes: Vec<GovernanceModeSpec>,
) -> PluginDescriptor {
    let mut descriptor =
        builtin_tools_descriptor(BUILTIN_PLANNING_PLUGIN_ID, "Builtin Planning", tools);
    descriptor.modes = modes;
    descriptor.hooks.push(HookDescriptor {
        hook_id: PLANNING_INPUT_HOOK_ID.to_string(),
        event: "input".to_string(),
        stage: HookStage::Host,
        dispatch_mode: HookDispatchMode::Sequential,
        failure_policy: HookFailurePolicy::FailOpen,
        priority: 100,
        entry_ref: format!("builtin://hooks/{PLANNING_INPUT_HOOK_ID}"),
        input_schema: Some("astrcode.hooks.input.v1".to_string()),
        effect_schema: Some("astrcode.hooks.input.effects.v1".to_string()),
    });
    descriptor
}

/// 将内置 mode specs 按 plan / 非 plan 分区。
pub fn split_builtin_planning_modes(
    modes: Vec<GovernanceModeSpec>,
) -> (Vec<GovernanceModeSpec>, Vec<GovernanceModeSpec>) {
    modes
        .into_iter()
        .partition(|mode| mode.id.as_str() != "plan")
}

/// 从 descriptor 列表中提取指定 plugin ID 的 modes。
pub fn descriptor_modes(
    descriptors: &[PluginDescriptor],
    plugin_id: &str,
) -> Vec<GovernanceModeSpec> {
    descriptors
        .iter()
        .find(|descriptor| descriptor.plugin_id == plugin_id)
        .map(|descriptor| descriptor.modes.clone())
        .unwrap_or_default()
}

/// 从 descriptor 列表中提取多个 plugin ID 的 modes（合并）。
pub fn descriptor_modes_for_plugin_ids(
    descriptors: &[PluginDescriptor],
    plugin_ids: &[&str],
) -> Vec<GovernanceModeSpec> {
    plugin_ids
        .iter()
        .flat_map(|plugin_id| descriptor_modes(descriptors, plugin_id))
        .collect()
}
