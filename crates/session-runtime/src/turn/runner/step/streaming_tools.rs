use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{Arc, Mutex},
    time::Instant,
};

use astrcode_core::ToolCallRequest;
use serde_json::Value;
use tokio::task::JoinHandle;

use super::TurnExecutionResources;
use crate::turn::{
    llm_cycle::{StreamedToolCallDelta, ToolCallDeltaSink},
    tool_cycle::{self, BufferedToolExecution, BufferedToolExecutionRequest},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct StreamingToolStats {
    pub launched_count: usize,
    pub matched_count: usize,
    pub fallback_count: usize,
    pub discard_count: usize,
    pub overlap_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamingToolFallbackReason {
    ToolNotConcurrencySafe,
    IdentityNeverStabilized,
    ArgumentsNeverFormedStableJson,
    FinalPlanChanged,
    BufferedExecutionJoinFailed,
}

impl StreamingToolFallbackReason {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::ToolNotConcurrencySafe => "tool is not concurrency_safe",
            Self::IdentityNeverStabilized => "streamed identity never stabilized",
            Self::ArgumentsNeverFormedStableJson => {
                "streamed arguments never formed a stable JSON payload"
            },
            Self::FinalPlanChanged => "final tool plan changed after provisional execution",
            Self::BufferedExecutionJoinFailed => "buffered execution join failed",
        }
    }
}

impl fmt::Display for StreamingToolFallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Default)]
pub(super) struct StreamingToolAssembly {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: String,
    pub launched: bool,
    json_tracker: StreamingJsonTracker,
}

#[cfg(test)]
impl StreamingToolAssembly {
    pub(super) fn for_test(
        id: Option<String>,
        name: Option<String>,
        arguments: impl Into<String>,
    ) -> Self {
        let arguments = arguments.into();
        let mut assembly = Self {
            id,
            name,
            arguments,
            launched: false,
            json_tracker: StreamingJsonTracker::default(),
        };
        assembly.json_tracker.observe_chunk(&assembly.arguments);
        assembly
    }
}

struct StreamingToolCandidate {
    index: usize,
    request: ToolCallRequest,
}

#[derive(Default)]
struct StreamingToolAssembler {
    assemblies: BTreeMap<usize, StreamingToolAssembly>,
}

#[derive(Debug, Default)]
struct StreamingJsonTracker {
    started: bool,
    in_string: bool,
    escape: bool,
    object_depth: usize,
    complete: bool,
    fallback_to_full_parse: bool,
}

impl StreamingJsonTracker {
    fn observe_chunk(&mut self, chunk: &str) {
        if self.fallback_to_full_parse {
            return;
        }

        for ch in chunk.chars() {
            if self.complete {
                if !ch.is_whitespace() {
                    self.complete = false;
                    self.fallback_to_full_parse = true;
                }
                continue;
            }

            if self.in_string {
                if self.escape {
                    self.escape = false;
                    continue;
                }
                match ch {
                    '\\' => self.escape = true,
                    '"' => self.in_string = false,
                    _ => {},
                }
                continue;
            }

            if !self.started {
                if ch.is_whitespace() {
                    continue;
                }
                self.started = true;
                if ch == '{' {
                    self.object_depth = 1;
                } else {
                    self.fallback_to_full_parse = true;
                }
                continue;
            }

            match ch {
                '"' => self.in_string = true,
                '{' => {
                    self.object_depth = self.object_depth.saturating_add(1);
                },
                '}' => {
                    if self.object_depth == 0 {
                        self.fallback_to_full_parse = true;
                        return;
                    }
                    self.object_depth -= 1;
                    if self.object_depth == 0 {
                        self.complete = true;
                    }
                },
                _ => {},
            }
        }
    }

    fn should_attempt_parse(&self) -> bool {
        self.complete || self.fallback_to_full_parse
    }
}

impl StreamingToolAssembler {
    fn observe_delta(&mut self, delta: StreamedToolCallDelta) -> Option<StreamingToolCandidate> {
        let assembly = self.assemblies.entry(delta.index).or_default();
        if let Some(id) = delta.id {
            assembly.id = Some(id);
        }
        if let Some(name) = delta.name {
            assembly.name = Some(name);
        }
        assembly.arguments.push_str(&delta.arguments_delta);
        assembly.json_tracker.observe_chunk(&delta.arguments_delta);

        if assembly.launched {
            return None;
        }

        let id = assembly.id.clone()?;
        let name = assembly.name.clone()?;
        if !assembly.json_tracker.should_attempt_parse() {
            return None;
        }
        let Ok(args) = serde_json::from_str::<Value>(&assembly.arguments) else {
            return None;
        };

        Some(StreamingToolCandidate {
            index: delta.index,
            request: ToolCallRequest { id, name, args },
        })
    }

