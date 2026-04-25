use astrcode_core::{AstrError, CapabilitySpec, HookEventKey, Result};
use astrcode_governance_contract::GovernanceModeSpec;

use crate::{
    builtin_hooks::{BuiltinHookRegistry, HookBinding, HookExecutorRef},
    descriptor::{
        CommandDescriptor, HookDescriptor, PluginDescriptor, PromptDescriptor, ProviderDescriptor,
        ResourceDescriptor, SkillDescriptor, ThemeDescriptor,
    },
};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PluginActiveSnapshot {
    pub snapshot_id: String,
    pub revision: u64,
    pub plugin_ids: Vec<String>,
    pub tools: Vec<CapabilitySpec>,
    pub hooks: Vec<HookDescriptor>,
    /// 可执行的 hook binding（生产路径使用，不包含预计算 effect）。
    pub hook_bindings: Vec<HookBinding>,
    pub providers: Vec<ProviderDescriptor>,
    pub resources: Vec<ResourceDescriptor>,
    pub commands: Vec<CommandDescriptor>,
    pub themes: Vec<ThemeDescriptor>,
    pub prompts: Vec<PromptDescriptor>,
    pub skills: Vec<SkillDescriptor>,
    pub modes: Vec<GovernanceModeSpec>,
}

impl PluginActiveSnapshot {
    pub fn from_descriptors(
        revision: u64,
        snapshot_id: impl Into<String>,
        descriptors: &[PluginDescriptor],
    ) -> Self {
        let mut snapshot = Self {
            snapshot_id: snapshot_id.into(),
            revision,
            plugin_ids: descriptors
                .iter()
                .map(|item| item.plugin_id.clone())
                .collect(),
            ..Self::default()
        };

        for descriptor in descriptors {
            snapshot.tools.extend(descriptor.tools.clone());
            snapshot.hooks.extend(descriptor.hooks.clone());
            snapshot.providers.extend(descriptor.providers.clone());
            snapshot.resources.extend(descriptor.resources.clone());
            snapshot.commands.extend(descriptor.commands.clone());
            snapshot.themes.extend(descriptor.themes.clone());
            snapshot.prompts.extend(descriptor.prompts.clone());
            snapshot.skills.extend(descriptor.skills.clone());
            snapshot.modes.extend(descriptor.modes.clone());
        }

        snapshot
    }

    /// 从 descriptors + registry 构建 snapshot，同时生成 executable hook bindings。
    ///
    /// 校验每个 hook descriptor 的 `entry_ref` 是否能解析到 registry 中的 executor。
    /// 解析失败的 descriptor 会导致整个 staging 失败。
    pub fn from_descriptors_with_registry(
        revision: u64,
        snapshot_id: impl Into<String>,
        descriptors: &[PluginDescriptor],
        registry: &BuiltinHookRegistry,
    ) -> Result<Self> {
        let snapshot_id: String = snapshot_id.into();
        let mut snapshot = Self::from_descriptors(revision, snapshot_id.clone(), descriptors);

        for descriptor in descriptors {
            for hook in &descriptor.hooks {
                let executor = resolve_executor(hook, registry)?;
                let event_key = parse_hook_event_key(&hook.event)?;

                snapshot.hook_bindings.push(HookBinding {
                    plugin_id: descriptor.plugin_id.clone(),
                    hook_id: hook.hook_id.clone(),
                    event: event_key,
                    dispatch_mode: hook.dispatch_mode,
                    failure_policy: hook.failure_policy,
                    priority: hook.priority,
                    executor,
                    snapshot_id: snapshot_id.clone(),
                });
            }
        }

        Ok(snapshot)
    }
}

fn resolve_executor(
    hook: &HookDescriptor,
    registry: &BuiltinHookRegistry,
) -> Result<HookExecutorRef> {
    if hook.entry_ref.is_empty() {
        return Err(AstrError::Validation(format!(
            "hook '{}' has no entry_ref",
            hook.hook_id
        )));
    }

    if hook.entry_ref.starts_with("builtin://") {
        let Some(registered_event) = registry.event_for(&hook.entry_ref) else {
            return Err(AstrError::Validation(format!(
                "hook '{}' entry_ref '{}' not found in builtin registry",
                hook.hook_id, hook.entry_ref
            )));
        };
        let descriptor_event = parse_hook_event_key(&hook.event)?;
        if registered_event != descriptor_event {
            return Err(AstrError::Validation(format!(
                "hook '{}' entry_ref '{}' registered for '{registered_event:?}' but descriptor \
                 uses '{descriptor_event:?}'",
                hook.hook_id, hook.entry_ref
            )));
        }
        Ok(HookExecutorRef::Builtin(hook.entry_ref.clone()))
    } else {
        // External handler ref — validated at backend connection time.
        Ok(HookExecutorRef::External(hook.entry_ref.clone()))
    }
}

fn parse_hook_event_key(event: &str) -> Result<HookEventKey> {
    match event {
        "input" => Ok(HookEventKey::Input),
        "context" => Ok(HookEventKey::Context),
        "before_agent_start" => Ok(HookEventKey::BeforeAgentStart),
        "before_provider_request" => Ok(HookEventKey::BeforeProviderRequest),
        "tool_call" => Ok(HookEventKey::ToolCall),
        "tool_result" => Ok(HookEventKey::ToolResult),
        "turn_start" => Ok(HookEventKey::TurnStart),
        "turn_end" => Ok(HookEventKey::TurnEnd),
        "session_before_compact" => Ok(HookEventKey::SessionBeforeCompact),
        "resources_discover" => Ok(HookEventKey::ResourcesDiscover),
        "model_select" => Ok(HookEventKey::ModelSelect),
        other => Err(AstrError::Validation(format!(
            "unknown hook event key: '{other}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        descriptor::{HookDescriptor, PluginDescriptor},
        snapshot::PluginActiveSnapshot,
    };

    #[test]
    fn snapshot_flattens_plugin_contributions() {
        let mut descriptor = PluginDescriptor::builtin("core-tools", "Core Tools");
        descriptor.hooks.push(HookDescriptor {
            hook_id: "turn-start".to_string(),
            event: "turn_start".to_string(),
            ..Default::default()
        });

        let snapshot =
            PluginActiveSnapshot::from_descriptors(7, "snapshot-7", &[descriptor.clone()]);

        assert_eq!(snapshot.revision, 7);
        assert_eq!(snapshot.snapshot_id, "snapshot-7");
        assert_eq!(snapshot.plugin_ids, vec![descriptor.plugin_id]);
        assert_eq!(snapshot.hooks.len(), 1);
    }
}
