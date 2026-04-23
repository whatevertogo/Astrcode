use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, StorageEvent, StorageEventPayload, StoredEvent,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{
    AgentLineageEntry, CollaborationFactSummary, CompactTrace, ErrorTrace, PromptMetricsSnapshot,
    SessionTrace, StorageSeqRange, SubRunTrace, ToolCallDeltaRecord, ToolCallRecord,
    ToolResultReferenceTrace, TurnTimelineEvent, TurnTimelineEventKind, TurnTrace,
};
use crate::{EvalError, EvalResult};

pub struct TraceExtractor;

impl TraceExtractor {
    pub fn extract_file(path: impl AsRef<Path>) -> EvalResult<SessionTrace> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|error| {
            EvalError::io(
                format!("无法打开 session JSONL 文件 {}", path.display()),
                error,
            )
        })?;
        Self::extract_reader(BufReader::new(file))
    }

    pub fn extract_reader<R: BufRead>(reader: R) -> EvalResult<SessionTrace> {
        let mut builder = SessionTraceBuilder::default();

        for (index, line) in reader.lines().enumerate() {
            let line_no = index + 1;
            let line = line.map_err(|error| {
                EvalError::io(format!("读取 session JSONL 第 {line_no} 行失败"), error)
            })?;
            if line.trim().is_empty() {
                continue;
            }

            let stored = serde_json::from_str::<StoredEvent>(&line).ok();
            let event = match serde_json::from_str::<StorageEvent>(&line) {
                Ok(event) => event,
                Err(source) => {
                    return Err(EvalError::JsonLine {
                        line: line_no,
                        source,
                    });
                },
            };

            builder.push(stored.map(|item| item.storage_seq), event);
        }

        Ok(builder.finish())
    }
}

#[derive(Default)]
struct SessionTraceBuilder {
    session: SessionTrace,
    turn_order: Vec<String>,
    turns: HashMap<String, TurnBuilder>,
    session_lineage_seen: HashSet<String>,
    last_turn_id: Option<String>,
}

impl SessionTraceBuilder {
    fn push(&mut self, storage_seq: Option<u64>, event: StorageEvent) {
        match event.payload {
            StorageEventPayload::SessionStart {
                session_id,
                timestamp,
                working_dir,
                parent_session_id,
                parent_storage_seq,
            } => {
                self.session.session_id = Some(session_id);
                self.session.working_dir = Some(working_dir);
                self.session.started_at = Some(timestamp);
                self.session.parent_session_id = parent_session_id;
                self.session.parent_storage_seq = parent_storage_seq;
            },
            payload => {
                let event = EventEnvelope {
                    storage_seq,
                    turn_id: event.turn_id,
                    agent: event.agent,
                    payload,
                };
                self.record_lineage(&event.agent);
                if let Some(turn_id) = self.resolve_turn_id(&event) {
                    self.last_turn_id = Some(turn_id.clone());
                    let turn = self.turns.entry(turn_id.clone()).or_insert_with(|| {
                        self.turn_order.push(turn_id.clone());
                        TurnBuilder::new(turn_id)
                    });
                    turn.push(event);
                }
            },
        }
    }

    fn resolve_turn_id(&self, event: &EventEnvelope) -> Option<String> {
        if let Some(turn_id) = &event.turn_id {
            return Some(turn_id.clone());
        }

        if let Some(turn_id) = &event.agent.parent_turn_id {
            return Some(turn_id.to_string());
        }

        match event.payload {
            StorageEventPayload::PromptMetrics { .. }
            | StorageEventPayload::CompactApplied { .. }
            | StorageEventPayload::SubRunStarted { .. }
            | StorageEventPayload::SubRunFinished { .. }
            | StorageEventPayload::AgentCollaborationFact { .. } => self.last_turn_id.clone(),
            _ => None,
        }
    }

    fn record_lineage(&mut self, agent: &AgentEventContext) {
        if agent.is_empty() {
            return;
        }
        let entry = AgentLineageEntry {
            agent_id: agent.agent_id.as_ref().map(ToString::to_string),
            parent_turn_id: agent.parent_turn_id.as_ref().map(ToString::to_string),
            agent_profile: agent.agent_profile.clone(),
            sub_run_id: agent.sub_run_id.as_ref().map(ToString::to_string),
            parent_sub_run_id: agent.parent_sub_run_id.as_ref().map(ToString::to_string),
            invocation_kind: agent.invocation_kind,
            storage_mode: agent.storage_mode,
            child_session_id: agent.child_session_id.as_ref().map(ToString::to_string),
        };
        let key = lineage_key(&entry);
        if self.session_lineage_seen.insert(key) {
            self.session.agent_lineage.push(entry);
        }
    }

