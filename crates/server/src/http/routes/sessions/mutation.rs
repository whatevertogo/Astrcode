use std::{fs, path::Path as FsPath};

use astrcode_core::{ExecutionControl, SessionId};
use astrcode_governance_contract::ModeId;
use astrcode_host_session::{
    CompactSessionMutationInput, ForkPoint, InterruptSessionMutationInput, TurnMutationPreparation,
};
use astrcode_protocol::http::{
    CompactSessionRequest, CompactSessionResponse, CreateSessionRequest, DeleteProjectResultDto,
    ForkSessionRequest, PromptRequest, PromptSubmitResponse, SessionListItem, SessionModeStateDto,
    SwitchModeRequest,
};
use astrcode_support::hostpaths::project_dir;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{
    ApiError, AppState,
    auth::require_auth,
    mapper::to_session_list_item,
    root_execute_service::{ServerAgentExecuteSummary, ServerSessionPromptRequest},
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
        .session_catalog
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
) -> Result<(StatusCode, Json<PromptSubmitResponse>), ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let loaded = state
        .session_catalog
        .ensure_loaded_session(&SessionId::from(session_id.clone()))
        .await
        .map_err(ApiError::from)?;
    let text = normalize_prompt_request_text(request.text, request.skill_invocation)?;
    let outcome = state
        .agent_api
        .submit_existing_session_prompt(ServerSessionPromptRequest {
            session_id,
            working_dir: loaded.working_dir.display().to_string(),
            text,
        })
        .await
        .map_err(ApiError::from)?;
    Ok(match outcome {
        ServerAgentExecuteSummary::Accepted {
            session_id,
            turn_id,
            branched_from_session_id,
            ..
        } => (
            StatusCode::ACCEPTED,
            Json(PromptSubmitResponse::Accepted {
                session_id,
                turn_id,
                branched_from_session_id,
            }),
        ),
        ServerAgentExecuteSummary::Handled {
            session_id,
            message,
            ..
        } => (
            StatusCode::OK,
            Json(PromptSubmitResponse::Handled {
                session_id,
                message,
            }),
        ),
    })
}

pub(crate) async fn interrupt_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    state
        .session_catalog
        .interrupt_running_turn(InterruptSessionMutationInput {
            session_id: SessionId::from(session_id),
        })
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
        .session_catalog
        .request_manual_compact(CompactSessionMutationInput {
            session_id: SessionId::from(session_id),
            control: normalize_compact_control(
                request.as_ref().and_then(|request| request.control.clone()),
            ),
            instructions: normalize_compact_instructions(
                request.and_then(|request| request.instructions),
            ),
            preparation: TurnMutationPreparation::external_preparation("server/application"),
        })
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

fn normalize_prompt_request_text(
    text: String,
    skill_invocation: Option<astrcode_protocol::http::PromptSkillInvocation>,
) -> Result<String, ApiError> {
    let text = text.trim().to_string();
    let Some(skill_invocation) = skill_invocation else {
        if text.is_empty() {
            return Err(ApiError::bad_request(
                "prompt must not be empty".to_string(),
            ));
        }
        return Ok(text);
    };

    let skill_prompt = skill_invocation
        .user_prompt
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if !text.is_empty() && !skill_prompt.is_empty() && text != skill_prompt {
        return Err(ApiError::bad_request(
            "skillInvocation.userPrompt must match prompt text".to_string(),
        ));
    }
    if !text.is_empty() {
        Ok(text)
    } else {
        Ok(skill_prompt)
    }
}

fn normalize_compact_control(control: Option<ExecutionControl>) -> Option<ExecutionControl> {
    let mut control = control.unwrap_or(ExecutionControl {
        manual_compact: None,
    });
    if control.manual_compact.is_none() {
        control.manual_compact = Some(true);
    }
    Some(control)
}

