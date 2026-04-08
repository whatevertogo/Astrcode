use astrcode_core::{SubRunOutcome, SubRunResult, ToolExecutionResult};
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
        ok: !matches!(result.status, SubRunOutcome::Failed),
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
        "outcome": result.status.as_str(),
        "handoff": result.handoff,
        "failure": result.failure,
        "result": result,
    });
    if let Value::Object(object) = &mut metadata {
        object.insert(
            "schema".to_string(),
            Value::String(SUBRUN_RESULT_SCHEMA.to_string()),
        );
    }
    metadata
}

fn tool_output_for_result(result: &SubRunResult) -> String {
    match result.status {
        SubRunOutcome::Failed => result
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
