use astrcode_core::{CapabilityDescriptor, ToolPromptMetadata};
use async_trait::async_trait;

use crate::{
    resolve_prompt_skills, skill_roots_cache_marker, BlockKind, BlockSpec, PromptContext,
    PromptContribution, PromptContributor, PromptDeclaration, PromptDeclarationKind, SkillSpec,
};

pub struct CapabilityPromptContributor;

const MAX_ALWAYS_ON_DETAILED_GUIDES: usize = 4;

#[async_trait]
impl PromptContributor for CapabilityPromptContributor {
    fn contributor_id(&self) -> &'static str {
        "capability-prompt"
    }

    fn cache_version(&self) -> u64 {
        3
    }

    fn cache_fingerprint(&self, ctx: &PromptContext) -> String {
        format!(
            "{}|{}",
            ctx.contributor_cache_fingerprint(),
            skill_roots_cache_marker(&ctx.working_dir)
        )
    }

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        let mut blocks = Vec::new();
        let tool_guides = collect_tool_guides(&ctx.capability_descriptors);
        let resolved_skills = resolve_prompt_skills(&ctx.skills, &ctx.working_dir);
        if !tool_guides.is_empty() {
            blocks.push(build_tool_summary_block(&tool_guides));
        }

        blocks.extend(
            tool_guides
                .iter()
                .filter(|guide| {
                    guide.prompt.always_include
                        || should_expand_tool_guides(ctx, &resolved_skills, tool_guides.len())
                })
                .map(build_detailed_tool_block),
        );

        blocks.extend(
            ctx.prompt_declarations
                .iter()
                .map(build_prompt_declaration_block),
        );

        PromptContribution {
            blocks,
            ..PromptContribution::default()
        }
    }
}

#[derive(Clone)]
struct ToolGuideEntry {
    descriptor: CapabilityDescriptor,
    prompt: ToolPromptMetadata,
}

fn collect_tool_guides(capability_descriptors: &[CapabilityDescriptor]) -> Vec<ToolGuideEntry> {
    let mut guides = capability_descriptors
        .iter()
        .filter(|descriptor| descriptor.kind.is_tool())
        .filter_map(|descriptor| {
            let prompt = descriptor
                .metadata
                .get("prompt")
                .cloned()
                .and_then(
                    |value| match serde_json::from_value::<ToolPromptMetadata>(value) {
                        Ok(prompt) => Some(prompt),
                        Err(error) => {
                            log::warn!(
                                "ignoring invalid prompt metadata for capability '{}': {}",
                                descriptor.name,
                                error
                            );
                            None
                        }
                    },
                )?;
            Some(ToolGuideEntry {
                descriptor: descriptor.clone(),
                prompt,
            })
        })
        .collect::<Vec<_>>();
    guides.sort_by(|left, right| left.descriptor.name.cmp(&right.descriptor.name));
    guides
}

fn should_expand_tool_guides(
    ctx: &PromptContext,
    resolved_skills: &[SkillSpec],
    tool_guide_count: usize,
) -> bool {
    tool_guide_count <= MAX_ALWAYS_ON_DETAILED_GUIDES
        || matched_skill_wants_tool_guides(ctx, resolved_skills)
}

fn matched_skill_wants_tool_guides(ctx: &PromptContext, resolved_skills: &[SkillSpec]) -> bool {
    resolved_skills
        .iter()
        .filter(|skill| skill.matches(&ctx.tool_names, ctx.latest_user_message()))
        .any(|skill| skill.expand_tool_guides)
}

fn build_tool_summary_block(tool_guides: &[ToolGuideEntry]) -> BlockSpec {
    let mut content = String::from(
        "Use the narrowest tool that can answer the request. Prefer read-only inspection before mutation.\n",
    );
    for guide in tool_guides {
        let caveat = guide
            .prompt
            .caveats
            .first()
            .map(|caveat| format!(" Caveat: {caveat}"))
            .unwrap_or_default();
        content.push_str(&format!(
            "\n- `{}`: {}{}",
            guide.descriptor.name, guide.prompt.summary, caveat
        ));
    }

    BlockSpec::system_text(
        "tool-summary",
        BlockKind::ToolGuide,
        "Tool Summary",
        content,
    )
    .with_tag("source:capability")
    .with_category("capabilities")
}