    fn finish(mut self) -> SessionTrace {
        let turn_order = std::mem::take(&mut self.turn_order);
        self.session.turns = turn_order
            .into_iter()
            .filter_map(|turn_id| self.turns.remove(&turn_id))
            .map(TurnBuilder::finish)
            .collect();
        self.session
    }
}

struct EventEnvelope {
    storage_seq: Option<u64>,
    turn_id: Option<String>,
    agent: AgentEventContext,
    payload: StorageEventPayload,
}

struct TurnBuilder {
    trace: TurnTrace,
    tool_order: Vec<String>,
    tools: HashMap<String, ToolCallBuilder>,
    sub_run_order: Vec<String>,
    sub_runs: HashMap<String, SubRunBuilder>,
    lineage_seen: HashSet<String>,
}

impl TurnBuilder {
    fn new(turn_id: String) -> Self {
        Self {
            trace: TurnTrace {
                turn_id,
                user_input: None,
                assistant_output: None,
                assistant_reasoning: None,
                thinking_deltas: Vec::new(),
                tool_calls: Vec::new(),
                prompt_metrics: Vec::new(),
                compactions: Vec::new(),
                sub_runs: Vec::new(),
                collaboration_facts: Vec::new(),
                errors: Vec::new(),
                timeline: Vec::new(),
                agent_lineage: Vec::new(),
                storage_seq_range: None,
                completed_at: None,
                completion_reason: None,
                incomplete: true,
            },
            tool_order: Vec::new(),
            tools: HashMap::new(),
            sub_run_order: Vec::new(),
            sub_runs: HashMap::new(),
            lineage_seen: HashSet::new(),
        }
    }

