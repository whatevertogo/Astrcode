use astrcode_application::ExecutionAccepted;
use astrcode_protocol::http::{
    CompactSessionRequest, CompactSessionResponse, CreateSessionRequest, DeleteProjectResultDto,
    ExecutionControlDto, PromptAcceptedResponse, PromptRequest, SessionListItem,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{
    ApiError, AppState, auth::require_auth, mapper::to_session_list_item,
    routes::sessions::validate_session_path_id,
};

fn to_execution_control(
    control: Option<ExecutionControlDto>,
) -> Option<astrcode_application::ExecutionControl> {
    control.map(|control| astrcode_application::ExecutionControl {
        max_steps: control.max_steps,
        manual_compact: control.manual_compact,
    })
}

fn normalize_compact_control(control: Option<ExecutionControlDto>) -> ExecutionControlDto {
    let mut control = control.unwrap_or(ExecutionControlDto {
        max_steps: None,
        manual_compact: None,
    });
    if control.manual_compact.is_none() {
        control.manual_compact = Some(true);
    }
    control
}

fn normalize_compact_instructions(instructions: Option<String>) -> Option<String> {
    instructions
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteProjectQuery {
    working_dir: String,
}

pub(crate) async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<SessionListItem>, ApiError> {
    require_auth(&state, &headers, None)?;
    let meta = state
        .app
        .create_session(request.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_session_list_item(meta)))
}

pub(crate) async fn submit_prompt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<PromptRequest>,
) -> Result<(StatusCode, Json<PromptAcceptedResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let accepted: ExecutionAccepted = state
        .app
        .submit_prompt_with_control(
            &session_id,
            request.text,
            to_execution_control(request.control.clone()),
        )
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(PromptAcceptedResponse {
            turn_id: accepted.turn_id.to_string(),
            session_id: accepted.session_id.to_string(),
            branched_from_session_id: accepted.branched_from_session_id,
            accepted_control: request.control,
        }),
    ))
}

pub(crate) async fn interrupt_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    state
        .app
        .interrupt_session(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn compact_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    request: Option<Json<CompactSessionRequest>>,
) -> Result<(StatusCode, Json<CompactSessionResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let request = request.map(|request| request.0);
    let instructions = normalize_compact_instructions(
        request
            .as_ref()
            .and_then(|request| request.instructions.clone()),
    );
    let control = normalize_compact_control(request.and_then(|request| request.control));
    let accepted = state
        .app
        .compact_session_with_options(
            &session_id,
            to_execution_control(Some(control)),
            instructions,
        )
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(CompactSessionResponse {
            accepted: true,
            deferred: accepted.deferred,
            message: if accepted.deferred {
                "手动 compact 已登记，会在当前 turn 完成后执行。".to_string()
            } else {
                "手动 compact 已执行。".to_string()
            },
        }),
    ))
}

pub(crate) async fn delete_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    state
        .app
        .delete_session(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DeleteProjectQuery>,
) -> Result<Json<DeleteProjectResultDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let result = state
        .app
        .delete_project(&query.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DeleteProjectResultDto {
        success_count: result.success_count,
        failed_session_ids: result.failed_session_ids,
    }))
}