fn build_detailed_tool_block(guide: &ToolGuideEntry) -> BlockSpec {
    let mut sections = vec![guide.prompt.guide.clone()];
    if !guide.prompt.caveats.is_empty() {
        sections.push(format!(
            "Caveats:\n{}",
            guide
                .prompt
                .caveats
                .iter()
                .map(|caveat| format!("- {caveat}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !guide.prompt.examples.is_empty() {
        sections.push(format!(
            "Examples:\n{}",
            guide
                .prompt
                .examples
                .iter()
                .map(|example| format!("- {example}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let mut block = BlockSpec::system_text(
        format!("tool-guide-{}", guide.descriptor.name),
        BlockKind::ToolGuide,
        format!("Tool Guide: {}", guide.descriptor.name),
        sections.join("\n\n"),
    )
    .with_category("capabilities")
    .with_tag("source:capability")
    .with_tag(format!("capability:{}", guide.descriptor.name));
    if let Some(origin) = guide
        .descriptor
        .metadata
        .get("origin")
        .and_then(serde_json::Value::as_str)
    {
        block = block.with_origin(origin.to_string());
    }
    block
}

fn build_prompt_declaration_block(declaration: &PromptDeclaration) -> BlockSpec {
    let mut block = BlockSpec::message_text(
        declaration.block_id.clone(),
        declaration.kind.as_block_kind(),
        declaration.title.clone(),
        declaration.content.clone(),
        declaration.render_target.as_render_target(),
    )
    .with_category(match declaration.kind {
        PromptDeclarationKind::ToolGuide => "capabilities",
        PromptDeclarationKind::ExtensionInstruction => "extensions",
    })
    .with_tag(declaration.source.as_tag());

    if let Some(priority_hint) = declaration.priority_hint {
        block = block.with_priority(priority_hint);
    }
    if let Some(capability_name) = &declaration.capability_name {
        block = block.with_tag(format!("capability:{capability_name}"));
    }
    if let Some(origin) = &declaration.origin {
        block = block.with_origin(origin.clone());
    }
    block
}

#[cfg(test)]
mod tests {
    use std::fs;

    use astrcode_core::test_support::TestEnvGuard;
    use astrcode_core::{CapabilityDescriptor, CapabilityKind, ToolPromptMetadata};
    use serde_json::json;

    use super::*;
    use crate::{SkillSource, SkillSpec};

    fn tool_descriptor(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor::builder(name, CapabilityKind::tool())
            .description(format!("descriptor for {name}"))
            .schema(json!({"type": "object"}), json!({"type": "string"}))
            .metadata(json!({
                "prompt": ToolPromptMetadata::new(
                    format!("{name} summary"),
                    format!("{name} detailed guide")
                )
                .caveat(format!("{name} caveat"))
                .example(format!("{name} example"))
            }))
            .build()
            .expect("descriptor should build")
    }

    fn context() -> PromptContext {
        PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string(), "grep".to_string()],
            capability_descriptors: vec![tool_descriptor("shell"), tool_descriptor("grep")],
            prompt_declarations: vec![PromptDeclaration {
                block_id: "plugin-guide".to_string(),
                title: "Plugin Guide".to_string(),
                content: "Use the plugin carefully".to_string(),
                render_target: crate::PromptDeclarationRenderTarget::System,
                kind: PromptDeclarationKind::ExtensionInstruction,
                priority_hint: Some(581),
                always_include: false,
                source: crate::PromptDeclarationSource::Plugin,
                capability_name: Some("plugin.search".to_string()),
                origin: Some("example-plugin".to_string()),
            }],
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        }
    }

    #[tokio::test]
    async fn contributes_tool_summary_and_extension_instruction_blocks() {
        let contribution = CapabilityPromptContributor.contribute(&context()).await;

        assert!(contribution
            .blocks
            .iter()
            .any(|block| block.id == "tool-summary" && block.kind == BlockKind::ToolGuide));
        assert!(contribution.blocks.iter().any(|block| {
            block.id == "plugin-guide" && block.kind == BlockKind::ExtensionInstruction
        }));
    }

    #[tokio::test]
    async fn project_skill_override_can_disable_detailed_tool_guides() {
        let _guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let skill_dir = project
            .path()
            .join(".astrcode")
            .join("skills")
            .join("tool-expander");
        fs::create_dir_all(&skill_dir).expect("skill directory should be created");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Tool Expander\nwhen_to_use: When the user asks for the tool expander workflow\n---\nUse the project override.\n",
        )
        .expect("skill file should be written");

        let mut ctx = context();
        ctx.working_dir = project.path().to_string_lossy().into_owned();
        ctx.capability_descriptors = vec![
            tool_descriptor("alpha"),
            tool_descriptor("beta"),
            tool_descriptor("gamma"),
            tool_descriptor("delta"),
            tool_descriptor("epsilon"),
        ];
        ctx.vars.insert(
            "turn.user_message".to_string(),
            "run the tool expander workflow".to_string(),
        );
        ctx.skills = vec![SkillSpec {
            id: "tool-expander".to_string(),
            name: "Tool Expander".to_string(),
            description: "expand".to_string(),
            guide: "expand".to_string(),
            skill_root: None,
            reference_files: Vec::new(),
            allowed_tools: Vec::new(),
            triggers: vec!["tool expander workflow".to_string()],
            source: SkillSource::Builtin,
            expand_tool_guides: true,
        }];

        let contribution = CapabilityPromptContributor.contribute(&ctx).await;

        assert!(!contribution
            .blocks
            .iter()
            .any(|block| block.id == "tool-guide-epsilon"));
    }
}
