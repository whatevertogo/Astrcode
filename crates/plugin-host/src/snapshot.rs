use astrcode_core::{CapabilitySpec, GovernanceModeSpec};

use crate::descriptor::{
    CommandDescriptor, HookDescriptor, PluginDescriptor, PromptDescriptor, ProviderDescriptor,
    ResourceDescriptor, SkillDescriptor, ThemeDescriptor,
};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PluginActiveSnapshot {
    pub snapshot_id: String,
    pub revision: u64,
    pub plugin_ids: Vec<String>,
    pub tools: Vec<CapabilitySpec>,
    pub hooks: Vec<HookDescriptor>,
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
        });

        let snapshot =
            PluginActiveSnapshot::from_descriptors(7, "snapshot-7", &[descriptor.clone()]);

        assert_eq!(snapshot.revision, 7);
        assert_eq!(snapshot.snapshot_id, "snapshot-7");
        assert_eq!(snapshot.plugin_ids, vec![descriptor.plugin_id]);
        assert_eq!(snapshot.hooks.len(), 1);
    }
}