    fn push(&mut self, event: EventEnvelope) {
        self.record_seq(event.storage_seq);
        self.record_lineage(&event.agent);

        match event.payload {
            StorageEventPayload::UserMessage {
                content, timestamp, ..
            } => {
                self.trace.user_input = Some(content.clone());
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::UserMessage,
                    Some(timestamp),
                    None,
                    Some(content),
                );
            },
            StorageEventPayload::AssistantDelta { token } => {
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::AssistantDelta,
                    None,
                    None,
                    Some(token),
                );
            },
            StorageEventPayload::ThinkingDelta { token } => {
                self.trace.thinking_deltas.push(token.clone());
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::ThinkingDelta,
                    None,
                    None,
                    Some(token),
                );
            },
            StorageEventPayload::AssistantFinal {
                content,
                reasoning_content,
                timestamp,
                ..
            } => {
                self.trace.assistant_output = Some(content.clone());
                self.trace.assistant_reasoning = reasoning_content.clone();
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::AssistantFinal,
                    timestamp,
                    None,
                    Some(content),
                );
            },
            StorageEventPayload::ToolCall {
                tool_call_id,
                tool_name,
                args,
            } => {
                let builder = self.tool_builder(&tool_call_id);
                if builder.tool_name.is_empty() {
                    builder.tool_name = tool_name.clone();
                }
                builder.args = args.clone();
                builder.started_storage_seq = event.storage_seq;
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::ToolCall,
                    None,
                    Some(tool_call_id),
                    Some(tool_name),
                );
            },
            StorageEventPayload::ToolCallDelta {
                tool_call_id,
                tool_name,
                stream,
                delta,
            } => {
                let builder = self.tool_builder(&tool_call_id);
                if builder.tool_name.is_empty() && !tool_name.is_empty() {
                    builder.tool_name = tool_name.clone();
                }
                builder.stream_deltas.push(ToolCallDeltaRecord {
                    storage_seq: event.storage_seq,
                    stream,
                    delta: delta.clone(),
                });
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::ToolCallDelta,
                    None,
                    Some(tool_call_id),
                    Some(delta),
                );
            },
            StorageEventPayload::ToolResult {
                tool_call_id,
                tool_name,
                output,
                success,
                error,
                metadata,
                continuation,
                duration_ms,
            } => {
                let builder = self.tool_builder(&tool_call_id);
                if builder.tool_name.is_empty() && !tool_name.is_empty() {
                    builder.tool_name = tool_name.clone();
                }
                builder.output = Some(output.clone());
                builder.success = Some(success);
                builder.error = error.clone();
                builder.metadata = metadata;
                builder.continuation = continuation;
                builder.duration_ms = Some(duration_ms);
                builder.finished_storage_seq = event.storage_seq;
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::ToolResult,
                    None,
                    Some(tool_call_id),
                    Some(output),
                );
            },
            StorageEventPayload::ToolResultReferenceApplied {
                tool_call_id,
                persisted_output,
                replacement,
                original_bytes,
            } => {
                let builder = self.tool_builder(&tool_call_id);
                builder.persisted_reference = Some(ToolResultReferenceTrace {
                    storage_seq: event.storage_seq,
                    persisted_output,
                    replacement: replacement.clone(),
                    original_bytes,
                });
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::ToolResultReferenceApplied,
                    None,
                    Some(tool_call_id),
                    Some(replacement),
                );
            },
            StorageEventPayload::PromptMetrics { metrics } => {
                self.trace.prompt_metrics.push(PromptMetricsSnapshot {
                    storage_seq: event.storage_seq,
                    agent: event.agent,
                    metrics,
                });
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::PromptMetrics,
                    None,
                    None,
                    None,
                );
            },
            StorageEventPayload::CompactApplied {
                trigger,
                summary,
                meta,
                preserved_recent_turns,
                pre_tokens,
                post_tokens_estimate,
                messages_removed,
                tokens_freed,
                timestamp,
            } => {
                self.trace.compactions.push(CompactTrace {
                    storage_seq: event.storage_seq,
                    agent: event.agent,
                    trigger,
                    summary: summary.clone(),
                    meta,
                    preserved_recent_turns,
                    pre_tokens,
                    post_tokens_estimate,
                    messages_removed,
                    tokens_freed,
                    timestamp,
                });
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::CompactApplied,
                    Some(timestamp),
                    None,
                    Some(summary),
                );
            },
            StorageEventPayload::SubRunStarted {
                tool_call_id,
                resolved_overrides,
                resolved_limits,
                timestamp,
            } => {
                let Some(sub_run_id) = event.agent.sub_run_id.as_ref().map(ToString::to_string)
                else {
                    return;
                };
                let builder = self.sub_run_builder(&sub_run_id);
                builder.tool_call_id = tool_call_id.clone();
                builder.agent_id = event.agent.agent_id.as_ref().map(ToString::to_string);
                builder.agent_profile = event.agent.agent_profile.clone();
                builder.parent_turn_id =
                    event.agent.parent_turn_id.as_ref().map(ToString::to_string);
                builder.parent_sub_run_id = event
                    .agent
                    .parent_sub_run_id
                    .as_ref()
                    .map(ToString::to_string);
                builder.child_session_id = event
                    .agent
                    .child_session_id
                    .as_ref()
                    .map(ToString::to_string);
                builder.storage_mode = event.agent.storage_mode;
                builder.resolved_overrides = Some(resolved_overrides);
                builder.resolved_limits = resolved_limits;
                builder.started_at = timestamp;
                builder.start_storage_seq = event.storage_seq;
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::SubRunStarted,
                    timestamp,
                    Some(sub_run_id),
                    None,
                );
            },
            StorageEventPayload::SubRunFinished {
                tool_call_id,
                result,
                step_count,
                estimated_tokens,
                timestamp,
            } => {
                let Some(sub_run_id) = event.agent.sub_run_id.as_ref().map(ToString::to_string)
                else {
                    return;
                };
                let builder = self.sub_run_builder(&sub_run_id);
                builder.tool_call_id = tool_call_id.clone().or(builder.tool_call_id.clone());
                builder.agent_id = event.agent.agent_id.as_ref().map(ToString::to_string);
                builder.agent_profile = event.agent.agent_profile.clone();
                builder.parent_turn_id =
                    event.agent.parent_turn_id.as_ref().map(ToString::to_string);
                builder.parent_sub_run_id = event
                    .agent
                    .parent_sub_run_id
                    .as_ref()
                    .map(ToString::to_string);
                builder.child_session_id = event
                    .agent
                    .child_session_id
                    .as_ref()
                    .map(ToString::to_string);
                builder.storage_mode = event.agent.storage_mode;
                builder.result = Some(result);
                builder.step_count = Some(step_count);
                builder.estimated_tokens = Some(estimated_tokens);
                builder.finished_at = timestamp;
                builder.end_storage_seq = event.storage_seq;
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::SubRunFinished,
                    timestamp,
                    Some(sub_run_id),
                    None,
                );
            },
            StorageEventPayload::AgentCollaborationFact { fact, timestamp } => {
                let summary = summarize_fact(event.storage_seq, &fact);
                if let Some(sub_run_id) = &summary.child_sub_run_id {
                    if let Some(sub_run) = self.sub_runs.get_mut(sub_run_id) {
                        sub_run.collaboration_facts.push(summary.clone());
                    }
                }
                self.trace.collaboration_facts.push(summary.clone());
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::AgentCollaborationFact,
                    timestamp,
                    Some(summary.fact_id.clone()),
                    summary.summary.clone(),
                );
            },
            StorageEventPayload::Error { message, timestamp } => {
                self.trace.errors.push(ErrorTrace {
                    storage_seq: event.storage_seq,
                    agent: event.agent,
                    message: message.clone(),
                    timestamp,
                });
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::Error,
                    timestamp,
                    None,
                    Some(message),
                );
            },
            StorageEventPayload::TurnDone {
                timestamp, reason, ..
            } => {
                self.trace.completed_at = Some(timestamp);
                self.trace.completion_reason = reason;
                self.trace.incomplete = false;
                self.push_timeline(
                    event.storage_seq,
                    TurnTimelineEventKind::TurnDone,
                    Some(timestamp),
                    None,
                    None,
                );
            },
            StorageEventPayload::ChildSessionNotification { .. }
            | StorageEventPayload::ModeChanged { .. }
            | StorageEventPayload::AgentInputQueued { .. }
            | StorageEventPayload::AgentInputBatchStarted { .. }
            | StorageEventPayload::AgentInputBatchAcked { .. }
            | StorageEventPayload::AgentInputDiscarded { .. }
            | StorageEventPayload::SessionStart { .. } => {},
        }
    }

    fn tool_builder(&mut self, tool_call_id: &str) -> &mut ToolCallBuilder {
        if !self.tools.contains_key(tool_call_id) {
            self.tool_order.push(tool_call_id.to_string());
        }
        self.tools
            .entry(tool_call_id.to_string())
            .or_insert_with(|| ToolCallBuilder::new(tool_call_id.to_string()))
    }

    fn sub_run_builder(&mut self, sub_run_id: &str) -> &mut SubRunBuilder {
        if !self.sub_runs.contains_key(sub_run_id) {
            self.sub_run_order.push(sub_run_id.to_string());
        }
        self.sub_runs
            .entry(sub_run_id.to_string())
            .or_insert_with(|| SubRunBuilder::new(sub_run_id.to_string()))
    }

    fn record_seq(&mut self, storage_seq: Option<u64>) {
        let Some(storage_seq) = storage_seq else {
            return;
        };
        let range = self.trace.storage_seq_range.get_or_insert(StorageSeqRange {
            start: storage_seq,
            end: storage_seq,
        });
        range.start = range.start.min(storage_seq);
        range.end = range.end.max(storage_seq);
    }

    fn record_lineage(&mut self, agent: &AgentEventContext) {
        if agent.is_empty() {
            return;
        }
        let entry = AgentLineageEntry {
            agent_id: agent.agent_id.as_ref().map(ToString::to_string),
            parent_turn_id: agent.parent_turn_id.as_ref().map(ToString::to_string),
            agent_profile: agent.agent_profile.clone(),
            sub_run_id: agent.sub_run_id.as_ref().map(ToString::to_string),
            parent_sub_run_id: agent.parent_sub_run_id.as_ref().map(ToString::to_string),
            invocation_kind: agent.invocation_kind,
            storage_mode: agent.storage_mode,
            child_session_id: agent.child_session_id.as_ref().map(ToString::to_string),
        };
        let key = lineage_key(&entry);
        if self.lineage_seen.insert(key) {
            self.trace.agent_lineage.push(entry);
        }
    }

    fn push_timeline(
        &mut self,
        storage_seq: Option<u64>,
        kind: TurnTimelineEventKind,
        timestamp: Option<DateTime<Utc>>,
        subject_id: Option<String>,
        summary: Option<String>,
    ) {
        self.trace.timeline.push(TurnTimelineEvent {
            storage_seq,
            kind,
            timestamp,
            subject_id,
            summary,
        });
    }

    fn finish(mut self) -> TurnTrace {
        let tool_order = std::mem::take(&mut self.tool_order);
        self.trace.tool_calls = tool_order
            .into_iter()
            .filter_map(|tool_call_id| self.tools.remove(&tool_call_id))
            .map(ToolCallBuilder::finish)
            .collect();

        let sub_run_order = std::mem::take(&mut self.sub_run_order);
        self.trace.sub_runs = sub_run_order
            .into_iter()
            .filter_map(|sub_run_id| self.sub_runs.remove(&sub_run_id))
            .map(SubRunBuilder::finish)
            .collect();

        self.trace
    }
}

