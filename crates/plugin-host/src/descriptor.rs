use std::collections::BTreeSet;

use astrcode_core::{AstrError, CapabilitySpec, Result, mode::GovernanceModeSpec};

use crate::hooks::{HookDispatchMode, HookFailurePolicy, HookStage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginSourceKind {
    #[default]
    Builtin,
    Process,
    Command,
    Http,
}

impl PluginSourceKind {
    pub fn to_backend_kind(self) -> crate::backend::PluginBackendKind {
        match self {
            Self::Builtin => crate::backend::PluginBackendKind::InProcess,
            Self::Process => crate::backend::PluginBackendKind::Process,
            Self::Command => crate::backend::PluginBackendKind::Command,
            Self::Http => crate::backend::PluginBackendKind::Http,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDescriptor {
    pub hook_id: String,
    pub event: String,
    pub stage: HookStage,
    pub dispatch_mode: HookDispatchMode,
    pub failure_policy: HookFailurePolicy,
    pub priority: i32,
    pub entry_ref: String,
    pub input_schema: Option<String>,
    pub effect_schema: Option<String>,
}

impl Default for HookDescriptor {
    fn default() -> Self {
        Self {
            hook_id: String::new(),
            event: String::new(),
            stage: HookStage::Runtime,
            dispatch_mode: HookDispatchMode::Sequential,
            failure_policy: HookFailurePolicy::FailOpen,
            priority: 0,
            entry_ref: String::new(),
            input_schema: None,
            effect_schema: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderDescriptor {
    pub provider_id: String,
    pub api_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceDescriptor {
    pub resource_id: String,
    pub kind: String,
    pub locator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandDescriptor {
    pub command_id: String,
    pub entry_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeDescriptor {
    pub theme_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptDescriptor {
    pub prompt_id: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkillDescriptor {
    pub skill_id: String,
    pub entry_ref: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PluginDescriptor {
    pub plugin_id: String,
    pub display_name: String,
    pub version: String,
    pub source_kind: PluginSourceKind,
    pub source_ref: String,
    pub enabled: bool,
    pub priority: i32,
    pub launch_command: Option<String>,
    pub launch_args: Vec<String>,
    pub working_dir: Option<String>,
    pub repository: Option<String>,
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

impl PluginDescriptor {
    pub fn builtin(plugin_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            display_name: display_name.into(),
            version: "0.1.0".to_string(),
            source_kind: PluginSourceKind::Builtin,
            source_ref: "builtin".to_string(),
            enabled: true,
            priority: 0,
            launch_command: None,
            launch_args: Vec::new(),
            working_dir: None,
            repository: None,
            tools: Vec::new(),
            hooks: Vec::new(),
            providers: Vec::new(),
            resources: Vec::new(),
            commands: Vec::new(),
            themes: Vec::new(),
            prompts: Vec::new(),
            skills: Vec::new(),
            modes: Vec::new(),
        }
    }
}

pub fn validate_descriptors(descriptors: &[PluginDescriptor]) -> Result<()> {
    let mut plugin_ids = BTreeSet::new();
    let mut tool_names = BTreeSet::new();
    let mut hook_ids = BTreeSet::new();
    let mut provider_ids = BTreeSet::new();
    let mut resource_ids = BTreeSet::new();
    let mut command_ids = BTreeSet::new();
    let mut theme_ids = BTreeSet::new();
    let mut prompt_ids = BTreeSet::new();
    let mut skill_ids = BTreeSet::new();
    let mut mode_ids = BTreeSet::new();

    for descriptor in descriptors {
        if descriptor.plugin_id.trim().is_empty() {
            return Err(AstrError::Validation("plugin_id 不能为空".to_string()));
        }

        if !plugin_ids.insert(descriptor.plugin_id.clone()) {
            return Err(AstrError::Validation(format!(
                "plugin_id '{}' 重复，无法构建统一 active snapshot",
                descriptor.plugin_id
            )));
        }

        for tool in &descriptor.tools {
            let tool_name = tool.name.to_string();
            if !tool_names.insert(tool_name.clone()) {
                return Err(AstrError::Validation(format!(
                    "tool '{}' 在同一 snapshot 中重复注册",
                    tool_name
                )));
            }
        }

        for hook in &descriptor.hooks {
            if !hook_ids.insert(hook.hook_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "hook '{}' 在同一 snapshot 中重复注册",
                    hook.hook_id
                )));
            }
        }

        for provider in &descriptor.providers {
            if !provider_ids.insert(provider.provider_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "provider '{}' 在同一 snapshot 中重复注册",
                    provider.provider_id
                )));
            }
        }

        for resource in &descriptor.resources {
            if !resource_ids.insert(resource.resource_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "resource '{}' 在同一 snapshot 中重复注册",
                    resource.resource_id
                )));
            }
        }

        for command in &descriptor.commands {
            if !command_ids.insert(command.command_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "command '{}' 在同一 snapshot 中重复注册",
                    command.command_id
                )));
            }
        }

        for theme in &descriptor.themes {
            if !theme_ids.insert(theme.theme_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "theme '{}' 在同一 snapshot 中重复注册",
                    theme.theme_id
                )));
            }
        }

        for prompt in &descriptor.prompts {
            if !prompt_ids.insert(prompt.prompt_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "prompt '{}' 在同一 snapshot 中重复注册",
                    prompt.prompt_id
                )));
            }
        }

        for skill in &descriptor.skills {
            if !skill_ids.insert(skill.skill_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "skill '{}' 在同一 snapshot 中重复注册",
                    skill.skill_id
                )));
            }
        }

        for mode in &descriptor.modes {
            mode.validate()?;
            let mode_id = mode.id.as_str().to_string();
            if !mode_ids.insert(mode_id.clone()) {
                return Err(AstrError::Validation(format!(
                    "mode '{}' 在同一 snapshot 中重复注册",
                    mode_id
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilityKind, CapabilitySpec, InvocationMode, SideEffect, Stability};

    use super::{
        CommandDescriptor, HookDescriptor, PluginDescriptor, PluginSourceKind, PromptDescriptor,
        ProviderDescriptor, ResourceDescriptor, SkillDescriptor, ThemeDescriptor,
        validate_descriptors,
    };

    fn tool(name: &str) -> CapabilitySpec {
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
    fn builtin_descriptor_uses_builtin_defaults() {
        let descriptor = PluginDescriptor::builtin("core-tools", "Core Tools");

        assert_eq!(descriptor.plugin_id, "core-tools");
        assert_eq!(descriptor.display_name, "Core Tools");
        assert_eq!(descriptor.source_kind, PluginSourceKind::Builtin);
        assert!(descriptor.enabled);
        assert!(descriptor.tools.is_empty());
        assert!(descriptor.hooks.is_empty());
        assert!(descriptor.modes.is_empty());
    }

    #[test]
    fn validate_descriptors_rejects_duplicate_plugin_ids() {
        let descriptors = vec![
            PluginDescriptor::builtin("alpha", "Alpha"),
            PluginDescriptor::builtin("alpha", "Alpha Again"),
        ];

        let error =
            validate_descriptors(&descriptors).expect_err("duplicate plugin ids should fail");
        assert!(error.to_string().contains("plugin_id 'alpha' 重复"));
    }

    #[test]
    fn validate_descriptors_rejects_duplicate_tool_names() {
        let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
        alpha.tools.push(tool("tool.shared"));
        let mut beta = PluginDescriptor::builtin("beta", "Beta");
        beta.tools.push(tool("tool.shared"));

        let error =
            validate_descriptors(&[alpha, beta]).expect_err("duplicate tool names should fail");
        assert!(error.to_string().contains("tool 'tool.shared'"));
    }

    #[test]
    fn validate_descriptors_rejects_duplicate_resource_like_ids() {
        let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
        alpha.hooks.push(HookDescriptor {
            hook_id: "hook.shared".to_string(),
            event: "tool_call".to_string(),
            ..Default::default()
        });
        let mut beta = PluginDescriptor::builtin("beta", "Beta");
        beta.hooks.push(HookDescriptor {
            hook_id: "hook.shared".to_string(),
            event: "tool_result".to_string(),
            ..Default::default()
        });

        let error =
            validate_descriptors(&[alpha, beta]).expect_err("duplicate hook ids should fail");
        assert!(error.to_string().contains("hook 'hook.shared'"));
    }

    #[test]
    fn validate_descriptors_rejects_duplicate_resource_command_theme_prompt_skill_ids() {
        let cases = vec![
            {
                let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
                alpha.resources.push(ResourceDescriptor {
                    resource_id: "resource.shared".to_string(),
                    kind: "docs".to_string(),
                    locator: "docs".to_string(),
                });
                let mut beta = PluginDescriptor::builtin("beta", "Beta");
                beta.resources.push(ResourceDescriptor {
                    resource_id: "resource.shared".to_string(),
                    kind: "docs".to_string(),
                    locator: "docs-other".to_string(),
                });
                (vec![alpha, beta], "resource 'resource.shared'")
            },
            {
                let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
                alpha.commands.push(CommandDescriptor {
                    command_id: "command.shared".to_string(),
                    entry_ref: "commands/a.md".to_string(),
                });
                let mut beta = PluginDescriptor::builtin("beta", "Beta");
                beta.commands.push(CommandDescriptor {
                    command_id: "command.shared".to_string(),
                    entry_ref: "commands/b.md".to_string(),
                });
                (vec![alpha, beta], "command 'command.shared'")
            },
            {
                let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
                alpha.themes.push(ThemeDescriptor {
                    theme_id: "theme.shared".to_string(),
                });
                let mut beta = PluginDescriptor::builtin("beta", "Beta");
                beta.themes.push(ThemeDescriptor {
                    theme_id: "theme.shared".to_string(),
                });
                (vec![alpha, beta], "theme 'theme.shared'")
            },
            {
                let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
                alpha.prompts.push(PromptDescriptor {
                    prompt_id: "prompt.shared".to_string(),
                    body: "a".to_string(),
                });
                let mut beta = PluginDescriptor::builtin("beta", "Beta");
                beta.prompts.push(PromptDescriptor {
                    prompt_id: "prompt.shared".to_string(),
                    body: "b".to_string(),
                });
                (vec![alpha, beta], "prompt 'prompt.shared'")
            },
            {
                let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
                alpha.skills.push(SkillDescriptor {
                    skill_id: "skill.shared".to_string(),
                    entry_ref: "skills/a/SKILL.md".to_string(),
                });
                let mut beta = PluginDescriptor::builtin("beta", "Beta");
                beta.skills.push(SkillDescriptor {
                    skill_id: "skill.shared".to_string(),
                    entry_ref: "skills/b/SKILL.md".to_string(),
                });
                (vec![alpha, beta], "skill 'skill.shared'")
            },
            {
                let mut alpha = PluginDescriptor::builtin("alpha", "Alpha");
                alpha.providers.push(ProviderDescriptor {
                    provider_id: "provider.shared".to_string(),
                    api_kind: "openai".to_string(),
                });
                let mut beta = PluginDescriptor::builtin("beta", "Beta");
                beta.providers.push(ProviderDescriptor {
                    provider_id: "provider.shared".to_string(),
                    api_kind: "anthropic".to_string(),
                });
                (vec![alpha, beta], "provider 'provider.shared'")
            },
        ];

        for (descriptors, expected) in cases {
            let error = validate_descriptors(&descriptors)
                .expect_err("duplicate resource-like ids should fail");
            assert!(error.to_string().contains(expected), "{expected}");
        }
    }
}