fn normalize_compact_instructions(instructions: Option<String>) -> Option<String> {
    instructions
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) async fn fork_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    request: Option<Json<ForkSessionRequest>>,
) -> Result<Json<SessionListItem>, ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let request = request
        .map(|request| request.0)
        .unwrap_or(ForkSessionRequest {
            turn_id: None,
            storage_seq: None,
        });
    if request.turn_id.is_some() && request.storage_seq.is_some() {
        return Err(ApiError::bad_request(
            "turnId and storageSeq are mutually exclusive".to_string(),
        ));
    }
    let fork_point = match (request.turn_id, request.storage_seq) {
        (Some(turn_id), None) => ForkPoint::TurnEnd(turn_id),
        (None, Some(storage_seq)) => ForkPoint::StorageSeq(storage_seq),
        (None, None) => ForkPoint::Latest,
        (Some(_), Some(_)) => unreachable!("validated above"),
    };
    let source_session_id = SessionId::from(session_id.clone());
    let source = state
        .session_catalog
        .ensure_loaded_session(&source_session_id)
        .await
        .map_err(ApiError::from)?;
    let result = state
        .session_catalog
        .fork_session(&source_session_id, fork_point)
        .await
        .map_err(ApiError::from)?;
    copy_fork_plan_artifacts(
        &session_id,
        result.new_session_id.as_str(),
        source.working_dir.as_path(),
    )?;
    let meta = state
        .session_catalog
        .list_session_metas()
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .find(|meta| meta.session_id == result.new_session_id.as_str())
        .ok_or_else(|| {
            ApiError::internal_server_error(format!(
                "forked session '{}' was not found in catalog",
                result.new_session_id
            ))
        })?;
    Ok(Json(to_session_list_item(meta)))
}

pub(crate) async fn switch_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(request): Json<SwitchModeRequest>,
) -> Result<(StatusCode, Json<SessionModeStateDto>), ApiError> {
    require_auth(&state, &headers, None)?;
    let session_id = validate_session_path_id(&session_id)?;
    let session_id = SessionId::from(session_id);
    let current_mode = state
        .session_catalog
        .session_mode_state(&session_id)
        .await
        .map_err(ApiError::from)?;
    let current_mode_id = current_mode.current_mode_id.clone();
    let target_mode_id = ModeId::from(request.mode_id.clone());
    state
        .mode_catalog
        .validate_transition(&current_mode_id, &target_mode_id)
        .map_err(ApiError::from)?;
    let mode = state
        .session_catalog
        .switch_mode(&session_id, request.mode_id.into())
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(SessionModeStateDto {
            current_mode_id: mode.current_mode_id.to_string(),
            last_mode_changed_at: mode
                .last_mode_changed_at
                .map(astrcode_core::format_local_rfc3339),
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
        .session_catalog
        .delete_session(&SessionId::from(session_id))
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
    let working_dir = fs::canonicalize(&query.working_dir).map_err(|error| {
        ApiError::bad_request(format!(
            "invalid workingDir '{}': {error}",
            query.working_dir
        ))
    })?;
    let result = state
        .session_catalog
        .delete_project(&working_dir.display().to_string())
        .await
        .map_err(ApiError::from)?;
    Ok(Json(result))
}

fn copy_fork_plan_artifacts(
    source_session_id: &str,
    target_session_id: &str,
    working_dir: &FsPath,
) -> Result<(), ApiError> {
    let project_dir = project_dir(working_dir).map_err(|error| {
        ApiError::internal_server_error(format!(
            "failed to resolve project directory for '{}': {error}",
            working_dir.display()
        ))
    })?;
    let source_dir = project_dir
        .join("sessions")
        .join(source_session_id)
        .join("plan");
    if !source_dir.exists() {
        return Ok(());
    }
    let target_dir = project_dir
        .join("sessions")
        .join(target_session_id)
        .join("plan");
    copy_dir_recursive(&source_dir, &target_dir)
}

fn copy_dir_recursive(source: &FsPath, target: &FsPath) -> Result<(), ApiError> {
    fs::create_dir_all(target).map_err(|error| {
        ApiError::internal_server_error(format!(
            "creating directory '{}' failed: {error}",
            target.display()
        ))
    })?;
    for entry in fs::read_dir(source).map_err(|error| {
        ApiError::internal_server_error(format!(
            "reading directory '{}' failed: {error}",
            source.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            ApiError::internal_server_error(format!(
                "reading directory entry '{}' failed: {error}",
                source.display()
            ))
        })?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| {
            ApiError::internal_server_error(format!(
                "reading file type '{}' failed: {error}",
                source_path.display()
            ))
        })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).map_err(|error| {
                ApiError::internal_server_error(format!(
                    "copying file '{}' failed: {error}",
                    source_path.display()
                ))
            })?;
        }
    }
    Ok(())
}