    fn mark_launched(&mut self, index: usize) {
        if let Some(assembly) = self.assemblies.get_mut(&index) {
            assembly.launched = true;
        }
    }
}

struct SpawnedStreamingTool {
    request: ToolCallRequest,
    handle: JoinHandle<BufferedToolExecution>,
}

#[derive(Default)]
struct StreamingToolLaunchContext {
    gateway: Option<astrcode_kernel::KernelGateway>,
    session_state: Option<Arc<crate::SessionState>>,
    session_id: String,
    working_dir: String,
    turn_id: String,
    agent: Option<astrcode_core::AgentEventContext>,
    cancel: Option<astrcode_core::CancelToken>,
    tool_result_inline_limit: usize,
}

#[derive(Default)]
struct StreamingToolLauncher {
    context: StreamingToolLaunchContext,
    spawned: HashMap<String, SpawnedStreamingTool>,
    stats: StreamingToolStats,
}

impl StreamingToolLauncher {
    fn from_resources(resources: &TurnExecutionResources<'_>) -> Self {
        Self {
            context: StreamingToolLaunchContext {
                gateway: Some(resources.gateway.clone()),
                session_state: Some(Arc::clone(resources.session_state)),
                session_id: resources.session_id.to_string(),
                working_dir: resources.working_dir.to_string(),
                turn_id: resources.turn_id.to_string(),
                agent: Some(resources.agent.clone()),
                cancel: Some(resources.cancel.clone()),
                tool_result_inline_limit: resources.runtime.tool_result_inline_limit,
            },
            ..Self::default()
        }
    }

    fn launch_if_ready(&mut self, candidate: &StreamingToolCandidate) -> bool {
        let Some(gateway) = self.context.gateway.as_ref() else {
            return false;
        };
        let Some(capability) = gateway
            .capabilities()
            .capability_spec(&candidate.request.name)
        else {
            return false;
        };
        if !capability.concurrency_safe {
            return false;
        }

        let Some(session_state) = self.context.session_state.as_ref() else {
            return false;
        };
        let Some(agent) = self.context.agent.as_ref() else {
            return false;
        };
        let Some(cancel) = self.context.cancel.as_ref() else {
            return false;
        };

        let request = candidate.request.clone();
        let handle = tokio::spawn(tool_cycle::execute_buffered_tool_call(
            BufferedToolExecutionRequest {
                gateway: gateway.clone(),
                session_state: Arc::clone(session_state),
                tool_call: request.clone(),
                session_id: self.context.session_id.clone(),
                working_dir: self.context.working_dir.clone(),
                turn_id: self.context.turn_id.clone(),
                agent: agent.clone(),
                cancel: cancel.clone(),
                tool_result_inline_limit: self.context.tool_result_inline_limit,
            },
        ));

        self.stats.launched_count = self.stats.launched_count.saturating_add(1);
        self.spawned
            .insert(request.id.clone(), SpawnedStreamingTool { request, handle });
        true
    }

    fn abort_all(&mut self) {
        let discarded = self.spawned.len();
        self.stats.discard_count = self.stats.discard_count.saturating_add(discarded);
        for (_, spawned_tool) in self.spawned.drain() {
            spawned_tool.handle.abort();
        }
    }
}

#[derive(Default)]
struct StreamingToolPlanner {
    assembler: StreamingToolAssembler,
    launcher: StreamingToolLauncher,
}

pub(super) struct StreamingToolFinalizeResult {
    pub matched_results: HashMap<String, BufferedToolExecution>,
    pub remaining_tool_calls: Vec<ToolCallRequest>,
    pub stats: StreamingToolStats,
    pub used_streaming_path: bool,
}

struct StreamingToolReconciler {
    gateway: Option<astrcode_kernel::KernelGateway>,
    assemblies: BTreeMap<usize, StreamingToolAssembly>,
    spawned: HashMap<String, SpawnedStreamingTool>,
    stats: StreamingToolStats,
}

