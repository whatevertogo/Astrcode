use astrcode_core::{
    AgentStatus, ChildAgentRef, ChildSessionLineageKind, SubRunResult, ToolExecutionResult,
};
use serde_json::{Value, json};

const TOOL_NAME: &str = "spawnAgent";
const SUBRUN_RESULT_SCHEMA: &str = "subRunResult";

pub(crate) fn invalid_params_result(tool_call_id: String, message: String) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_call_id,
        tool_name: TOOL_NAME.to_string(),
        ok: false,
        output: String::new(),
        error: Some(message),
        metadata: None,
        duration_ms: 0,
        truncated: false,
    }
}

pub(crate) fn map_subrun_result(tool_call_id: String, result: SubRunResult) -> ToolExecutionResult {
    let error = result
        .failure
        .as_ref()
        .map(|failure| failure.technical_message.clone());
    let output = tool_output_for_result(&result);
    let metadata = subrun_metadata(&result);

    ToolExecutionResult {
        tool_call_id,
        tool_name: TOOL_NAME.to_string(),
        ok: !matches!(result.status, AgentStatus::Failed),
        output,
        error,
        metadata: Some(metadata),
        duration_ms: 0,
        truncated: false,
    }
}

fn subrun_metadata(result: &SubRunResult) -> Value {
    let mut metadata = json!({
        "schema": SUBRUN_RESULT_SCHEMA,
        "outcome": status_label(result.status),
        "handoff": result.handoff,
        "failure": result.failure,
        "result": result,
    });
    if let Value::Object(object) = &mut metadata {
        object.insert(
            "schema".to_string(),
            Value::String(SUBRUN_RESULT_SCHEMA.to_string()),
        );
        if let Some(child_ref) = extract_child_ref(result) {
            if let Ok(value) = serde_json::to_value(&child_ref) {
                object.insert("agentRef".to_string(), value);
            }
            object.insert(
                "openSessionId".to_string(),
                Value::String(child_ref.open_session_id.clone()),
            );
        }
    }
    metadata
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Pending => "pending",
        AgentStatus::Running => "running",
        AgentStatus::Completed => "completed",
        AgentStatus::Cancelled => "cancelled",
        AgentStatus::Failed => "failed",
        AgentStatus::TokenExceeded => "token_exceeded",
    }
}

fn extract_child_ref(result: &SubRunResult) -> Option<ChildAgentRef> {
    let handoff = result.handoff.as_ref()?;
    let sub_run_id = artifact_id(&handoff.artifacts, "subRun")?;
    let agent_id = artifact_id(&handoff.artifacts, "agent")?;
    let session_id = artifact_id(&handoff.artifacts, "parentSession")?;
    let open_session_id = artifact_id(&handoff.artifacts, "session")?;
    let parent_agent_id = artifact_id(&handoff.artifacts, "parentAgent");

    Some(ChildAgentRef {
        agent_id,
        session_id,
        sub_run_id,
        parent_agent_id,
        lineage_kind: ChildSessionLineageKind::Spawn,
        status: result.status,
        open_session_id,
    })
}

fn artifact_id(artifacts: &[astrcode_core::ArtifactRef], kind: &str) -> Option<String> {
    artifacts
        .iter()
        .find(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.id.clone())
}

fn tool_output_for_result(result: &SubRunResult) -> String {
    match result.status {
        AgentStatus::Failed => result
            .failure
            .as_ref()
            .map(|failure| failure.display_message.clone())
            .unwrap_or_else(|| "子 Agent 执行失败。".to_string()),
        _ => result
            .handoff
            .as_ref()
            .map(|handoff| handoff.summary.clone())
            .unwrap_or_else(|| "子 Agent 未返回摘要。".to_string()),
    }
}
