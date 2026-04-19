//! session 级计划工件。
//!
//! 这里维护 session 下唯一 canonical plan 的路径规则、状态模型、审批归档和 prompt 注入，
//! 保持 plan mode 的流程真相收敛在 application，而不是散落在 handler / tool / UI。

use std::{
    fs,
    path::{Path, PathBuf},
};

use astrcode_core::{
    ModeId, PromptDeclaration, SessionPlanState, SessionPlanStatus, project::project_dir,
    session_plan_content_digest,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ApplicationError, mode::builtin_prompts};

const PLAN_DIR_NAME: &str = "plan";
const PLAN_ARCHIVE_DIR_NAME: &str = "plan-archives";
const PLAN_STATE_FILE_NAME: &str = "state.json";
const PLAN_ARCHIVE_FILE_NAME: &str = "plan.md";
const PLAN_ARCHIVE_METADATA_FILE_NAME: &str = "metadata.json";
const PLAN_PATH_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlanSummary {
    pub slug: String,
    pub path: String,
    pub status: String,
    pub title: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionPlanControlSummary {
    pub active_plan: Option<SessionPlanSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanPromptContext {
    pub target_plan_path: String,
    pub target_plan_exists: bool,
    pub target_plan_slug: String,
    pub active_plan: Option<SessionPlanSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanApprovalParseResult {
    pub approved: bool,
    pub matched_phrase: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPlanArchiveMetadata {
    pub archive_id: String,
    pub title: String,
    pub source_session_id: String,
    pub source_plan_slug: String,
    pub source_plan_path: String,
    pub approved_at: DateTime<Utc>,
    pub archived_at: DateTime<Utc>,
    pub status: String,
    pub content_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPlanArchiveSummary {
    pub metadata: ProjectPlanArchiveMetadata,
    pub archive_dir: String,
    pub plan_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPlanArchiveDetail {
    pub summary: ProjectPlanArchiveSummary,
    pub content: String,
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

fn project_plan_archive_dir(working_dir: &Path) -> Result<PathBuf, ApplicationError> {
    Ok(project_dir(working_dir)
        .map_err(|error| {
            ApplicationError::Internal(format!(
                "failed to resolve project directory for '{}': {error}",
                working_dir.display()
            ))
        })?
        .join(PLAN_ARCHIVE_DIR_NAME))
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

fn archive_paths(
    working_dir: &Path,
    archive_id: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), ApplicationError> {
    validate_archive_id(archive_id)?;
    let archive_dir = project_plan_archive_dir(working_dir)?.join(archive_id);
    Ok((
        archive_dir.clone(),
        archive_dir.join(PLAN_ARCHIVE_FILE_NAME),
        archive_dir.join(PLAN_ARCHIVE_METADATA_FILE_NAME),
    ))
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
    serde_json::from_str::<SessionPlanState>(&content)
        .map(Some)
        .map_err(|error| {
            ApplicationError::Internal(format!(
                "failed to parse session plan state '{}': {error}",
                path.display()
            ))
        })
}

pub(crate) fn session_plan_control_summary(
    session_id: &str,
    working_dir: &Path,
) -> Result<SessionPlanControlSummary, ApplicationError> {
    Ok(SessionPlanControlSummary {
        active_plan: active_plan_summary(session_id, working_dir)?,
    })
}

pub(crate) fn active_plan_summary(
    session_id: &str,
    working_dir: &Path,
) -> Result<Option<SessionPlanSummary>, ApplicationError> {
    let Some(state) = load_session_plan_state(session_id, working_dir)? else {
        return Ok(None);
    };
    Ok(Some(plan_summary(session_id, working_dir, &state)?))
}

pub(crate) fn build_plan_prompt_context(
    session_id: &str,
    working_dir: &Path,
    user_text: &str,
) -> Result<PlanPromptContext, ApplicationError> {
    if let Some(active_plan) = active_plan_summary(session_id, working_dir)? {
        return Ok(PlanPromptContext {
            target_plan_path: active_plan.path.clone(),
            target_plan_exists: Path::new(&active_plan.path).exists(),
            target_plan_slug: active_plan.slug.clone(),
            active_plan: Some(active_plan),
        });
    }

    let suggested_slug = slugify_plan_topic(user_text)
        .unwrap_or_else(|| format!("plan-{}", Utc::now().format(PLAN_PATH_TIMESTAMP_FORMAT)));
    let path = session_plan_markdown_path(session_id, working_dir, &suggested_slug)?;
    Ok(PlanPromptContext {
        target_plan_path: path.display().to_string(),
        target_plan_exists: false,
        target_plan_slug: suggested_slug,
        active_plan: None,
    })
}

pub(crate) fn build_plan_prompt_declarations(
    session_id: &str,
    context: &PlanPromptContext,
) -> Vec<PromptDeclaration> {
    let active_plan_line = context
        .active_plan
        .as_ref()
        .map(|plan| {
            format!(
                "- activePlan: slug={}, title={}, status={}, path={}",
                plan.slug, plan.title, plan.status, plan.path
            )
        })
        .unwrap_or_else(|| "- activePlan: (none)".to_string());
    let mut declarations = vec![PromptDeclaration {
        block_id: format!("session.plan.facts.{session_id}"),
        title: "Session Plan Artifact".to_string(),
        content: format!(
            "Session plan facts:\n- targetPlanPath: {}\n- targetPlanExists: {}\n- targetPlanSlug: \
             {}\n{}\n\nUse `upsertSessionPlan` as the only canonical write path for session \
             plans. This session has exactly one canonical plan artifact. When continuing the \
             same task, revise the current plan. When the user clearly changes tasks, overwrite \
             the current plan instead of creating another canonical plan. Only call \
             `exitPlanMode` after the current plan is executable and ready for user review.",
            context.target_plan_path,
            context.target_plan_exists,
            context.target_plan_slug,
            active_plan_line,
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

    if context.active_plan.is_some() {
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
    summary: &SessionPlanSummary,
) -> PromptDeclaration {
    PromptDeclaration {
        block_id: format!("session.plan.exit.{session_id}"),
        title: "Plan Mode Exit".to_string(),
        content: format!(
            "{}\n\nApproved plan artifact:\n- path: {}\n- slug: {}\n- title: {}\n- status: {}",
            builtin_prompts::plan_mode_exit_prompt(),
            summary.path,
            summary.slug,
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

pub(crate) fn active_plan_requires_approval(state: Option<&SessionPlanState>) -> bool {
    state.is_some_and(|state| state.status == SessionPlanStatus::AwaitingApproval)
}

pub(crate) fn mark_active_session_plan_approved(
    session_id: &str,
    working_dir: &Path,
) -> Result<Option<SessionPlanSummary>, ApplicationError> {
    let Some(mut state) = load_session_plan_state(session_id, working_dir)? else {
        return Ok(None);
    };
    if state.status != SessionPlanStatus::AwaitingApproval {
        return Ok(None);
    }

    let plan_path = session_plan_markdown_path(session_id, working_dir, &state.active_plan_slug)?;
    let plan_content =
        fs::read_to_string(&plan_path).map_err(|error| io_error("reading", &plan_path, error))?;
    let plan_content = plan_content.trim().to_string();
    let content_digest = session_plan_content_digest(&plan_content);
    let now = Utc::now();

    state.status = SessionPlanStatus::Approved;
    state.updated_at = now;
    state.approved_at = Some(now);
    if state.archived_plan_digest.as_deref() != Some(content_digest.as_str()) {
        let archive_summary = write_plan_archive_snapshot(
            session_id,
            working_dir,
            &state,
            &plan_path,
            &plan_content,
            &content_digest,
            now,
        )?;
        state.archived_plan_digest = Some(content_digest);
        state.archived_at = Some(archive_summary.metadata.archived_at);
    }
    persist_plan_state(&session_plan_state_path(session_id, working_dir)?, &state)?;
    Ok(Some(plan_summary(session_id, working_dir, &state)?))
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

pub(crate) fn list_project_plan_archives(
    working_dir: &Path,
) -> Result<Vec<ProjectPlanArchiveSummary>, ApplicationError> {
    let archive_root = project_plan_archive_dir(working_dir)?;
    if !archive_root.exists() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for entry in fs::read_dir(&archive_root)
        .map_err(|error| io_error("reading directory", &archive_root, error))?
    {
        let entry =
            entry.map_err(|error| io_error("reading directory entry", &archive_root, error))?;
        let archive_dir = entry.path();
        if !entry
            .file_type()
            .map_err(|error| io_error("reading file type", &archive_dir, error))?
            .is_dir()
        {
            continue;
        }
        let metadata_path = archive_dir.join(PLAN_ARCHIVE_METADATA_FILE_NAME);
        let plan_path = archive_dir.join(PLAN_ARCHIVE_FILE_NAME);
        if !metadata_path.exists() || !plan_path.exists() {
            continue;
        }
        let metadata = fs::read_to_string(&metadata_path)
            .map_err(|error| io_error("reading", &metadata_path, error))
            .and_then(|content| {
                serde_json::from_str::<ProjectPlanArchiveMetadata>(&content).map_err(|error| {
                    ApplicationError::Internal(format!(
                        "failed to parse plan archive metadata '{}': {error}",
                        metadata_path.display()
                    ))
                })
            })?;
        items.push(ProjectPlanArchiveSummary {
            archive_dir: archive_dir.display().to_string(),
            plan_path: plan_path.display().to_string(),
            metadata,
        });
    }
    items.sort_by(|left, right| {
        right
            .metadata
            .archived_at
            .cmp(&left.metadata.archived_at)
            .then_with(|| left.metadata.archive_id.cmp(&right.metadata.archive_id))
    });
    Ok(items)
}

pub(crate) fn read_project_plan_archive(
    working_dir: &Path,
    archive_id: &str,
) -> Result<Option<ProjectPlanArchiveDetail>, ApplicationError> {
    let (archive_dir, plan_path, metadata_path) = archive_paths(working_dir, archive_id)?;
    if !plan_path.exists() || !metadata_path.exists() {
        return Ok(None);
    }
    let metadata = fs::read_to_string(&metadata_path)
        .map_err(|error| io_error("reading", &metadata_path, error))
        .and_then(|content| {
            serde_json::from_str::<ProjectPlanArchiveMetadata>(&content).map_err(|error| {
                ApplicationError::Internal(format!(
                    "failed to parse plan archive metadata '{}': {error}",
                    metadata_path.display()
                ))
            })
        })?;
    let content =
        fs::read_to_string(&plan_path).map_err(|error| io_error("reading", &plan_path, error))?;
    Ok(Some(ProjectPlanArchiveDetail {
        summary: ProjectPlanArchiveSummary {
            metadata,
            archive_dir: archive_dir.display().to_string(),
            plan_path: plan_path.display().to_string(),
        },
        content,
    }))
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

fn plan_summary(
    session_id: &str,
    working_dir: &Path,
    state: &SessionPlanState,
) -> Result<SessionPlanSummary, ApplicationError> {
    Ok(SessionPlanSummary {
        slug: state.active_plan_slug.clone(),
        path: session_plan_markdown_path(session_id, working_dir, &state.active_plan_slug)?
            .display()
            .to_string(),
        status: state.status.to_string(),
        title: state.title.clone(),
        updated_at: state.updated_at,
    })
}

fn write_plan_archive_snapshot(
    session_id: &str,
    working_dir: &Path,
    state: &SessionPlanState,
    plan_path: &Path,
    plan_content: &str,
    content_digest: &str,
    approved_at: DateTime<Utc>,
) -> Result<ProjectPlanArchiveSummary, ApplicationError> {
    let archived_at = Utc::now();
    let archive_root = project_plan_archive_dir(working_dir)?;
    fs::create_dir_all(&archive_root)
        .map_err(|error| io_error("creating directory", &archive_root, error))?;
    let archive_id = reserve_archive_id(&archive_root, approved_at, &state.active_plan_slug)?;
    let (archive_dir, archive_plan_path, metadata_path) = archive_paths(working_dir, &archive_id)?;
    fs::create_dir_all(&archive_dir)
        .map_err(|error| io_error("creating directory", &archive_dir, error))?;
    fs::write(&archive_plan_path, format!("{plan_content}\n"))
        .map_err(|error| io_error("writing", &archive_plan_path, error))?;
    let metadata = ProjectPlanArchiveMetadata {
        archive_id: archive_id.clone(),
        title: state.title.clone(),
        source_session_id: session_id.to_string(),
        source_plan_slug: state.active_plan_slug.clone(),
        source_plan_path: plan_path.display().to_string(),
        approved_at,
        archived_at,
        status: SessionPlanStatus::Approved.to_string(),
        content_digest: content_digest.to_string(),
    };
    let metadata_content = serde_json::to_string_pretty(&metadata).map_err(|error| {
        ApplicationError::Internal(format!(
            "failed to serialize plan archive metadata '{}': {error}",
            metadata_path.display()
        ))
    })?;
    fs::write(&metadata_path, metadata_content)
        .map_err(|error| io_error("writing", &metadata_path, error))?;
    Ok(ProjectPlanArchiveSummary {
        metadata,
        archive_dir: archive_dir.display().to_string(),
        plan_path: archive_plan_path.display().to_string(),
    })
}

fn reserve_archive_id(
    archive_root: &Path,
    approved_at: DateTime<Utc>,
    slug: &str,
) -> Result<String, ApplicationError> {
    let base = format!(
        "{}-{}",
        approved_at.format(PLAN_PATH_TIMESTAMP_FORMAT),
        slug
    );
    for attempt in 0..=99 {
        let candidate = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        };
        if !archive_root.join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(ApplicationError::Internal(format!(
        "failed to reserve a unique plan archive id for slug '{}'",
        slug
    )))
}

fn validate_archive_id(archive_id: &str) -> Result<(), ApplicationError> {
    let archive_id = archive_id.trim();
    if archive_id.is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "archiveId must not be empty".to_string(),
        ));
    }
    if archive_id.contains("..")
        || archive_id.contains('/')
        || archive_id.contains('\\')
        || Path::new(archive_id).is_absolute()
    {
        return Err(ApplicationError::InvalidArgument(format!(
            "archiveId '{}' is invalid",
            archive_id
        )));
    }
    Ok(())
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
            reviewed_plan_digest: Some("abc".to_string()),
            approved_at: None,
            archived_plan_digest: Some("def".to_string()),
            archived_at: None,
        };

        let encoded = serde_json::to_string(&state).expect("state should serialize");
        let decoded =
            serde_json::from_str::<SessionPlanState>(&encoded).expect("state should deserialize");
        assert_eq!(decoded.active_plan_slug, "cleanup-crates");
        assert_eq!(decoded.archived_plan_digest.as_deref(), Some("def"));
    }

    #[test]
    fn build_plan_prompt_declarations_include_single_plan_facts() {
        let declarations = build_plan_prompt_declarations(
            "session-a",
            &PlanPromptContext {
                target_plan_path: "/tmp/cleanup-crates.md".to_string(),
                target_plan_exists: false,
                target_plan_slug: "cleanup-crates".to_string(),
                active_plan: None,
            },
        );

        assert_eq!(declarations.len(), 2);
        assert!(
            declarations[0]
                .content
                .contains("targetPlanPath: /tmp/cleanup-crates.md")
        );
        assert!(
            declarations[0]
                .content
                .contains("overwrite the current plan instead of creating another canonical plan")
        );
        assert!(declarations[1].content.contains("## Implementation Steps"));
    }

    #[test]
    fn reserve_archive_id_adds_suffix_on_collision() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let root = temp.path();
        fs::create_dir_all(root.join("20260419T000000Z-cleanup-crates"))
            .expect("seed dir should exist");
        let candidate = reserve_archive_id(
            root,
            DateTime::parse_from_rfc3339("2026-04-19T00:00:00Z")
                .expect("datetime should parse")
                .with_timezone(&Utc),
            "cleanup-crates",
        )
        .expect("candidate should be reserved");
        assert_eq!(candidate, "20260419T000000Z-cleanup-crates-1");
    }

    #[test]
    fn read_project_plan_archive_returns_saved_content() {
        let _guard = astrcode_core::test_support::TestEnvGuard::new();
        let working_dir = _guard.home_dir().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should exist");
        let archive_root =
            project_plan_archive_dir(&working_dir).expect("archive root should resolve");
        fs::create_dir_all(archive_root.join("archive-a")).expect("archive dir should exist");
        fs::write(
            archive_root.join("archive-a").join(PLAN_ARCHIVE_FILE_NAME),
            "# Plan\n",
        )
        .expect("plan should be written");
        fs::write(
            archive_root
                .join("archive-a")
                .join(PLAN_ARCHIVE_METADATA_FILE_NAME),
            serde_json::to_string_pretty(&ProjectPlanArchiveMetadata {
                archive_id: "archive-a".to_string(),
                title: "Cleanup crates".to_string(),
                source_session_id: "session-a".to_string(),
                source_plan_slug: "cleanup-crates".to_string(),
                source_plan_path: "/tmp/cleanup-crates.md".to_string(),
                approved_at: Utc::now(),
                archived_at: Utc::now(),
                status: "approved".to_string(),
                content_digest: "abc".to_string(),
            })
            .expect("metadata should serialize"),
        )
        .expect("metadata should be written");

        let archive = read_project_plan_archive(&working_dir, "archive-a")
            .expect("archive should load")
            .expect("archive should exist");
        assert_eq!(archive.summary.metadata.archive_id, "archive-a");
        assert_eq!(archive.content, "# Plan\n");
    }

    #[test]
    fn read_project_plan_archive_rejects_path_traversal_archive_id() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let working_dir = temp.path().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should exist");

        let error = read_project_plan_archive(&working_dir, "../secrets")
            .expect_err("path traversal archive id should be rejected");

        assert!(matches!(error, ApplicationError::InvalidArgument(_)));
        assert!(error.to_string().contains("archiveId"));
    }
}
