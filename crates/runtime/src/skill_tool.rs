use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use astrcode_core::{
    Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};

use crate::prompt::{normalize_skill_name, resolve_prompt_skills, SkillSpec};

#[derive(Debug, Deserialize)]
struct SkillToolInput {
    skill: String,
    #[serde(default)]
    args: Option<String>,
}

pub(crate) struct SkillTool {
    base_skills: Vec<SkillSpec>,
}

impl SkillTool {
    pub(crate) fn new(base_skills: Vec<SkillSpec>) -> Self {
        Self { base_skills }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "Skill".to_string(),
            description: "Execute a skill within the main conversation.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "skill": {
                        "type": "string",
                        "description": "The kebab-case skill name to load, such as `git-commit`."
                    },
                    "args": {
                        "type": "string",
                        "description": "Optional free-form arguments that should be considered while following the skill."
                    }
                },
                "required": ["skill"],
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .side_effect(SideEffectLevel::None)
            .prompt(ToolPromptMetadata::new(
                "Loads a skill's full instructions and resource paths on demand.",
                "Use `Skill` when the system skill index says a task matches a named skill. Call it before continuing with the task.",
            ))
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let parsed_input = match serde_json::from_value::<SkillToolInput>(input) {
            Ok(parsed_input) => parsed_input,
            Err(error) => {
                return Ok(ToolExecutionResult {
                    tool_call_id,
                    tool_name: "Skill".to_string(),
                    ok: false,
                    output: String::new(),
                    error: Some(format!("invalid Skill input: {error}")),
                    metadata: None,
                    duration_ms: 0,
                    truncated: false,
                });
            }
        };

        let working_dir = ctx.working_dir().to_string_lossy().into_owned();
        let resolved_skills = resolve_prompt_skills(&self.base_skills, &working_dir);
        let Some(skill) = resolved_skills
            .iter()
            .find(|skill| skill.matches_requested_name(&parsed_input.skill))
        else {
            let available = resolved_skills
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "Skill".to_string(),
                ok: false,
                output: String::new(),
                error: Some(format!(
                    "unknown skill '{}'. Available skills: {}",
                    normalize_skill_name(&parsed_input.skill),
                    available
                )),
                metadata: None,
                duration_ms: 0,
                truncated: false,
            });
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "Skill".to_string(),
            ok: true,
            output: render_skill_content(skill, parsed_input.args.as_deref(), ctx.session_id()),
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        })
    }
}

fn render_skill_content(skill: &SkillSpec, args: Option<&str>, session_id: &str) -> String {
    let mut sections = Vec::new();
    sections.push(format!("Loaded skill: {}", skill.id));

    if !skill.description.trim().is_empty() {
        sections.push(format!("Description: {}", skill.description.trim()));
    }
    if let Some(args) = args.filter(|value| !value.trim().is_empty()) {
        // Keep arguments explicit in the tool result so the next model step can
        // adapt the skill instructions without inventing an out-of-band state
        // channel between the skill index and the loaded prompt body.
        sections.push(format!("Invocation arguments: {}", args.trim()));
    }
    if let Some(skill_root) = &skill.skill_root {
        sections.push(format!(
            "Base directory for this skill: {}",
            normalize_skill_path(skill_root)
        ));
    }

    let mut guide = skill.guide.clone();
    if let Some(skill_root) = &skill.skill_root {
        let normalized_root = normalize_skill_path(skill_root);
        guide = guide.replace("${CLAUDE_SKILL_DIR}", &normalized_root);
        guide = guide.replace("${ASTRCODE_SKILL_DIR}", &normalized_root);
    }
    guide = guide.replace("${CLAUDE_SESSION_ID}", session_id);
    guide = guide.replace("${ASTRCODE_SESSION_ID}", session_id);
    sections.push(guide.trim().to_string());

    if !skill.allowed_tools.is_empty() {
        sections.push(format!("Allowed tools: {}", skill.allowed_tools.join(", ")));
    }
    if !skill.asset_files.is_empty() {
        sections.push(format!(
            "Available skill files:\n{}",
            skill
                .asset_files
                .iter()
                .map(|path| format!("- {}", path))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    sections.join("\n\n")
}

fn normalize_skill_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use astrcode_core::{CancelToken, ToolContext};

    use super::*;
    use crate::prompt::SkillSource;

    fn tool_context() -> ToolContext {
        ToolContext::new(
            "session-1".to_string(),
            std::env::temp_dir(),
            CancelToken::new(),
        )
    }

    fn sample_skill() -> SkillSpec {
        SkillSpec {
            id: "git-commit".to_string(),
            name: "git-commit".to_string(),
            description: "Use this skill when the user asks for a git commit.".to_string(),
            guide: "Run from ${CLAUDE_SKILL_DIR} in session ${CLAUDE_SESSION_ID}.".to_string(),
            skill_root: Some("C:\\skills\\git-commit".to_string()),
            asset_files: vec!["scripts/run.sh".to_string()],
            allowed_tools: vec!["shell".to_string()],
            source: SkillSource::Builtin,
        }
    }

    #[tokio::test]
    async fn loads_and_expands_skill_content() {
        let tool = SkillTool::new(vec![sample_skill()]);
        let result = tool
            .execute(
                "call-1".to_string(),
                json!({ "skill": "git-commit" }),
                &tool_context(),
            )
            .await
            .expect("skill tool should execute");

        assert!(result.ok);
        assert!(result
            .output
            .contains("Base directory for this skill: C:/skills/git-commit"));
        assert!(result.output.contains("session-1"));
        assert!(result.output.contains("scripts/run.sh"));
    }

    #[tokio::test]
    async fn rejects_unknown_skills() {
        let tool = SkillTool::new(vec![sample_skill()]);
        let result = tool
            .execute(
                "call-1".to_string(),
                json!({ "skill": "missing" }),
                &tool_context(),
            )
            .await
            .expect("skill tool should execute");

        assert!(!result.ok);
        assert!(result
            .error
            .as_deref()
            .is_some_and(|message| message.contains("unknown skill")));
    }
}
