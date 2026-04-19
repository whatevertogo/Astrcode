//! 会话恢复候选列表查询。
//!
//! 根据搜索关键词和限制数量，从 session 列表中筛选出可恢复的会话候选项，
//! 按更新时间倒序排列。支持按标题、工作目录、会话 ID 模糊匹配。

use std::{cmp::Reverse, collections::HashSet, path::Path};

use crate::{
    App, ApplicationError, ComposerOptionKind, ComposerOptionsRequest, SessionMeta,
    session_plan::active_plan_summary,
    terminal::{
        ConversationAuthoritativeSummary, ConversationFocus, TerminalChildSummaryFacts,
        TerminalControlFacts, TerminalResumeCandidateFacts, TerminalSlashAction,
        TerminalSlashCandidateFacts, summarize_conversation_authoritative,
    },
};

impl App {
    pub async fn terminal_resume_candidates(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TerminalResumeCandidateFacts>, ApplicationError> {
        let metas = self.session_runtime.list_session_metas().await?;
        let query = normalize_query(query);
        let limit = normalize_limit(limit);
        let mut items = metas
            .into_iter()
            .filter(|meta| resume_candidate_matches(meta, query.as_deref()))
            .map(|meta| TerminalResumeCandidateFacts {
                session_id: meta.session_id,
                title: meta.title,
                display_name: meta.display_name,
                working_dir: meta.working_dir,
                updated_at: meta.updated_at,
                created_at: meta.created_at,
                phase: meta.phase,
                parent_session_id: meta.parent_session_id,
            })
            .collect::<Vec<_>>();

        items.sort_by_key(|item| Reverse(item.updated_at));
        items.truncate(limit);
        Ok(items)
    }

    pub async fn terminal_child_summaries(
        &self,
        session_id: &str,
    ) -> Result<Vec<TerminalChildSummaryFacts>, ApplicationError> {
        self.conversation_child_summaries(session_id, &ConversationFocus::Root)
            .await
    }

    pub async fn conversation_child_summaries(
        &self,
        session_id: &str,
        focus: &ConversationFocus,
    ) -> Result<Vec<TerminalChildSummaryFacts>, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let focus_session_id = self
            .resolve_conversation_focus_session_id(session_id, focus)
            .await?;
        let children = self
            .session_runtime
            .session_child_nodes(&focus_session_id)
            .await?;
        let session_metas = self.session_runtime.list_session_metas().await?;

        let summaries = children
            .into_iter()
            .filter(|node| node.parent_sub_run_id().is_none())
            .map(|node| async {
                self.require_permission(
                    node.parent_session_id.as_str() == focus_session_id,
                    format!(
                        "child '{}' is not visible from session '{}'",
                        node.sub_run_id(),
                        focus_session_id
                    ),
                )?;
                let child_meta = session_metas
                    .iter()
                    .find(|meta| meta.session_id == node.child_session_id.as_str());
                let child_transcript = self
                    .session_runtime
                    .conversation_snapshot(node.child_session_id.as_str())
                    .await?;
                Ok::<_, ApplicationError>(TerminalChildSummaryFacts {
                    node,
                    phase: child_transcript.phase,
                    title: child_meta.map(|meta| meta.title.clone()),
                    display_name: child_meta.map(|meta| meta.display_name.clone()),
                    recent_output: super::summary::latest_terminal_summary(&child_transcript),
                })
            })
            .collect::<Vec<_>>();

        let mut resolved = Vec::with_capacity(summaries.len());
        for summary in summaries {
            resolved.push(summary.await?);
        }
        resolved.sort_by(|left, right| left.node.sub_run_id().cmp(right.node.sub_run_id()));
        Ok(resolved)
    }

    pub async fn terminal_slash_candidates(
        &self,
        session_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<TerminalSlashCandidateFacts>, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await?;
        let query = normalize_query(query);
        let control = self.terminal_control_facts(session_id).await?;
        let mut candidates = terminal_builtin_candidates(&control);
        candidates.extend(
            self.list_composer_options(
                session_id,
                ComposerOptionsRequest {
                    query: query.clone(),
                    kinds: vec![ComposerOptionKind::Skill],
                    limit: 50,
                },
            )
            .await?
            .into_iter()
            .map(|option| TerminalSlashCandidateFacts {
                kind: option.kind,
                id: option.id.clone(),
                title: option.title,
                description: option.description,
                keywords: option.keywords,
                badges: option.badges,
                action: TerminalSlashAction::InsertText {
                    text: format!("/{}", option.id),
                },
            }),
        );

        if let Some(query) = query.as_deref() {
            candidates.retain(|candidate| slash_candidate_matches(candidate, query));
        }

        let _ = Path::new(&working_dir);
        Ok(candidates)
    }

