use std::{cmp::Reverse, collections::HashSet, path::Path};

use astrcode_session_runtime::{
    ConversationBlockFacts, ConversationChildHandoffBlockFacts, ConversationErrorBlockFacts,
    ConversationSnapshotFacts, ConversationSystemNoteBlockFacts, SessionControlStateSnapshot,
    ToolCallBlockFacts,
};

use crate::{
    App, ApplicationError, ComposerOptionKind, ComposerOptionsRequest, SessionMeta,
    terminal::{
        ConversationFocus, TerminalChildSummaryFacts, TerminalControlFacts, TerminalFacts,
        TerminalLastCompactMetaFacts, TerminalRehydrateFacts, TerminalRehydrateReason,
        TerminalResumeCandidateFacts, TerminalSlashAction, TerminalSlashCandidateFacts,
        TerminalStreamFacts, TerminalStreamReplayFacts, latest_transcript_cursor,
        truncate_terminal_summary,
    },
};

impl App {
    pub async fn conversation_snapshot_facts(
        &self,
        session_id: &str,
        focus: ConversationFocus,
    ) -> Result<TerminalFacts, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let focus_session_id = self
            .resolve_conversation_focus_session_id(session_id, &focus)
            .await?;
        let transcript = self
            .session_runtime
            .conversation_snapshot(&focus_session_id)
            .await?;
        let session_title = self
            .session_runtime
            .list_session_metas()
            .await?
            .into_iter()
            .find(|meta| meta.session_id == session_id)
            .map(|meta| meta.title)
            .ok_or_else(|| {
                ApplicationError::NotFound(format!("session '{session_id}' not found"))
            })?;
        let control = self.terminal_control_facts(session_id).await?;
        let child_summaries = self
            .conversation_child_summaries(session_id, &focus)
            .await?;
        let slash_candidates = self.terminal_slash_candidates(session_id, None).await?;

        Ok(TerminalFacts {
            active_session_id: session_id.to_string(),
            session_title,
            transcript,
            control,
            child_summaries,
            slash_candidates,
        })
    }

    pub async fn terminal_snapshot_facts(
        &self,
        session_id: &str,
    ) -> Result<TerminalFacts, ApplicationError> {
        self.conversation_snapshot_facts(session_id, ConversationFocus::Root)
            .await
    }

    pub async fn conversation_stream_facts(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
        focus: ConversationFocus,
    ) -> Result<TerminalStreamFacts, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let focus_session_id = self
            .resolve_conversation_focus_session_id(session_id, &focus)
            .await?;

        if let Some(requested_cursor) = last_event_id {
            validate_cursor_format(requested_cursor)?;
            let transcript = self
                .session_runtime
                .conversation_snapshot(&focus_session_id)
                .await?;
            let latest_cursor = latest_transcript_cursor(&transcript);
            if cursor_is_after_head(requested_cursor, latest_cursor.as_deref())? {
                return Ok(TerminalStreamFacts::RehydrateRequired(
                    TerminalRehydrateFacts {
                        session_id: session_id.to_string(),
                        requested_cursor: requested_cursor.to_string(),
                        latest_cursor,
                        reason: TerminalRehydrateReason::CursorExpired,
                    },
                ));
            }
        }

        let replay = self
            .session_runtime
            .conversation_stream_replay(&focus_session_id, last_event_id)
            .await?;
        let control = self.terminal_control_facts(session_id).await?;
        let child_summaries = self
            .conversation_child_summaries(session_id, &focus)
            .await?;
        let slash_candidates = self.terminal_slash_candidates(session_id, None).await?;

        Ok(TerminalStreamFacts::Replay(Box::new(
            TerminalStreamReplayFacts {
                active_session_id: session_id.to_string(),
                replay,
                control,
                child_summaries,
                slash_candidates,
            },
        )))
    }

    pub async fn terminal_stream_facts(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<TerminalStreamFacts, ApplicationError> {
        self.conversation_stream_facts(session_id, last_event_id, ConversationFocus::Root)
            .await
    }

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
                    recent_output: latest_terminal_summary(&child_transcript),
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

    async fn resolve_conversation_focus_session_id(
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
                    text: format!("/skill {}", option.id),
                },
            }),
        );

        if let Some(query) = query.as_deref() {
            candidates.retain(|candidate| slash_candidate_matches(candidate, query));
        }

        // Why: 顶层 palette 只暴露固定命令与可见 skill，不把 capability 噪声直接塞给终端。
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
        Ok(map_control_facts(control))
    }
}

