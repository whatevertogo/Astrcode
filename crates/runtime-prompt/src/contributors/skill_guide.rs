use async_trait::async_trait;

use crate::{
    resolve_prompt_skills, skill_roots_cache_marker, BlockKind, BlockSpec, PromptContext,
    PromptContribution, PromptContributor,
};

pub struct SkillGuideContributor;

#[async_trait]
impl PromptContributor for SkillGuideContributor {
    fn contributor_id(&self) -> &'static str {
        "skill-guide"
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
        let resolved_skills = resolve_prompt_skills(&ctx.skills, &ctx.working_dir);
        let mut matching_skills = resolved_skills
            .iter()
            .filter(|skill| skill.matches(&ctx.tool_names, ctx.latest_user_message()))
            .cloned()
            .collect::<Vec<_>>();
        matching_skills.sort_by(|left, right| left.id.cmp(&right.id));

        PromptContribution {
            blocks: matching_skills
                .iter()
                .map(|skill| {
                    let mut content_sections = Vec::new();
                    if !skill.description.trim().is_empty() {
                        content_sections.push(skill.description.trim().to_string());
                    }
                    content_sections.push(skill.guide.trim().to_string());
                    let mut content = content_sections.join("\n\n");
                    if !skill.allowed_tools.is_empty() {
                        content.push_str(&format!(
                            "\n\nAllowed tools: {}",
                            skill.allowed_tools.join(", ")
                        ));
                    }
                    if let Some(skill_root) = &skill.skill_root {
                        content.push_str(&format!(
                            "\n\nBase directory for this skill: {}",
                            skill_root
                        ));
                    }
                    if !skill.reference_files.is_empty() {
                        content.push_str(&format!(
                            "\nReference files:\n{}",
                            skill
                                .reference_files
                                .iter()
                                .map(|path| format!("- {}", path))
                                .collect::<Vec<_>>()
                                .join("\n")
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
    use std::fs;

    use astrcode_core::test_support::TestEnvGuard;

    use crate::{PromptContext, SkillSource, SkillSpec};

    use super::*;

    #[tokio::test]
    async fn only_includes_skills_when_allowed_tools_and_triggers_match() {
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
                        skill_root: Some("/workspace/demo/.astrcode/skills/matching".to_string()),
                        reference_files: vec!["references/do.md".to_string()],
                        allowed_tools: vec!["shell".to_string(), "grep".to_string()],
                        triggers: vec!["search".to_string()],
                        source: SkillSource::Builtin,
                        expand_tool_guides: true,
                    },
                    SkillSpec {
                        id: "missing".to_string(),
                        name: "Missing".to_string(),
                        description: "missing".to_string(),
                        guide: "missing".to_string(),
                        skill_root: None,
                        reference_files: Vec::new(),
                        allowed_tools: vec!["edit_file".to_string()],
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
        let crate::BlockContent::Text(content) = &contribution.blocks[0].content else {
            panic!("skill guide blocks should render as text");
        };
        assert!(content
            .contains("Base directory for this skill: /workspace/demo/.astrcode/skills/matching"));
        assert!(content.contains("references/do.md"));
    }

    #[tokio::test]
    async fn includes_project_skill_blocks_when_project_directory_matches() {
        let _guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let skill_dir = project
            .path()
            .join(".astrcode")
            .join("skills")
            .join("project-search");
        fs::create_dir_all(&skill_dir).expect("skill directory should be created");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Project Search\nwhen_to_use: When the user needs project search help\n---\nUse the project skill.\n",
        )
        .expect("skill file should be written");

        let contribution = SkillGuideContributor
            .contribute(&PromptContext {
                working_dir: project.path().to_string_lossy().into_owned(),
                tool_names: vec!["shell".to_string()],
                capability_descriptors: Vec::new(),
                prompt_declarations: Vec::new(),
                skills: Vec::new(),
                step_index: 0,
                turn_index: 0,
                vars: std::collections::HashMap::from([(
                    "turn.user_message".to_string(),
                    "search the project for this flow".to_string(),
                )]),
            })
            .await;

        assert_eq!(contribution.blocks.len(), 1);
        assert_eq!(contribution.blocks[0].id, "skill-guide-project-search");
    }
}
