use async_trait::async_trait;

use crate::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

pub struct SkillGuideContributor;

#[async_trait]
impl PromptContributor for SkillGuideContributor {
    fn contributor_id(&self) -> &'static str {
        "skill-guide"
    }

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        let mut matching_skills = ctx
            .skills
            .iter()
            .filter(|skill| skill.matches(&ctx.tool_names, ctx.latest_user_message()))
            .cloned()
            .collect::<Vec<_>>();
        matching_skills.sort_by(|left, right| left.id.cmp(&right.id));

        PromptContribution {
            blocks: matching_skills
                .iter()
                .map(|skill| {
                    let mut content =
                        format!("{}\n\n{}", skill.description.trim(), skill.guide.trim());
                    if !skill.required_tools.is_empty() {
                        content.push_str(&format!(
                            "\n\nRequired tools: {}",
                            skill.required_tools.join(", ")
                        ));
                    }
                    if !skill.triggers.is_empty() {
                        content
                            .push_str(&format!("\nTrigger hints: {}", skill.triggers.join(", ")));
                    }

                    BlockSpec::system_text(
                        format!("skill-guide-{}", skill.id),
                        BlockKind::SkillGuide,
                        format!("Skill Guide: {}", skill.name),
                        content,
                    )
                    .with_category("skills")
                    .with_tag(skill.source.as_tag())
                    .with_tag(format!("skill:{}", skill.id))
                })
                .collect(),
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{PromptContext, SkillSource, SkillSpec};

    use super::*;

    #[tokio::test]
    async fn only_includes_skills_when_required_tools_and_triggers_match() {
        let contribution = SkillGuideContributor
            .contribute(&PromptContext {
                working_dir: "/workspace/demo".to_string(),
                tool_names: vec!["shell".to_string(), "grep".to_string()],
                capability_descriptors: Vec::new(),
                prompt_declarations: Vec::new(),
                skills: vec![
                    SkillSpec {
                        id: "matching".to_string(),
                        name: "Matching".to_string(),
                        description: "matching".to_string(),
                        guide: "Use shell after grep.".to_string(),
                        required_tools: vec!["shell".to_string(), "grep".to_string()],
                        triggers: vec!["search".to_string()],
                        source: SkillSource::Builtin,
                        expand_tool_guides: true,
                    },
                    SkillSpec {
                        id: "missing".to_string(),
                        name: "Missing".to_string(),
                        description: "missing".to_string(),
                        guide: "missing".to_string(),
                        required_tools: vec!["edit_file".to_string()],
                        triggers: vec![],
                        source: SkillSource::Builtin,
                        expand_tool_guides: false,
                    },
                ],
                step_index: 0,
                turn_index: 0,
                vars: std::collections::HashMap::from([(
                    "turn.user_message".to_string(),
                    "search the repo".to_string(),
                )]),
            })
            .await;

        assert_eq!(contribution.blocks.len(), 1);
        assert_eq!(contribution.blocks[0].id, "skill-guide-matching");
    }
}