fn map_control_facts(control: SessionControlStateSnapshot) -> TerminalControlFacts {
    TerminalControlFacts {
        phase: control.phase,
        active_turn_id: control.active_turn_id,
        manual_compact_pending: control.manual_compact_pending,
        compacting: control.compacting,
        last_compact_meta: control
            .last_compact_meta
            .map(|meta| TerminalLastCompactMetaFacts {
                trigger: meta.trigger,
                meta: meta.meta,
            }),
    }
}

fn validate_cursor_format(cursor: &str) -> Result<(), ApplicationError> {
    let Some((storage_seq, subindex)) = cursor.split_once('.') else {
        return Err(ApplicationError::InvalidArgument(format!(
            "invalid cursor '{cursor}'"
        )));
    };
    if storage_seq.parse::<u64>().is_err() || subindex.parse::<u32>().is_err() {
        return Err(ApplicationError::InvalidArgument(format!(
            "invalid cursor '{cursor}'"
        )));
    }
    Ok(())
}

fn cursor_is_after_head(
    requested_cursor: &str,
    latest_cursor: Option<&str>,
) -> Result<bool, ApplicationError> {
    let Some(latest_cursor) = latest_cursor else {
        return Ok(false);
    };
    Ok(parse_cursor(requested_cursor)? > parse_cursor(latest_cursor)?)
}

