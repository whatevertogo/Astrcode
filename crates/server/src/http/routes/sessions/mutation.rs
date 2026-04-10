use astrcode_core::ExecutionAccepted;
use astrcode_protocol::http::{
    CreateSessionRequest, DeleteProjectResultDto, PromptAcceptedResponse, PromptRequest,
    SessionListItem,
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
        .service
        .sessions()
        .create(request.working_dir)
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
        .service
        .execution()
        .submit_prompt(&session_id, request.text)
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(PromptAcceptedResponse {
            turn_id: accepted.turn_id,
            session_id: accepted.session_id,
            branched_from_session_id: accepted.branched_from_session_id,
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
        .service
        .execution()
        .interrupt_session(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn compact_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    state
        .service
        .sessions()
        .compact(&session_id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    state
        .service
        .sessions()
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
        .service
        .sessions()
        .delete_project(&query.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DeleteProjectResultDto {
        success_count: result.success_count,
        failed_session_ids: result.failed_session_ids,
    }))
}