struct ToolCallBuilder {
    tool_call_id: String,
    tool_name: String,
    args: Value,
    output: Option<String>,
    success: Option<bool>,
    error: Option<String>,
    metadata: Option<Value>,
    continuation: Option<astrcode_core::ExecutionContinuation>,
    duration_ms: Option<u64>,
    started_storage_seq: Option<u64>,
    finished_storage_seq: Option<u64>,
    stream_deltas: Vec<ToolCallDeltaRecord>,
    persisted_reference: Option<ToolResultReferenceTrace>,
}

impl ToolCallBuilder {
    fn new(tool_call_id: String) -> Self {
        Self {
            tool_call_id,
            tool_name: String::new(),
            args: Value::Null,
            output: None,
            success: None,
            error: None,
            metadata: None,
            continuation: None,
            duration_ms: None,
            started_storage_seq: None,
            finished_storage_seq: None,
            stream_deltas: Vec::new(),
            persisted_reference: None,
        }
    }

    fn finish(self) -> ToolCallRecord {
        ToolCallRecord {
            tool_call_id: self.tool_call_id,
            tool_name: self.tool_name,
            args: self.args,
            output: self.output,
            success: self.success,
            error: self.error,
            metadata: self.metadata,
            continuation: self.continuation,
            duration_ms: self.duration_ms,
            started_storage_seq: self.started_storage_seq,
            finished_storage_seq: self.finished_storage_seq,
            stream_deltas: self.stream_deltas,
            persisted_reference: self.persisted_reference,
        }
    }
}