    pub async fn terminal_control_facts(
        &self,
        session_id: &str,
    ) -> Result<TerminalControlFacts, ApplicationError> {
        let control = self
            .session_runtime
            .session_control_state(session_id)
            .await?;
        let mut facts = super::map_control_facts(control);
        let working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await?;
        facts.active_plan = active_plan_summary(session_id, Path::new(&working_dir))?.map(|plan| {
            crate::terminal::ActivePlanFacts {
                path: plan.path,
                status: plan.status,
                title: plan.title,
            }
        });
        Ok(facts)
    }

    pub async fn conversation_authoritative_summary(
        &self,
        session_id: &str,
        focus: &ConversationFocus,
    ) -> Result<ConversationAuthoritativeSummary, ApplicationError> {
        Ok(summarize_conversation_authoritative(
            &self.terminal_control_facts(session_id).await?,
            &self.conversation_child_summaries(session_id, focus).await?,
            &self.terminal_slash_candidates(session_id, None).await?,
        ))
    }

    pub(super) async fn resolve_conversation_focus_session_id(
        &self,
        root_session_id: &str,
        focus: &ConversationFocus,
    ) -> Result<String, ApplicationError> {
        match focus {
            ConversationFocus::Root => Ok(root_session_id.to_string()),
            ConversationFocus::SubRun { sub_run_id } => {
                let mut pending = vec![root_session_id.to_string()];
                let mut visited = HashSet::new();

                while let Some(session_id) = pending.pop() {
                    if !visited.insert(session_id.clone()) {
                        continue;
                    }
                    for node in self
                        .session_runtime
                        .session_child_nodes(&session_id)
                        .await?
                    {
                        if node.sub_run_id().as_str() == *sub_run_id {
                            return Ok(node.child_session_id.to_string());
                        }
                        pending.push(node.child_session_id.to_string());
                    }
                }

                Err(ApplicationError::NotFound(format!(
                    "sub-run '{}' not found under session '{}'",
                    sub_run_id, root_session_id
                )))
            },
        }
    }
}

fn normalize_query(query: Option<&str>) -> Option<String> {
    query
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(|query| query.to_lowercase())
}

fn normalize_limit(limit: usize) -> usize {
    if limit == 0 { 20 } else { limit }
}

fn resume_candidate_matches(meta: &SessionMeta, query: Option<&str>) -> bool {
    let Some(query) = query else {
        return true;
    };
    [
        meta.session_id.as_str(),
        meta.title.as_str(),
        meta.display_name.as_str(),
        meta.working_dir.as_str(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(query))
}

fn terminal_builtin_candidates(control: &TerminalControlFacts) -> Vec<TerminalSlashCandidateFacts> {
    let mut candidates = vec![
        TerminalSlashCandidateFacts {
            kind: ComposerOptionKind::Command,
            id: "new".to_string(),
            title: "新建会话".to_string(),
            description: "创建新 session 并切换焦点".to_string(),
            keywords: vec!["new".to_string(), "session".to_string()],
            badges: vec!["built-in".to_string()],
            action: TerminalSlashAction::CreateSession,
        },
        TerminalSlashCandidateFacts {
            kind: ComposerOptionKind::Command,
            id: "resume".to_string(),
            title: "恢复会话".to_string(),
            description: "搜索并切换到已有 session".to_string(),
            keywords: vec!["resume".to_string(), "switch".to_string()],
            badges: vec!["built-in".to_string()],
            action: TerminalSlashAction::OpenResume,
        },
    ];

    if !control.manual_compact_pending && !control.compacting {
        candidates.push(TerminalSlashCandidateFacts {
            kind: ComposerOptionKind::Command,
            id: "compact".to_string(),
            title: "压缩上下文".to_string(),
            description: "向服务端提交显式 compact 控制请求".to_string(),
            keywords: vec!["compact".to_string(), "compress".to_string()],
            badges: vec!["built-in".to_string()],
            action: TerminalSlashAction::RequestCompact,
        });
    }
    candidates
}

fn slash_candidate_matches(candidate: &TerminalSlashCandidateFacts, query: &str) -> bool {
    candidate.id.to_lowercase().contains(query)
        || candidate.title.to_lowercase().contains(query)
        || candidate.description.to_lowercase().contains(query)
        || candidate
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(query))
}