fn parse_cursor(cursor: &str) -> Result<(u64, u32), ApplicationError> {
    let (storage_seq, subindex) = cursor
        .split_once('.')
        .ok_or_else(|| ApplicationError::InvalidArgument(format!("invalid cursor '{cursor}'")))?;
    let storage_seq = storage_seq
        .parse::<u64>()
        .map_err(|_| ApplicationError::InvalidArgument(format!("invalid cursor '{cursor}'")))?;
    let subindex = subindex
        .parse::<u32>()
        .map_err(|_| ApplicationError::InvalidArgument(format!("invalid cursor '{cursor}'")))?;
    Ok((storage_seq, subindex))
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
        TerminalSlashCandidateFacts {
            kind: ComposerOptionKind::Command,
            id: "skill".to_string(),
            title: "插入技能".to_string(),
            description: "打开 skill 候选面板".to_string(),
            keywords: vec!["skill".to_string(), "prompt".to_string()],
            badges: vec!["built-in".to_string()],
            action: TerminalSlashAction::OpenSkillPalette,
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

fn latest_terminal_summary(snapshot: &ConversationSnapshotFacts) -> Option<String> {
    snapshot
        .blocks
        .iter()
        .rev()
        .find_map(summary_from_block)
        .or_else(|| latest_transcript_cursor(snapshot).map(|cursor| format!("cursor:{cursor}")))
}

fn summary_from_block(block: &ConversationBlockFacts) -> Option<String> {
    match block {
        ConversationBlockFacts::Assistant(block) => summary_from_markdown(&block.markdown),
        ConversationBlockFacts::ToolCall(block) => summary_from_tool_call(block),
        ConversationBlockFacts::ChildHandoff(block) => summary_from_child_handoff(block),
        ConversationBlockFacts::Error(block) => summary_from_error_block(block),
        ConversationBlockFacts::SystemNote(block) => summary_from_system_note(block),
        ConversationBlockFacts::User(_) | ConversationBlockFacts::Thinking(_) => None,
    }
}

fn summary_from_markdown(markdown: &str) -> Option<String> {
    (!markdown.trim().is_empty()).then(|| truncate_terminal_summary(markdown))
}

fn summary_from_tool_call(block: &ToolCallBlockFacts) -> Option<String> {
    block
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .map(truncate_terminal_summary)
        .or_else(|| {
            block
                .error
                .as_deref()
                .filter(|error| !error.trim().is_empty())
                .map(truncate_terminal_summary)
        })
        .or_else(|| summary_from_markdown(&block.streams.stderr))
        .or_else(|| summary_from_markdown(&block.streams.stdout))
}

fn summary_from_child_handoff(block: &ConversationChildHandoffBlockFacts) -> Option<String> {
    block
        .message
        .as_deref()
        .filter(|message| !message.trim().is_empty())
        .map(truncate_terminal_summary)
}

fn summary_from_error_block(block: &ConversationErrorBlockFacts) -> Option<String> {
    summary_from_markdown(&block.message)
}

fn summary_from_system_note(block: &ConversationSystemNoteBlockFacts) -> Option<String> {
    summary_from_markdown(&block.markdown)
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc, time::Duration};

    use astrcode_core::AgentEvent;
    use astrcode_session_runtime::SessionRuntime;
    use async_trait::async_trait;
    use tokio::time::timeout;

    use super::*;
    use crate::{
        AppKernelPort, AppSessionPort, ComposerSkillPort, ConfigService, McpConfigScope, McpPort,
        McpServerStatusView, McpService, ProfileResolutionService,
        agent::{
            AgentOrchestrationService,
            test_support::{TestLlmBehavior, build_agent_test_harness},
        },
        composer::ComposerSkillSummary,
        mcp::RegisterMcpServerInput,
    };

    struct StaticComposerSkillPort {
        summaries: Vec<ComposerSkillSummary>,
    }

    impl ComposerSkillPort for StaticComposerSkillPort {
        fn list_skill_summaries(&self, _working_dir: &Path) -> Vec<ComposerSkillSummary> {
            self.summaries.clone()
        }
    }

    struct NoopMcpPort;

    #[async_trait]
    impl McpPort for NoopMcpPort {
        async fn list_server_status(&self) -> Vec<McpServerStatusView> {
            Vec::new()
        }

        async fn approve_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn reject_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn reconnect_server(&self, _name: &str) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn reset_project_choices(&self) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn upsert_server(
            &self,
            _input: &RegisterMcpServerInput,
        ) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn remove_server(
            &self,
            _scope: McpConfigScope,
            _name: &str,
        ) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn set_server_enabled(
            &self,
            _scope: McpConfigScope,
            _name: &str,
            _enabled: bool,
        ) -> Result<(), ApplicationError> {
            Ok(())
        }
    }

    struct TerminalAppHarness {
        app: App,
        session_runtime: Arc<SessionRuntime>,
    }

    fn build_terminal_app_harness(skill_ids: &[&str]) -> TerminalAppHarness {
        build_terminal_app_harness_with_behavior(
            skill_ids,
            TestLlmBehavior::Succeed {
                content: "子代理已完成。".to_string(),
            },
        )
    }

    fn build_terminal_app_harness_with_behavior(
        skill_ids: &[&str],
        llm_behavior: TestLlmBehavior,
    ) -> TerminalAppHarness {
        let harness =
            build_agent_test_harness(llm_behavior).expect("agent test harness should build");
        let kernel: Arc<dyn AppKernelPort> = harness.kernel.clone();
        let session_runtime = harness.session_runtime.clone();
        let session_port: Arc<dyn AppSessionPort> = session_runtime.clone();
        let config: Arc<ConfigService> = harness.config_service.clone();
        let profiles: Arc<ProfileResolutionService> = harness.profiles.clone();
        let composer_skills: Arc<dyn ComposerSkillPort> = Arc::new(StaticComposerSkillPort {
            summaries: skill_ids
                .iter()
                .map(|id| ComposerSkillSummary::new(*id, format!("{id} description")))
                .collect(),
        });
        let mcp_service = Arc::new(McpService::new(Arc::new(NoopMcpPort)));
        let agent_service: Arc<AgentOrchestrationService> = Arc::new(harness.service.clone());
        let app = App::new(
            kernel,
            session_port,
            profiles,
            config,
            composer_skills,
            mcp_service,
            agent_service,
        );
        TerminalAppHarness {
            app,
            session_runtime,
        }
    }

    #[tokio::test]
    async fn terminal_stream_facts_expose_live_llm_deltas_before_durable_completion() {
        let harness = build_terminal_app_harness_with_behavior(
            &[],
            TestLlmBehavior::Stream {
                reasoning_chunks: vec!["先".to_string(), "整理".to_string()],
                text_chunks: vec!["流".to_string(), "式".to_string()],
                final_content: "流式完成".to_string(),
                final_reasoning: Some("先整理".to_string()),
            },
        );
        let project = tempfile::tempdir().expect("tempdir should be created");
        let session = harness
            .app
            .create_session(project.path().display().to_string())
            .await
            .expect("session should be created");

        let TerminalStreamFacts::Replay(replay) = harness
            .app
            .terminal_stream_facts(&session.session_id, None)
            .await
            .expect("stream facts should build")
        else {
            panic!("fresh stream should start from replay facts");
        };
        let mut live_receiver = replay.replay.replay.live_receiver;

        let accepted = harness
            .app
            .submit_prompt(&session.session_id, "请流式回答".to_string())
            .await
            .expect("prompt should submit");

        let mut live_events = Vec::new();
        for _ in 0..4 {
            live_events.push(
                timeout(Duration::from_secs(1), live_receiver.recv())
                    .await
                    .expect("live delta should arrive in time")
                    .expect("live receiver should stay open"),
            );
        }

        assert!(matches!(
            &live_events[0],
            AgentEvent::ThinkingDelta { delta, .. } if delta == "先"
        ));
        assert!(matches!(
            &live_events[1],
            AgentEvent::ThinkingDelta { delta, .. } if delta == "整理"
        ));
        assert!(matches!(
            &live_events[2],
            AgentEvent::ModelDelta { delta, .. } if delta == "流"
        ));
        assert!(matches!(
            &live_events[3],
            AgentEvent::ModelDelta { delta, .. } if delta == "式"
        ));

        harness
            .session_runtime
            .wait_for_turn_terminal_snapshot(&session.session_id, accepted.turn_id.as_str())
            .await
            .expect("turn should settle");

        let snapshot = harness
            .app
            .terminal_snapshot_facts(&session.session_id)
            .await
            .expect("terminal snapshot should build");
        assert!(snapshot.transcript.blocks.iter().any(|block| matches!(
            block,
            ConversationBlockFacts::Assistant(block) if block.markdown == "流式完成"
        )));
        assert!(snapshot.transcript.blocks.iter().any(|block| matches!(
            block,
            ConversationBlockFacts::Thinking(block) if block.markdown == "先整理"
        )));
    }

    #[tokio::test]
    async fn terminal_snapshot_facts_hydrate_history_control_and_slash_candidates() {
        let harness = build_terminal_app_harness(&["openspec-apply-change"]);
        let project = tempfile::tempdir().expect("tempdir should be created");
        let session = harness
            .app
            .create_session(project.path().display().to_string())
            .await
            .expect("session should be created");
        harness
            .app
            .submit_prompt(&session.session_id, "请总结当前仓库".to_string())
            .await
            .expect("prompt should submit");

        let facts = harness
            .app
            .terminal_snapshot_facts(&session.session_id)
            .await
            .expect("terminal snapshot should build");

        assert_eq!(facts.active_session_id, session.session_id);
        assert!(!facts.transcript.blocks.is_empty());
        assert!(facts.transcript.cursor.is_some());
        assert!(
            facts
                .slash_candidates
                .iter()
                .any(|candidate| candidate.id == "new")
        );
        assert!(
            facts
                .slash_candidates
                .iter()
                .any(|candidate| candidate.id == "resume")
        );
        assert!(
            facts
                .slash_candidates
                .iter()
                .any(|candidate| candidate.id == "compact")
        );
        assert!(
            facts
                .slash_candidates
                .iter()
                .any(|candidate| candidate.id == "openspec-apply-change")
        );
    }

    #[tokio::test]
    async fn terminal_stream_facts_returns_replay_for_valid_cursor() {
        let harness = build_terminal_app_harness(&[]);
        let project = tempfile::tempdir().expect("tempdir should be created");
        let session = harness
            .app
            .create_session(project.path().display().to_string())
            .await
            .expect("session should be created");
        harness
            .app
            .submit_prompt(&session.session_id, "hello".to_string())
            .await
            .expect("prompt should submit");
        let snapshot = harness
            .app
            .terminal_snapshot_facts(&session.session_id)
            .await
            .expect("snapshot should build");
        let cursor = snapshot.transcript.cursor.clone();

        let facts = harness
            .app
            .terminal_stream_facts(&session.session_id, cursor.as_deref())
            .await
            .expect("stream facts should build");

        match facts {
            TerminalStreamFacts::Replay(replay) => {
                assert_eq!(replay.active_session_id, session.session_id);
                assert!(replay.replay.replay.history.is_empty());
                assert!(replay.replay.replay_frames.is_empty());
                assert_eq!(
                    replay
                        .replay
                        .seed_records
                        .last()
                        .map(|record| record.event_id.as_str()),
                    snapshot.transcript.cursor.as_deref()
                );
            },
            TerminalStreamFacts::RehydrateRequired(_) => {
                panic!("valid cursor should not require rehydrate");
            },
        }
    }

    #[tokio::test]
    async fn terminal_stream_facts_falls_back_to_rehydrate_for_future_cursor() {
        let harness = build_terminal_app_harness(&[]);
        let project = tempfile::tempdir().expect("tempdir should be created");
        let session = harness
            .app
            .create_session(project.path().display().to_string())
            .await
            .expect("session should be created");
        harness
            .app
            .submit_prompt(&session.session_id, "hello".to_string())
            .await
            .expect("prompt should submit");

        let facts = harness
            .app
            .terminal_stream_facts(&session.session_id, Some("999999.9"))
            .await
            .expect("stream facts should build");

        match facts {
            TerminalStreamFacts::Replay(_) => {
                panic!("future cursor should require rehydrate");
            },
            TerminalStreamFacts::RehydrateRequired(rehydrate) => {
                assert_eq!(rehydrate.reason, TerminalRehydrateReason::CursorExpired);
                assert_eq!(rehydrate.requested_cursor, "999999.9");
                assert!(rehydrate.latest_cursor.is_some());
            },
        }
    }

    #[tokio::test]
    async fn terminal_resume_candidates_use_server_fact_and_recent_sorting() {
        let harness = build_terminal_app_harness(&[]);
        let project = tempfile::tempdir().expect("tempdir should be created");
        let older_dir = project.path().join("older");
        let newer_dir = project.path().join("newer");
        std::fs::create_dir_all(&older_dir).expect("older dir should exist");
        std::fs::create_dir_all(&newer_dir).expect("newer dir should exist");
        let older = harness
            .app
            .create_session(older_dir.display().to_string())
            .await
            .expect("older session should be created");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let newer = harness
            .app
            .create_session(newer_dir.display().to_string())
            .await
            .expect("newer session should be created");

        let candidates = harness
            .app
            .terminal_resume_candidates(Some("newer"), 20)
            .await
            .expect("resume candidates should build");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].session_id, newer.session_id);
        let all_candidates = harness
            .app
            .terminal_resume_candidates(None, 20)
            .await
            .expect("resume candidates should build");
        assert_eq!(all_candidates[0].session_id, newer.session_id);
        assert_eq!(all_candidates[1].session_id, older.session_id);
    }

    #[tokio::test]
    async fn terminal_child_summaries_only_return_direct_visible_children() {
        let harness = build_terminal_app_harness(&[]);
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent_dir = project.path().join("parent");
        let child_dir = project.path().join("child");
        let unrelated_dir = project.path().join("unrelated");
        std::fs::create_dir_all(&parent_dir).expect("parent dir should exist");
        std::fs::create_dir_all(&child_dir).expect("child dir should exist");
        std::fs::create_dir_all(&unrelated_dir).expect("unrelated dir should exist");
        let parent = harness
            .session_runtime
            .create_session(parent_dir.display().to_string())
            .await
            .expect("parent session should be created");
        let child = harness
            .session_runtime
            .create_session(child_dir.display().to_string())
            .await
            .expect("child session should be created");
        let unrelated = harness
            .session_runtime
            .create_session(unrelated_dir.display().to_string())
            .await
            .expect("unrelated session should be created");

        let root = harness
            .app
            .ensure_session_root_agent_context(&parent.session_id)
            .await
            .expect("root context should exist");

        harness
            .session_runtime
            .append_child_session_notification(
                &parent.session_id,
                "turn-parent",
                root.clone(),
                astrcode_core::ChildSessionNotification {
                    notification_id: "child-1".to_string().into(),
                    child_ref: astrcode_core::ChildAgentRef {
                        identity: astrcode_core::ChildExecutionIdentity {
                            agent_id: "agent-child".to_string().into(),
                            session_id: parent.session_id.clone().into(),
                            sub_run_id: "subrun-child".to_string().into(),
                        },
                        parent: astrcode_core::ParentExecutionRef {
                            parent_agent_id: root.agent_id.clone(),
                            parent_sub_run_id: None,
                        },
                        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
                        status: astrcode_core::AgentLifecycleStatus::Running,
                        open_session_id: child.session_id.clone().into(),
                    },
                    kind: astrcode_core::ChildSessionNotificationKind::Started,
                    source_tool_call_id: Some("tool-call-1".to_string().into()),
                    delivery: Some(astrcode_core::ParentDelivery {
                        idempotency_key: "child-1".to_string(),
                        origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                        terminal_semantics:
                            astrcode_core::ParentDeliveryTerminalSemantics::NonTerminal,
                        source_turn_id: Some("turn-child".to_string()),
                        payload: astrcode_core::ParentDeliveryPayload::Progress(
                            astrcode_core::ProgressParentDeliveryPayload {
                                message: "child progress".to_string(),
                            },
                        ),
                    }),
                },
            )
            .await
            .expect("child notification should append");

        let accepted = harness
            .app
            .submit_prompt(&child.session_id, "child output".to_string())
            .await
            .expect("child prompt should submit");
        harness
            .session_runtime
            .wait_for_turn_terminal_snapshot(&child.session_id, accepted.turn_id.as_str())
            .await
            .expect("child turn should settle");
        harness
            .app
            .submit_prompt(&unrelated.session_id, "ignore me".to_string())
            .await
            .expect("unrelated prompt should submit");

        let children = harness
            .app
            .terminal_child_summaries(&parent.session_id)
            .await
            .expect("child summaries should build");

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].node.child_session_id, child.session_id.into());
        assert!(
            children[0]
                .recent_output
                .as_deref()
                .is_some_and(|summary| summary.contains("子代理已完成"))
        );
    }

    #[tokio::test]
    async fn conversation_focus_snapshot_reads_child_session_transcript() {
        let harness = build_terminal_app_harness(&[]);
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent_dir = project.path().join("parent");
        let child_dir = project.path().join("child");
        std::fs::create_dir_all(&parent_dir).expect("parent dir should exist");
        std::fs::create_dir_all(&child_dir).expect("child dir should exist");
        let parent = harness
            .session_runtime
            .create_session(parent_dir.display().to_string())
            .await
            .expect("parent session should be created");
        let child = harness
            .session_runtime
            .create_session(child_dir.display().to_string())
            .await
            .expect("child session should be created");
        let root = harness
            .app
            .ensure_session_root_agent_context(&parent.session_id)
            .await
            .expect("root context should exist");

        harness
            .session_runtime
            .append_child_session_notification(
                &parent.session_id,
                "turn-parent",
                root.clone(),
                astrcode_core::ChildSessionNotification {
                    notification_id: "child-1".to_string().into(),
                    child_ref: astrcode_core::ChildAgentRef {
                        identity: astrcode_core::ChildExecutionIdentity {
                            agent_id: "agent-child".to_string().into(),
                            session_id: parent.session_id.clone().into(),
                            sub_run_id: "subrun-child".to_string().into(),
                        },
                        parent: astrcode_core::ParentExecutionRef {
                            parent_agent_id: root.agent_id.clone(),
                            parent_sub_run_id: None,
                        },
                        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
                        status: astrcode_core::AgentLifecycleStatus::Running,
                        open_session_id: child.session_id.clone().into(),
                    },
                    kind: astrcode_core::ChildSessionNotificationKind::Started,
                    source_tool_call_id: Some("tool-call-1".to_string().into()),
                    delivery: Some(astrcode_core::ParentDelivery {
                        idempotency_key: "child-1".to_string(),
                        origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                        terminal_semantics:
                            astrcode_core::ParentDeliveryTerminalSemantics::NonTerminal,
                        source_turn_id: Some("turn-child".to_string()),
                        payload: astrcode_core::ParentDeliveryPayload::Progress(
                            astrcode_core::ProgressParentDeliveryPayload {
                                message: "child progress".to_string(),
                            },
                        ),
                    }),
                },
            )
            .await
            .expect("child notification should append");

        harness
            .app
            .submit_prompt(&parent.session_id, "parent prompt".to_string())
            .await
            .expect("parent prompt should submit");
        harness
            .app
            .submit_prompt(&child.session_id, "child prompt".to_string())
            .await
            .expect("child prompt should submit");

        let facts = harness
            .app
            .conversation_snapshot_facts(
                &parent.session_id,
                ConversationFocus::SubRun {
                    sub_run_id: "subrun-child".to_string(),
                },
            )
            .await
            .expect("conversation focus snapshot should build");

        assert_eq!(facts.active_session_id, parent.session_id);
        assert!(facts.transcript.blocks.iter().any(|block| matches!(
            block,
            ConversationBlockFacts::User(block) if block.markdown == "child prompt"
        )));
        assert!(facts.transcript.blocks.iter().all(|block| !matches!(
            block,
            ConversationBlockFacts::User(block) if block.markdown == "parent prompt"
        )));
        assert!(facts.child_summaries.is_empty());
    }

    #[test]
    fn cursor_is_after_head_treats_equal_cursor_as_caught_up() {
        assert!(!cursor_is_after_head("12.3", Some("12.3")).expect("equal cursor should parse"));
        assert!(cursor_is_after_head("12.4", Some("12.3")).expect("newer cursor should parse"));
        assert!(!cursor_is_after_head("12.2", Some("12.3")).expect("older cursor should parse"));
    }
}