struct SubRunBuilder {
    sub_run_id: String,
    tool_call_id: Option<String>,
    agent_id: Option<String>,
    agent_profile: Option<String>,
    parent_turn_id: Option<String>,
    parent_sub_run_id: Option<String>,
    child_session_id: Option<String>,
    storage_mode: Option<astrcode_core::SubRunStorageMode>,
    resolved_overrides: Option<astrcode_core::ResolvedSubagentContextOverrides>,
    resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    step_count: Option<u32>,
    estimated_tokens: Option<u64>,
    result: Option<astrcode_core::SubRunResult>,
    collaboration_facts: Vec<CollaborationFactSummary>,
    start_storage_seq: Option<u64>,
    end_storage_seq: Option<u64>,
}

impl SubRunBuilder {
    fn new(sub_run_id: String) -> Self {
        Self {
            sub_run_id,
            tool_call_id: None,
            agent_id: None,
            agent_profile: None,
            parent_turn_id: None,
            parent_sub_run_id: None,
            child_session_id: None,
            storage_mode: None,
            resolved_overrides: None,
            resolved_limits: Default::default(),
            started_at: None,
            finished_at: None,
            step_count: None,
            estimated_tokens: None,
            result: None,
            collaboration_facts: Vec::new(),
            start_storage_seq: None,
            end_storage_seq: None,
        }
    }