impl StreamingToolReconciler {
    async fn reconcile(
        mut self,
        final_tool_calls: &[ToolCallRequest],
        llm_finished_at: Instant,
    ) -> StreamingToolFinalizeResult {
        let mut matched_results = HashMap::new();
        let mut remaining_tool_calls = Vec::new();

        for (index, call) in final_tool_calls.iter().cloned().enumerate() {
            if let Some(spawned_tool) = self.spawned.remove(&call.id) {
                if spawned_tool.request == call {
                    match spawned_tool.handle.await {
                        Ok(buffered) => {
                            self.stats.matched_count = self.stats.matched_count.saturating_add(1);
                            self.stats.overlap_ms = self
                                .stats
                                .overlap_ms
                                .saturating_add(overlap_ms(&buffered, llm_finished_at));
                            matched_results.insert(call.id.clone(), buffered);
                        },
                        Err(error) => {
                            log::warn!(
                                "turn streaming tool execution join failed for {}: {error}",
                                call.id
                            );
                            self.log_fallback_reason(
                                &call,
                                StreamingToolFallbackReason::BufferedExecutionJoinFailed,
                            );
                            remaining_tool_calls.push(call);
                        },
                    }
                } else {
                    spawned_tool.handle.abort();
                    self.stats.discard_count = self.stats.discard_count.saturating_add(1);
                    self.log_fallback_reason(&call, StreamingToolFallbackReason::FinalPlanChanged);
                    remaining_tool_calls.push(call);
                }
                continue;
            }

            if let Some(reason) = fallback_reason_for_final_call(
                self.gateway.as_ref(),
                self.assemblies.get(&index),
                &call,
            ) {
                self.log_fallback_reason(&call, reason);
            }
            remaining_tool_calls.push(call);
        }

        self.stats.discard_count = self.stats.discard_count.saturating_add(self.spawned.len());
        for (_, spawned_tool) in self.spawned.drain() {
            spawned_tool.handle.abort();
        }

        StreamingToolFinalizeResult {
            matched_results,
            remaining_tool_calls,
            stats: self.stats,
            used_streaming_path: self.stats.launched_count > 0,
        }
    }

    fn log_fallback_reason(&mut self, call: &ToolCallRequest, reason: StreamingToolFallbackReason) {
        log::debug!(
            "turn streaming tool planner fallback for {} ({}): {}",
            call.id,
            call.name,
            reason
        );
        self.stats.fallback_count = self.stats.fallback_count.saturating_add(1);
    }
}

// TODO: streaming_tools.rs 里 Arc<Mutex<...>> -> channel/collector 的并发模型替换
#[derive(Clone)]
pub(super) struct StreamingToolPlannerHandle {
    inner: Arc<Mutex<StreamingToolPlanner>>,
}

impl StreamingToolPlannerHandle {
    pub(super) fn new(resources: &TurnExecutionResources<'_>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamingToolPlanner {
                assembler: StreamingToolAssembler::default(),
                launcher: StreamingToolLauncher::from_resources(resources),
            })),
        }
    }

    pub(super) fn tool_delta_sink(&self) -> ToolCallDeltaSink {
        let inner = Arc::clone(&self.inner);
        Arc::new(move |delta| {
            inner
                .lock()
                .expect("streaming tool planner lock should work")
                .observe_delta(delta);
        })
    }

    pub(super) fn abort_all(&self) {
        self.inner
            .lock()
            .expect("streaming tool planner lock should work")
            .launcher
            .abort_all();
    }

    pub(super) async fn finalize(
        &self,
        final_tool_calls: &[ToolCallRequest],
        llm_finished_at: Instant,
    ) -> StreamingToolFinalizeResult {
        let reconciler = {
            let mut planner = self
                .inner
                .lock()
                .expect("streaming tool planner lock should work");
            let assemblies = std::mem::take(&mut planner.assembler.assemblies);
            let launcher = std::mem::take(&mut planner.launcher);
            StreamingToolReconciler {
                gateway: launcher.context.gateway,
                assemblies,
                spawned: launcher.spawned,
                stats: launcher.stats,
            }
        };

        reconciler
            .reconcile(final_tool_calls, llm_finished_at)
            .await
    }
}

impl StreamingToolPlanner {
    fn observe_delta(&mut self, delta: StreamedToolCallDelta) {
        let Some(candidate) = self.assembler.observe_delta(delta) else {
            return;
        };
        if self.launcher.launch_if_ready(&candidate) {
            self.assembler.mark_launched(candidate.index);
        }
    }
}

pub(super) fn fallback_reason_for_final_call(
    gateway: Option<&astrcode_kernel::KernelGateway>,
    assembly: Option<&StreamingToolAssembly>,
    call: &ToolCallRequest,
) -> Option<StreamingToolFallbackReason> {
    let capability = gateway?.capabilities().capability_spec(&call.name)?;
    if !capability.concurrency_safe {
        return Some(StreamingToolFallbackReason::ToolNotConcurrencySafe);
    }
    let assembly = assembly?;
    if assembly.id.as_deref() != Some(call.id.as_str())
        || assembly.name.as_deref() != Some(call.name.as_str())
    {
        return Some(StreamingToolFallbackReason::IdentityNeverStabilized);
    }
    Some(StreamingToolFallbackReason::ArgumentsNeverFormedStableJson)
}

fn overlap_ms(buffered: &BufferedToolExecution, llm_finished_at: Instant) -> u64 {
    let overlap_end = if buffered.finished_at < llm_finished_at {
        buffered.finished_at
    } else {
        llm_finished_at
    };
    if buffered.started_at >= overlap_end {
        return 0;
    }
    overlap_end.duration_since(buffered.started_at).as_millis() as u64
}
