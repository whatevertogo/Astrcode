//! # Skill 工具 (Skill Tool)
//!
//! 实现内置 `Skill` 工具，允许 LLM 按需加载 Skill 的完整指令和资源路径。
//!
//! ## 两阶段模型
//!
//! System Prompt 只暴露 Skill 索引（`name` + `description`），
//! 真正的正文通过 `Skill` tool 按需加载。这样避免将所有 Skill 的
//! 完整内容都注入到每次 LLM 调用中，节省 Token 预算。
//!
//! ## Skill 名称匹配
//!
//! Skill 名称使用 kebab-case，匹配时进行归一化处理（忽略大小写、连字符）。

use std::sync::Arc;

use astrcode_core::{
    Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult,
    ToolPromptMetadata,
};
use astrcode_protocol::capability::SideEffectLevel;
use astrcode_runtime_skill_loader::{
    SKILL_TOOL_NAME, SkillCatalog, SkillSpec, normalize_skill_name,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

/// Skill 工具的输入参数。
///
/// LLM 调用 Skill 工具时会提供 skill 名称（kebab-case）和可选的参数。
#[derive(Debug, Deserialize)]
struct SkillToolInput {
    /// 要加载的 Skill 名称
    skill: String,
    /// 可选的自由格式参数，供 Skill 指令参考
    #[serde(default)]
    args: Option<String>,
}

/// 内置 Skill 工具实现。
///
/// 允许 LLM 按需加载 Skill 的完整指令和资源路径，
/// 避免将所有 Skill 内容都注入到每次 LLM 调用中。
pub(crate) struct SkillTool {
    /// 统一 skill 目录。
    ///
    /// Skill tool 不自己缓存解析结果，而是每次基于当前 working dir 查询 catalog，
    /// 这样 runtime surface 替换后不会残留旧 skill。
    skill_catalog: Arc<SkillCatalog>,
}

impl SkillTool {
    /// 从给定的 Skill 列表创建 Skill 工具实例。
    pub(crate) fn new(skill_catalog: Arc<SkillCatalog>) -> Self {
        Self { skill_catalog }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: SKILL_TOOL_NAME.to_string(),
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
                "Use `Skill` when the system skill index says a task matches a named skill. Call \
                 it before continuing with the task.",
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
                return Ok(skill_error(
                    tool_call_id,
                    format!("invalid Skill input: {error}"),
                ));
            },
        };

        let working_dir = ctx.working_dir().to_string_lossy().into_owned();
        let resolved_skills = self.skill_catalog.resolve_for_working_dir(&working_dir);
        let Some(skill) = resolved_skills
            .iter()
            .find(|skill| skill.matches_requested_name(&parsed_input.skill))
        else {
            let available = resolved_skills
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Ok(skill_error(
                tool_call_id,
                format!(
                    "unknown skill '{}'. Available skills: {}",
                    normalize_skill_name(&parsed_input.skill),
                    available
                ),
            ));
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: SKILL_TOOL_NAME.to_string(),
            ok: true,
            output: render_skill_content(skill, parsed_input.args.as_deref(), ctx.session_id()),
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        })
    }
}

fn skill_error(tool_call_id: String, error: String) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_call_id,
        tool_name: SKILL_TOOL_NAME.to_string(),
        ok: false,
        output: String::new(),
        error: Some(error),
        metadata: None,
        duration_ms: 0,
        truncated: false,
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

/// 规范化 skill 资源路径中的分隔符。
///
/// 将 Windows 风格的反斜杠转换为正斜杠，确保路径在不同操作系统下的一致性。
/// 这对于 skill 目录的跨平台共享和缓存键计算至关重要。
fn normalize_skill_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CancelToken, ToolContext, test_support::TestEnvGuard};
    use astrcode_runtime_skill_loader::SkillSource;
    use serde_json::json;

    use super::*;

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
        let _guard = TestEnvGuard::new();
        let tool = SkillTool::new(Arc::new(SkillCatalog::new(vec![sample_skill()])));
        let result = tool
            .execute(
                "call-1".to_string(),
                json!({ "skill": "git-commit" }),
                &tool_context(),
            )
            .await
            .expect("skill tool should execute");

        assert!(result.ok);
        assert!(
            result
                .output
                .contains("Base directory for this skill: C:/skills/git-commit")
        );
        assert!(result.output.contains("session-1"));
        assert!(result.output.contains("scripts/run.sh"));
    }

    #[tokio::test]
    async fn rejects_unknown_skills() {
        let _guard = TestEnvGuard::new();
        let tool = SkillTool::new(Arc::new(SkillCatalog::new(vec![sample_skill()])));
        let result = tool
            .execute(
                "call-1".to_string(),
                json!({ "skill": "missing" }),
                &tool_context(),
            )
            .await
            .expect("skill tool should execute");

        assert!(!result.ok);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|message| message.contains("unknown skill"))
        );
    }

    #[tokio::test]
    async fn reads_latest_skill_catalog_without_stale_cache() {
        let _guard = TestEnvGuard::new();
        let catalog = Arc::new(SkillCatalog::new(vec![sample_skill()]));
        let tool = SkillTool::new(Arc::clone(&catalog));
        catalog.replace_base_skills(vec![SkillSpec {
            id: "repo-search".to_string(),
            name: "repo-search".to_string(),
            description: "Search the repo.".to_string(),
            guide: "Use ripgrep.".to_string(),
            skill_root: None,
            asset_files: Vec::new(),
            allowed_tools: Vec::new(),
            source: SkillSource::Plugin,
        }]);

        let result = tool
            .execute(
                "call-2".to_string(),
                json!({ "skill": "repo-search" }),
                &tool_context(),
            )
            .await
            .expect("skill tool should execute");

        assert!(result.ok);
        assert!(result.output.contains("Loaded skill: repo-search"));
    }
}