    fn finish(self) -> SubRunTrace {
        let duration_ms = match (self.started_at, self.finished_at) {
            (Some(started_at), Some(finished_at)) => finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .try_into()
                .ok(),
            _ => None,
        };

        let storage_seq_range = match (self.start_storage_seq, self.end_storage_seq) {
            (Some(start), Some(end)) => Some(StorageSeqRange { start, end }),
            (Some(start), None) => Some(StorageSeqRange { start, end: start }),
            (None, Some(end)) => Some(StorageSeqRange { start: end, end }),
            (None, None) => None,
        };

        SubRunTrace {
            sub_run_id: self.sub_run_id,
            tool_call_id: self.tool_call_id,
            agent_id: self.agent_id,
            agent_profile: self.agent_profile,
            parent_turn_id: self.parent_turn_id,
            parent_sub_run_id: self.parent_sub_run_id,
            child_session_id: self.child_session_id,
            storage_mode: self.storage_mode,
            resolved_overrides: self.resolved_overrides,
            resolved_limits: self.resolved_limits,
            started_at: self.started_at,
            finished_at: self.finished_at,
            duration_ms,
            step_count: self.step_count,
            estimated_tokens: self.estimated_tokens,
            result: self.result,
            collaboration_facts: self.collaboration_facts,
            storage_seq_range,
        }
    }
}

fn summarize_fact(
    storage_seq: Option<u64>,
    fact: &AgentCollaborationFact,
) -> CollaborationFactSummary {
    CollaborationFactSummary {
        storage_seq,
        fact_id: fact.fact_id.to_string(),
        action: fact.action,
        outcome: fact.outcome,
        parent_session_id: fact.parent_session_id.to_string(),
        turn_id: fact.turn_id.to_string(),
        parent_agent_id: fact.parent_agent_id.as_ref().map(ToString::to_string),
        child_agent_id: fact.child_agent_id().map(ToString::to_string),
        child_session_id: fact.child_session_id().map(ToString::to_string),
        child_sub_run_id: fact.child_sub_run_id().map(ToString::to_string),
        delivery_id: fact.delivery_id.as_ref().map(ToString::to_string),
        reason_code: fact.reason_code.clone(),
        summary: fact.summary.clone(),
        latency_ms: fact.latency_ms,
        source_tool_call_id: fact.source_tool_call_id.as_ref().map(ToString::to_string),
    }
}

