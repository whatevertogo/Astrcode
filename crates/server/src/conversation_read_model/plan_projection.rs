use astrcode_tool_contract::ToolExecutionResult;
use serde_json::Value;

use super::*;

pub(super) fn plan_block_from_tool_result(
    turn_id: &str,
    result: &ToolExecutionResult,
) -> Option<ConversationPlanBlockFacts> {
    if !result.ok {
        return None;
    }

    let metadata = result.metadata.as_ref()?.as_object()?;
    match result.tool_name.as_str() {
        "upsertSessionPlan" => {
            let title = json_string(metadata, "title")?;
            let plan_path = json_string(metadata, "planPath")?;
            Some(ConversationPlanBlockFacts {
                id: format!("plan:{}:saved", result.tool_call_id),
                turn_id: Some(turn_id.to_string()),
                tool_call_id: result.tool_call_id.clone(),
                event_kind: ConversationPlanEventKind::Saved,
                title,
                plan_path,
                summary: Some(super::tool_result_summary(result)),
                status: json_string(metadata, "status"),
                slug: json_string(metadata, "slug"),
                updated_at: json_string(metadata, "updatedAt"),
                content: None,
                review: None,
                blockers: ConversationPlanBlockersFacts::default(),
            })
        },
        "exitPlanMode" => match json_string(metadata, "schema").as_deref() {
            Some("sessionPlanExit") => plan_presented_block(turn_id, result, metadata),
            Some("sessionPlanExitReviewPending") | Some("sessionPlanExitBlocked") => {
                plan_review_pending_block(turn_id, result, metadata)
            },
            _ => None,
        },
        _ => None,
    }
}

fn plan_presented_block(
    turn_id: &str,
    result: &ToolExecutionResult,
    metadata: &serde_json::Map<String, Value>,
) -> Option<ConversationPlanBlockFacts> {
    let plan = metadata.get("plan")?.as_object()?;
    Some(ConversationPlanBlockFacts {
        id: format!("plan:{}:presented", result.tool_call_id),
        turn_id: Some(turn_id.to_string()),
        tool_call_id: result.tool_call_id.clone(),
        event_kind: ConversationPlanEventKind::Presented,
        title: json_string(plan, "title")?,
        plan_path: json_string(plan, "planPath")?,
        summary: Some("计划已呈递".to_string()),
        status: json_string(plan, "status"),
        slug: json_string(plan, "slug"),
        updated_at: json_string(plan, "updatedAt"),
        content: json_string(plan, "content"),
        review: None,
        blockers: ConversationPlanBlockersFacts::default(),
    })
}

fn plan_review_pending_block(
    turn_id: &str,
    result: &ToolExecutionResult,
    metadata: &serde_json::Map<String, Value>,
) -> Option<ConversationPlanBlockFacts> {
    let plan = metadata.get("plan")?.as_object()?;
    let review = metadata
        .get("review")
        .and_then(Value::as_object)
        .and_then(|review| {
            let kind = match json_string(review, "kind").as_deref() {
                Some("revise_plan") => ConversationPlanReviewKind::RevisePlan,
                Some("final_review") => ConversationPlanReviewKind::FinalReview,
                _ => return None,
            };
            Some(ConversationPlanReviewFacts {
                kind,
                checklist: json_string_array(review, "checklist"),
            })
        });
    let blockers = metadata
        .get("blockers")
        .and_then(Value::as_object)
        .map(|blockers| ConversationPlanBlockersFacts {
            missing_headings: json_string_array(blockers, "missingHeadings"),
            invalid_sections: json_string_array(blockers, "invalidSections"),
        })
        .unwrap_or_default();

    Some(ConversationPlanBlockFacts {
        id: format!("plan:{}:review-pending", result.tool_call_id),
        turn_id: Some(turn_id.to_string()),
        tool_call_id: result.tool_call_id.clone(),
        event_kind: ConversationPlanEventKind::ReviewPending,
        title: json_string(plan, "title")?,
        plan_path: json_string(plan, "planPath")?,
        summary: Some(match review.as_ref().map(|review| review.kind) {
            Some(ConversationPlanReviewKind::RevisePlan) => "正在修计划".to_string(),
            Some(ConversationPlanReviewKind::FinalReview) => "正在做退出前自审".to_string(),
            None => "继续完善中".to_string(),
        }),
        status: None,
        slug: None,
        updated_at: None,
        content: None,
        review,
        blockers,
    })
}

fn json_string(container: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    container
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn json_string_array(container: &serde_json::Map<String, Value>, key: &str) -> Vec<String> {
    container
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}
