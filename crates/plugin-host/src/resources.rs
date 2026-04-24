use std::collections::BTreeSet;

use astrcode_core::{AstrError, Result, SkillSpec};

use crate::descriptor::{
    CommandDescriptor, PluginDescriptor, PromptDescriptor, ResourceDescriptor, SkillDescriptor,
    ThemeDescriptor, validate_descriptors,
};

/// plugin-host 聚合出的统一资源目录。
///
/// 这一层先只做只读聚合，不负责发现源本身的 watch/reload。
/// 后续 `resources_discover` hooks 接进来时，仍然可以把结果归并到这个 catalog。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceCatalog {
    pub plugin_ids: Vec<String>,
    pub resources: Vec<ResourceDescriptor>,
    pub commands: Vec<CommandDescriptor>,
    pub themes: Vec<ThemeDescriptor>,
    pub prompts: Vec<PromptDescriptor>,
    pub skills: Vec<SkillDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiscoverReport {
    pub descriptor_count: usize,
    pub plugin_ids: Vec<String>,
    pub catalog: ResourceCatalog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCatalogBaseBuild {
    pub base_skills: Vec<SkillSpec>,
    pub plugin_skill_count: usize,
    pub descriptor_skill_ids: Vec<String>,
}

pub fn resources_discover(descriptors: &[PluginDescriptor]) -> Result<ResourceDiscoverReport> {
    validate_descriptors(descriptors)?;
    let mut catalog = ResourceCatalog::default();
    catalog.extend_from_descriptors(descriptors)?;
    Ok(ResourceDiscoverReport {
        descriptor_count: descriptors.len(),
        plugin_ids: catalog.plugin_ids.clone(),
        catalog,
    })
}

pub fn build_skill_catalog_base(
    mut builtin_skills: Vec<SkillSpec>,
    mut plugin_skills: Vec<SkillSpec>,
    resource_catalog: &ResourceCatalog,
) -> SkillCatalogBaseBuild {
    let plugin_skill_count = plugin_skills.len();
    builtin_skills.append(&mut plugin_skills);
    SkillCatalogBaseBuild {
        base_skills: builtin_skills,
        plugin_skill_count,
        descriptor_skill_ids: resource_catalog
            .skills
            .iter()
            .map(|skill| skill.skill_id.clone())
            .collect(),
    }
}

impl ResourceCatalog {
    pub fn from_descriptors(descriptors: &[PluginDescriptor]) -> Self {
        resources_discover(descriptors)
            .expect("resource discovery should stay consistent for validated descriptors")
            .catalog
    }

    pub fn extend_from_descriptors(&mut self, descriptors: &[PluginDescriptor]) -> Result<()> {
        let mut plugin_ids = self.plugin_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut resource_ids = self
            .resources
            .iter()
            .map(|resource| resource.resource_id.clone())
            .collect::<BTreeSet<_>>();
        let mut command_ids = self
            .commands
            .iter()
            .map(|command| command.command_id.clone())
            .collect::<BTreeSet<_>>();
        let mut theme_ids = self
            .themes
            .iter()
            .map(|theme| theme.theme_id.clone())
            .collect::<BTreeSet<_>>();
        let mut prompt_ids = self
            .prompts
            .iter()
            .map(|prompt| prompt.prompt_id.clone())
            .collect::<BTreeSet<_>>();
        let mut skill_ids = self
            .skills
            .iter()
            .map(|skill| skill.skill_id.clone())
            .collect::<BTreeSet<_>>();

        for descriptor in descriptors {
            plugin_ids.insert(descriptor.plugin_id.clone());

            for resource in &descriptor.resources {
                if !resource_ids.insert(resource.resource_id.clone()) {
                    return Err(AstrError::Validation(format!(
                        "resource '{}' 在统一资源目录中重复注册",
                        resource.resource_id
                    )));
                }
                self.resources.push(resource.clone());
            }

            for command in &descriptor.commands {
                if !command_ids.insert(command.command_id.clone()) {
                    return Err(AstrError::Validation(format!(
                        "command '{}' 在统一资源目录中重复注册",
                        command.command_id
                    )));
                }
                self.commands.push(command.clone());
            }

            for theme in &descriptor.themes {
                if !theme_ids.insert(theme.theme_id.clone()) {
                    return Err(AstrError::Validation(format!(
                        "theme '{}' 在统一资源目录中重复注册",
                        theme.theme_id
                    )));
                }
                self.themes.push(theme.clone());
            }

            for prompt in &descriptor.prompts {
                if !prompt_ids.insert(prompt.prompt_id.clone()) {
                    return Err(AstrError::Validation(format!(
                        "prompt '{}' 在统一资源目录中重复注册",
                        prompt.prompt_id
                    )));
                }
                self.prompts.push(prompt.clone());
            }

            for skill in &descriptor.skills {
                if !skill_ids.insert(skill.skill_id.clone()) {
                    return Err(AstrError::Validation(format!(
                        "skill '{}' 在统一资源目录中重复注册",
                        skill.skill_id
                    )));
                }
                self.skills.push(skill.clone());
            }
        }

        self.plugin_ids = plugin_ids.into_iter().collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{SkillSource, SkillSpec};

    use super::{ResourceCatalog, build_skill_catalog_base, resources_discover};
    use crate::descriptor::{
        CommandDescriptor, PluginDescriptor, PromptDescriptor, ResourceDescriptor, SkillDescriptor,
        ThemeDescriptor,
    };

    #[test]
    fn catalog_flattens_resource_like_contributions() {
        let mut descriptor = PluginDescriptor::builtin("core-resources", "Core Resources");
        descriptor.resources.push(ResourceDescriptor {
            resource_id: "resource.docs".to_string(),
            kind: "docs".to_string(),
            locator: ".codex/resources/docs".to_string(),
        });
        descriptor.commands.push(CommandDescriptor {
            command_id: "review".to_string(),
            entry_ref: ".codex/commands/review.md".to_string(),
        });
        descriptor.themes.push(ThemeDescriptor {
            theme_id: "graphite".to_string(),
        });
        descriptor.prompts.push(PromptDescriptor {
            prompt_id: "review".to_string(),
            body: "Review the selected change".to_string(),
        });
        descriptor.skills.push(SkillDescriptor {
            skill_id: "skill.review".to_string(),
            entry_ref: ".codex/skills/review/SKILL.md".to_string(),
        });

        let catalog = ResourceCatalog::from_descriptors(&[descriptor.clone()]);

        assert_eq!(catalog.plugin_ids, vec![descriptor.plugin_id]);
        assert_eq!(catalog.resources.len(), 1);
        assert_eq!(catalog.commands.len(), 1);
        assert_eq!(catalog.themes.len(), 1);
        assert_eq!(catalog.prompts.len(), 1);
        assert_eq!(catalog.skills.len(), 1);
    }

    #[test]
    fn catalog_can_merge_multiple_descriptor_batches() {
        let mut builtin = PluginDescriptor::builtin("core-resources", "Core Resources");
        builtin.commands.push(CommandDescriptor {
            command_id: "review".to_string(),
            entry_ref: ".codex/commands/review.md".to_string(),
        });

        let mut discovered = PluginDescriptor::builtin("project-prompts", "Project Prompts");
        discovered.source_ref = ".codex".to_string();
        discovered.prompts.push(PromptDescriptor {
            prompt_id: "prompt.review".to_string(),
            body: "Review the selected change".to_string(),
        });
        discovered.skills.push(SkillDescriptor {
            skill_id: "skill.review".to_string(),
            entry_ref: ".codex/skills/review/SKILL.md".to_string(),
        });

        let mut catalog = ResourceCatalog::default();
        catalog
            .extend_from_descriptors(&[builtin.clone()])
            .expect("builtin batch should merge");
        catalog
            .extend_from_descriptors(&[discovered.clone()])
            .expect("discovered batch should merge");

        assert_eq!(
            catalog.plugin_ids,
            vec![builtin.plugin_id.clone(), discovered.plugin_id.clone()]
        );
        assert_eq!(catalog.commands.len(), 1);
        assert_eq!(catalog.prompts.len(), 1);
        assert_eq!(catalog.skills.len(), 1);
    }

    #[test]
    fn catalog_rejects_duplicate_resource_like_ids_across_batches() {
        let mut first = PluginDescriptor::builtin("first", "First");
        first.prompts.push(PromptDescriptor {
            prompt_id: "prompt.shared".to_string(),
            body: "a".to_string(),
        });

        let mut second = PluginDescriptor::builtin("second", "Second");
        second.prompts.push(PromptDescriptor {
            prompt_id: "prompt.shared".to_string(),
            body: "b".to_string(),
        });

        let mut catalog = ResourceCatalog::default();
        catalog
            .extend_from_descriptors(&[first])
            .expect("first batch should merge");
        let error = catalog
            .extend_from_descriptors(&[second])
            .expect_err("duplicate prompt ids should fail");
        assert!(error.to_string().contains("prompt 'prompt.shared'"));
    }

    #[test]
    fn resources_discover_reports_single_catalog_owner() {
        let mut descriptor = PluginDescriptor::builtin("project-resources", "Project Resources");
        descriptor.commands.push(CommandDescriptor {
            command_id: "apply-change".to_string(),
            entry_ref: ".codex/commands/apply-change.md".to_string(),
        });
        descriptor.prompts.push(PromptDescriptor {
            prompt_id: "prompt.apply-change".to_string(),
            body: "Apply the selected OpenSpec change".to_string(),
        });
        descriptor.skills.push(SkillDescriptor {
            skill_id: "openspec-apply-change".to_string(),
            entry_ref: ".codex/skills/openspec-apply-change/SKILL.md".to_string(),
        });

        let report = resources_discover(&[descriptor]).expect("resource discovery should succeed");

        assert_eq!(report.descriptor_count, 1);
        assert_eq!(report.plugin_ids, vec!["project-resources".to_string()]);
        assert_eq!(report.catalog.commands.len(), 1);
        assert_eq!(report.catalog.prompts.len(), 1);
        assert_eq!(report.catalog.skills.len(), 1);
    }

    #[test]
    fn resources_discover_rejects_duplicate_prompt_ids_before_reload_commit() {
        let mut first = PluginDescriptor::builtin("first", "First");
        first.prompts.push(PromptDescriptor {
            prompt_id: "prompt.shared".to_string(),
            body: "first".to_string(),
        });
        let mut second = PluginDescriptor::builtin("second", "Second");
        second.prompts.push(PromptDescriptor {
            prompt_id: "prompt.shared".to_string(),
            body: "second".to_string(),
        });

        let error =
            resources_discover(&[first, second]).expect_err("duplicate prompt ids should fail");

        assert!(error.to_string().contains("prompt 'prompt.shared'"));
    }

    #[test]
    fn skill_catalog_base_build_keeps_descriptor_skill_ids_under_resource_owner() {
        let builtin = SkillSpec {
            id: "builtin-skill".to_string(),
            name: "builtin-skill".to_string(),
            description: "builtin".to_string(),
            guide: "guide".to_string(),
            skill_root: None,
            asset_files: Vec::new(),
            allowed_tools: Vec::new(),
            source: SkillSource::Builtin,
        };
        let plugin = SkillSpec {
            id: "plugin-skill".to_string(),
            name: "plugin-skill".to_string(),
            description: "plugin".to_string(),
            guide: "guide".to_string(),
            skill_root: None,
            asset_files: Vec::new(),
            allowed_tools: Vec::new(),
            source: SkillSource::Plugin,
        };
        let mut descriptor = PluginDescriptor::builtin("plugin-resources", "Plugin Resources");
        descriptor.skills.push(SkillDescriptor {
            skill_id: "descriptor-skill".to_string(),
            entry_ref: ".codex/skills/descriptor/SKILL.md".to_string(),
        });
        let catalog = ResourceCatalog::from_descriptors(&[descriptor]);

        let build = build_skill_catalog_base(vec![builtin], vec![plugin], &catalog);

        assert_eq!(build.base_skills.len(), 2);
        assert_eq!(build.plugin_skill_count, 1);
        assert_eq!(
            build.descriptor_skill_ids,
            vec!["descriptor-skill".to_string()]
        );
    }
}