fn lineage_key(entry: &AgentLineageEntry) -> String {
    format!(
        "{}|{}|{}|{}|{}|{:?}|{:?}|{}",
        entry.agent_id.as_deref().unwrap_or_default(),
        entry.parent_turn_id.as_deref().unwrap_or_default(),
        entry.agent_profile.as_deref().unwrap_or_default(),
        entry.sub_run_id.as_deref().unwrap_or_default(),
        entry.parent_sub_run_id.as_deref().unwrap_or_default(),
        entry.invocation_kind,
        entry.storage_mode,
        entry.child_session_id.as_deref().unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use astrcode_core::{
        AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
        AgentCollaborationPolicyContext, AgentEventContext, ChildExecutionIdentity, InvocationKind,
        PersistedToolOutput, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
        StorageEvent, StorageEventPayload, StoredEvent, SubRunStorageMode, ToolOutputStream,
    };
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::TraceExtractor;

    fn persisted_output() -> PersistedToolOutput {
        PersistedToolOutput {
            storage_kind: "file".to_string(),
            absolute_path: "D:/workspace/.astrcode/tool-results/call-1.txt".to_string(),
            relative_path: "tool-results/call-1.txt".to_string(),
            total_bytes: 128,
            preview_text: "hello".to_string(),
            preview_bytes: 5,
        }
    }

    fn line(storage_seq: u64, event: StorageEvent) -> String {
        serde_json::to_string(&StoredEvent { storage_seq, event }).expect("line should serialize")
    }

    #[test]
    fn extractor_merges_tool_call_lifecycle_with_stream_and_reference() {
        let turn_id = "turn-1";
        let agent = AgentEventContext::root_execution("agent-root", "default");
        let events = [
            line(
                1,
                StorageEvent {
                    turn_id: None,
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::SessionStart {
                        session_id: "session-1".to_string(),
                        timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
                        working_dir: "D:/workspace".to_string(),
                        parent_session_id: None,
                        parent_storage_seq: None,
                    },
                },
            ),
            line(
                2,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::ToolCall {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "Read".to_string(),
                        args: json!({"path": "README.md"}),
                    },
                },
            ),
            line(
                3,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::ToolCallDelta {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "Read".to_string(),
                        stream: ToolOutputStream::Stdout,
                        delta: "hel".to_string(),
                    },
                },
            ),
            line(
                4,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::ToolResult {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "Read".to_string(),
                        output: "hello".to_string(),
                        success: true,
                        error: None,
                        metadata: None,
                        continuation: None,
                        duration_ms: 22,
                    },
                },
            ),
            line(
                5,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent,
                    payload: StorageEventPayload::ToolResultReferenceApplied {
                        tool_call_id: "call-1".to_string(),
                        persisted_output: persisted_output(),
                        replacement: "<persisted-output>".to_string(),
                        original_bytes: 128,
                    },
                },
            ),
        ];

        let input = Cursor::new(events.join("\n"));
        let trace = TraceExtractor::extract_reader(input).expect("extract should succeed");
        let tool_call = &trace.turns[0].tool_calls[0];
        assert_eq!(tool_call.tool_name, "Read");
        assert_eq!(tool_call.stream_deltas.len(), 1);
        assert_eq!(tool_call.output.as_deref(), Some("hello"));
        assert_eq!(
            tool_call
                .persisted_reference
                .as_ref()
                .map(|item| item.original_bytes),
            Some(128)
        );
    }

    #[test]
    fn extractor_keeps_incomplete_turn_without_turn_done() {
        let input = Cursor::new(line(
            1,
            StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::root_execution("agent-root", "default"),
                payload: StorageEventPayload::UserMessage {
                    content: "hello".to_string(),
                    timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
                    origin: Default::default(),
                },
            },
        ));

        let trace = TraceExtractor::extract_reader(input).expect("extract should succeed");
        assert_eq!(trace.turns.len(), 1);
        assert!(trace.turns[0].incomplete);
    }

    #[test]
    fn extractor_links_collaboration_facts_to_sub_runs() {
        let turn_id = "turn-1";
        let agent = AgentEventContext {
            agent_id: Some("agent-child".into()),
            parent_turn_id: Some(turn_id.into()),
            agent_profile: Some("worker".to_string()),
            sub_run_id: Some("sub-1".into()),
            parent_sub_run_id: None,
            invocation_kind: Some(InvocationKind::SubRun),
            storage_mode: Some(SubRunStorageMode::IndependentSession),
            child_session_id: Some("session-child".into()),
        };

        let fact = AgentCollaborationFact {
            fact_id: "fact-1".into(),
            action: AgentCollaborationActionKind::Spawn,
            outcome: AgentCollaborationOutcomeKind::Accepted,
            parent_session_id: "session-parent".into(),
            turn_id: turn_id.into(),
            parent_agent_id: Some("agent-root".into()),
            child_identity: Some(ChildExecutionIdentity {
                agent_id: "agent-child".into(),
                session_id: "session-child".into(),
                sub_run_id: "sub-1".into(),
            }),
            delivery_id: None,
            reason_code: None,
            summary: Some("delegated".to_string()),
            latency_ms: Some(8),
            source_tool_call_id: Some("call-1".into()),
            mode_id: None,
            governance_revision: None,
            policy: AgentCollaborationPolicyContext {
                policy_revision: "rev-1".to_string(),
                max_subrun_depth: 2,
                max_spawn_per_turn: 4,
            },
        };

        let lines = [
            line(
                1,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::SubRunStarted {
                        tool_call_id: Some("call-1".to_string()),
                        resolved_overrides: ResolvedSubagentContextOverrides::default(),
                        resolved_limits: ResolvedExecutionLimitsSnapshot,
                        timestamp: Some(Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap()),
                    },
                },
            ),
            line(
                2,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::AgentCollaborationFact {
                        fact,
                        timestamp: Some(Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 1).unwrap()),
                    },
                },
            ),
            line(
                3,
                StorageEvent {
                    turn_id: Some(turn_id.to_string()),
                    agent,
                    payload: StorageEventPayload::SubRunFinished {
                        tool_call_id: Some("call-1".to_string()),
                        result: astrcode_core::SubRunResult::Completed {
                            outcome: astrcode_core::CompletedSubRunOutcome::Completed,
                            handoff: astrcode_core::SubRunHandoff {
                                findings: Vec::new(),
                                artifacts: Vec::new(),
                                delivery: None,
                            },
                        },
                        step_count: 2,
                        estimated_tokens: 55,
                        timestamp: Some(Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 2).unwrap()),
                    },
                },
            ),
        ];

        let trace = TraceExtractor::extract_reader(Cursor::new(lines.join("\n")))
            .expect("extract should succeed");
        assert_eq!(trace.turns[0].collaboration_facts.len(), 1);
        assert_eq!(trace.turns[0].sub_runs[0].collaboration_facts.len(), 1);
    }
}
