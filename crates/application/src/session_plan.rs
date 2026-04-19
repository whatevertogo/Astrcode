//! session 级计划工件。
//!
//! 这里维护 session 下 `plan/` 目录的路径规则、状态模型、审批解析和 prompt 注入，
//! 保持 plan mode 的流程真相收敛在 application，而不是散落在 handler / tool / UI。

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use astrcode_core::{ModeId, PromptDeclaration, project::project_dir};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ApplicationError, mode::builtin_prompts};

const PLAN_DIR_NAME: &str = "plan";
const PLAN_STATE_FILE_NAME: &str = "state.json";
const PLAN_PATH_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPlanStatus {
    Draft,
    AwaitingApproval,
    Approved,
    Superseded,
}

impl SessionPlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::Superseded => "superseded",
        }
    }
}

impl fmt::Display for SessionPlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlanState {
    pub active_plan_slug: String,
    pub title: String,
    pub status: SessionPlanStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePlanSummary {
    pub path: String,
    pub status: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanPromptContext {
    pub plan_path: String,
    pub plan_exists: bool,
    pub plan_status: Option<SessionPlanStatus>,
    pub plan_title: Option<String>,
    pub plan_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanApprovalParseResult {
    pub approved: bool,
    pub matched_phrase: Option<&'static str>,
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ApplicationError {
    ApplicationError::Internal(format!("{action} '{}' failed: {error}", path.display()))
}

pub(crate) fn session_plan_dir(
    session_id: &str,
    working_dir: &Path,
) -> Result<PathBuf, ApplicationError> {
    Ok(project_dir(working_dir)
        .map_err(|error| {
            ApplicationError::Internal(format!(
                "failed to resolve project directory for '{}': {error}",
                working_dir.display()
            ))
        })?
        .join("sessions")
        .join(session_id)
        .join(PLAN_DIR_NAME))
}

fn session_plan_state_path(
    session_id: &str,
    working_dir: &Path,
) -> Result<PathBuf, ApplicationError> {
    Ok(session_plan_dir(session_id, working_dir)?.join(PLAN_STATE_FILE_NAME))
}

fn session_plan_markdown_path(
    session_id: &str,
    working_dir: &Path,
    slug: &str,
) -> Result<PathBuf, ApplicationError> {
    Ok(session_plan_dir(session_id, working_dir)?.join(format!("{slug}.md")))
}

pub(crate) fn load_session_plan_state(
    session_id: &str,
    working_dir: &Path,
) -> Result<Option<SessionPlanState>, ApplicationError> {
    let path = session_plan_state_path(session_id, working_dir)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| io_error("reading", &path, error))?;
    let state = serde_json::from_str::<SessionPlanState>(&content).map_err(|error| {
        ApplicationError::Internal(format!(
            "failed to parse session plan state '{}': {error}",
            path.display()
        ))
    })?;
    Ok(Some(state))
}

pub(crate) fn active_plan_summary(
    session_id: &str,
    working_dir: &Path,
) -> Result<Option<ActivePlanSummary>, ApplicationError> {
    let Some(state) = load_session_plan_state(session_id, working_dir)? else {
        return Ok(None);
    };
    let path = session_plan_markdown_path(session_id, working_dir, &state.active_plan_slug)?;
    Ok(Some(ActivePlanSummary {
        path: path.display().to_string(),
        status: state.status.to_string(),
        title: state.title,
    }))
}

pub(crate) fn build_plan_prompt_context(
    session_id: &str,
    working_dir: &Path,
    user_text: &str,
) -> Result<PlanPromptContext, ApplicationError> {
    if let Some(state) = load_session_plan_state(session_id, working_dir)? {
        let path = session_plan_markdown_path(session_id, working_dir, &state.active_plan_slug)?;
        return Ok(PlanPromptContext {
            plan_path: path.display().to_string(),
            plan_exists: path.exists(),
            plan_status: Some(state.status),
            plan_title: Some(state.title),
            plan_slug: state.active_plan_slug,
        });
    }

    let suggested_slug = slugify_plan_topic(user_text)
        .unwrap_or_else(|| format!("plan-{}", Utc::now().format(PLAN_PATH_TIMESTAMP_FORMAT)));
    let path = session_plan_markdown_path(session_id, working_dir, &suggested_slug)?;
    Ok(PlanPromptContext {
        plan_path: path.display().to_string(),
        plan_exists: false,
        plan_status: None,
        plan_title: None,
        plan_slug: suggested_slug,
    })
}

pub(crate) fn build_plan_prompt_declarations(
    session_id: &str,
    context: &PlanPromptContext,
) -> Vec<PromptDeclaration> {
    let mut declarations = vec![PromptDeclaration {
        block_id: format!("session.plan.facts.{session_id}"),
        title: "Session Plan Artifact".to_string(),
        content: format!(
            "Session plan facts:\n- planPath: {}\n- planExists: {}\n- planSlug: {}\n- planStatus: \
             {}\n- planTitle: {}\n\nUse `upsertSessionPlan` to create or update this \
             session-scoped plan artifact. When the plan does not exist yet, create the first \
             draft at the provided path using the provided slug. Keep revising the same file \
             while the topic stays the same.",
            context.plan_path,
            context.plan_exists,
            context.plan_slug,
            context
                .plan_status
                .as_ref()
                .map(SessionPlanStatus::as_str)
                .unwrap_or("missing"),
            context.plan_title.as_deref().unwrap_or("(none)")
        ),
        render_target: astrcode_core::PromptDeclarationRenderTarget::System,
        layer: astrcode_core::SystemPromptLayer::Dynamic,
        kind: astrcode_core::PromptDeclarationKind::ExtensionInstruction,
        priority_hint: Some(605),
        always_include: true,
        source: astrcode_core::PromptDeclarationSource::Builtin,
        capability_name: None,
        origin: Some("session-plan:facts".to_string()),
    }];

    if context.plan_exists {
        declarations.push(PromptDeclaration {
            block_id: format!("session.plan.reentry.{session_id}"),
            title: "Plan Re-entry".to_string(),
            content: builtin_prompts::plan_mode_reentry_prompt().to_string(),
            render_target: astrcode_core::PromptDeclarationRenderTarget::System,
            layer: astrcode_core::SystemPromptLayer::Dynamic,
            kind: astrcode_core::PromptDeclarationKind::ExtensionInstruction,
            priority_hint: Some(604),
            always_include: true,
            source: astrcode_core::PromptDeclarationSource::Builtin,
            capability_name: None,
            origin: Some("session-plan:reentry".to_string()),
        });
    } else {
        declarations.push(PromptDeclaration {
            block_id: format!("session.plan.template.{session_id}"),
            title: "Plan Template".to_string(),
            content: builtin_prompts::plan_template_prompt().to_string(),
            render_target: astrcode_core::PromptDeclarationRenderTarget::System,
            layer: astrcode_core::SystemPromptLayer::Dynamic,
            kind: astrcode_core::PromptDeclarationKind::ExtensionInstruction,
            priority_hint: Some(604),
            always_include: true,
            source: astrcode_core::PromptDeclarationSource::Builtin,
            capability_name: None,
            origin: Some("session-plan:template".to_string()),
        });
    }

    declarations
}

pub(crate) fn build_plan_exit_declaration(
    session_id: &str,
    summary: &ActivePlanSummary,
) -> PromptDeclaration {
    PromptDeclaration {
        block_id: format!("session.plan.exit.{session_id}"),
        title: "Plan Mode Exit".to_string(),
        content: format!(
            "{}\n\nApproved plan artifact:\n- path: {}\n- title: {}\n- status: {}",
            builtin_prompts::plan_mode_exit_prompt(),
            summary.path,
            summary.title,
            summary.status
        ),
        render_target: astrcode_core::PromptDeclarationRenderTarget::System,
        layer: astrcode_core::SystemPromptLayer::Dynamic,
        kind: astrcode_core::PromptDeclarationKind::ExtensionInstruction,
        priority_hint: Some(605),
        always_include: true,
        source: astrcode_core::PromptDeclarationSource::Builtin,
        capability_name: None,
        origin: Some("session-plan:exit".to_string()),
    }
}

pub(crate) fn parse_plan_approval(text: &str) -> PlanApprovalParseResult {
    let normalized_english = text
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for phrase in ["approved", "go ahead", "implement it"] {
        if normalized_english == phrase
            || (phrase != "implement it" && normalized_english.starts_with(&format!("{phrase} ")))
        {
            return PlanApprovalParseResult {
                approved: true,
                matched_phrase: Some(phrase),
            };
        }
    }

    let normalized_chinese = text
        .chars()
        .filter(|ch| !ch.is_whitespace() && !is_common_punctuation(*ch))
        .collect::<String>();
    for phrase in ["同意", "可以", "按这个做", "开始实现"] {
        let matched = match phrase {
            "同意" | "可以" => normalized_chinese == phrase,
            _ => normalized_chinese == phrase || normalized_chinese.starts_with(phrase),
        };
        if matched {
            return PlanApprovalParseResult {
                approved: true,
                matched_phrase: Some(phrase),
            };
        }
    }

    PlanApprovalParseResult {
        approved: false,
        matched_phrase: None,
    }
}

pub(crate) fn mark_session_plan_approved(
    session_id: &str,
    working_dir: &Path,
) -> Result<Option<SessionPlanState>, ApplicationError> {
    let Some(mut state) = load_session_plan_state(session_id, working_dir)? else {
        return Ok(None);
    };
    let path = session_plan_state_path(session_id, working_dir)?;
    let now = Utc::now();
    state.status = SessionPlanStatus::Approved;
    state.updated_at = now;
    state.approved_at = Some(now);
    persist_plan_state(&path, &state)?;
    Ok(Some(state))
}

pub(crate) fn copy_session_plan_artifacts(
    source_session_id: &str,
    target_session_id: &str,
    working_dir: &Path,
) -> Result<(), ApplicationError> {
    let source_dir = session_plan_dir(source_session_id, working_dir)?;
    if !source_dir.exists() {
        return Ok(());
    }
    let target_dir = session_plan_dir(target_session_id, working_dir)?;
    copy_dir_recursive(&source_dir, &target_dir)
}

pub(crate) fn current_mode_requires_plan_context(mode_id: &ModeId) -> bool {
    mode_id == &ModeId::plan()
}

fn persist_plan_state(path: &Path, state: &SessionPlanState) -> Result<(), ApplicationError> {
    let Some(parent) = path.parent() else {
        return Err(ApplicationError::Internal(format!(
            "session plan state '{}' has no parent directory",
            path.display()
        )));
    };
    fs::create_dir_all(parent).map_err(|error| io_error("creating directory", parent, error))?;
    let content = serde_json::to_string_pretty(state).map_err(|error| {
        ApplicationError::Internal(format!(
            "failed to serialize session plan state '{}': {error}",
            path.display()
        ))
    })?;
    fs::write(path, content).map_err(|error| io_error("writing", path, error))
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), ApplicationError> {
    fs::create_dir_all(target).map_err(|error| io_error("creating directory", target, error))?;
    for entry in
        fs::read_dir(source).map_err(|error| io_error("reading directory", source, error))?
    {
        let entry = entry.map_err(|error| io_error("reading directory entry", source, error))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| io_error("reading file type", &source_path, error))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)
                .map_err(|error| io_error("copying file", &source_path, error))?;
        }
    }
    Ok(())
}

