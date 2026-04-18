use astrcode_protocol::http::{
    CompactSessionRequest, CompactSessionResponse, CreateSessionRequest, DeleteProjectResultDto,
    PromptAcceptedResponse, PromptRequest, SessionListItem,
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
        .app
        .create_session(request.working_dir)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_session_list_item(
        astrcode_application::summarize_session_meta(meta),
    )))
}

pub(crate) async fn submit_prompt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<PromptRequest>,
) -> Result<(StatusCode, Json<PromptAcceptedResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let summary = state
        .app
        .submit_prompt_summary(
            &session_id,
            request.text,
            request.control,
            request.skill_invocation.map(|invocation| {
                astrcode_application::PromptSkillInvocation {
                    skill_id: invocation.skill_id,
                    user_prompt: invocation.user_prompt,
                }
            }),
        )
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(PromptAcceptedResponse {
            turn_id: summary.turn_id,
            session_id: summary.session_id,
            branched_from_session_id: summary.branched_from_session_id,
            accepted_control: summary.accepted_control,
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
    let summary = state
        .app
        .compact_session_summary(
            &session_id,
            request.as_ref().and_then(|request| request.control.clone()),
            request.and_then(|request| request.instructions),
        )
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(CompactSessionResponse {
            accepted: summary.accepted,
            deferred: summary.deferred,
            message: summary.message,
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
    Ok(Json(result))
}