fn is_common_punctuation(ch: char) -> bool {
    matches!(
        ch,
        ',' | '.' | ';' | ':' | '!' | '?' | '，' | '。' | '；' | '：' | '！' | '？' | '、'
    )
}

fn slugify_plan_topic(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in input.chars().map(|ch| ch.to_ascii_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
            continue;
        }
        if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { None } else { Some(slug) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plan_approval_is_conservative() {
        assert!(parse_plan_approval("同意").approved);
        assert!(parse_plan_approval("按这个做，开始吧").approved);
        assert!(parse_plan_approval("approved please continue").approved);
        assert!(!parse_plan_approval("可以再想想").approved);
        assert!(!parse_plan_approval("don't implement it yet").approved);
    }

    #[test]
    fn copy_session_plan_artifacts_ignores_missing_source() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        copy_session_plan_artifacts("session-a", "session-b", temp.path())
            .expect("missing source should be ignored");
    }

    #[test]
    fn session_plan_state_round_trips_through_json_schema() {
        let state = SessionPlanState {
            active_plan_slug: "cleanup-crates".to_string(),
            title: "Cleanup crates".to_string(),
            status: SessionPlanStatus::AwaitingApproval,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            approved_at: None,
        };

        let encoded = serde_json::to_string(&state).expect("state should serialize");
        let decoded =
            serde_json::from_str::<SessionPlanState>(&encoded).expect("state should deserialize");
        assert_eq!(decoded.active_plan_slug, "cleanup-crates");
        assert_eq!(decoded.status, SessionPlanStatus::AwaitingApproval);
    }

    #[test]
    fn build_plan_prompt_declarations_include_facts_and_template() {
        let declarations = build_plan_prompt_declarations(
            "session-a",
            &PlanPromptContext {
                plan_path: "/tmp/cleanup-crates.md".to_string(),
                plan_exists: false,
                plan_status: None,
                plan_title: None,
                plan_slug: "cleanup-crates".to_string(),
            },
        );

        assert_eq!(declarations.len(), 2);
        assert!(
            declarations[0]
                .content
                .contains("planPath: /tmp/cleanup-crates.md")
        );
        assert!(declarations[1].content.contains("## Implementation Steps"));
    }
}
