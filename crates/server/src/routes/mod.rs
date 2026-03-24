pub(crate) mod config;
pub(crate) mod model;
pub(crate) mod sessions;

use astrcode_protocol::http::{AuthExchangeRequest, AuthExchangeResponse};
use axum::extract::State;
use axum::routing::{delete, get, post};
use axum::{Json, Router};

use crate::{auth::secure_token_eq, ApiError, AppState};

pub(crate) fn build_api_router() -> Router<AppState> {
    Router::<AppState>::new()
        .route("/api/auth/exchange", post(exchange_auth))
        .route(
            "/api/sessions",
            post(sessions::create_session).get(sessions::list_sessions),
        )
        .route(
            "/api/sessions/:id/messages",
            get(sessions::session_messages),
        )
        .route("/api/sessions/:id/prompts", post(sessions::submit_prompt))
        .route(
            "/api/sessions/:id/interrupt",
            post(sessions::interrupt_session),
        )
        .route("/api/sessions/:id/events", get(sessions::session_events))
        .route("/api/sessions/:id", delete(sessions::delete_session))
        .route("/api/projects", delete(sessions::delete_project))
        .route("/api/config", get(config::get_config))
        .route(
            "/api/config/active-selection",
            post(config::save_active_selection),
        )
        .route("/api/models/current", get(model::get_current_model))
        .route("/api/models", get(model::list_models))
        .route("/api/models/test", post(model::test_model_connection))
}

async fn exchange_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthExchangeRequest>,
) -> Result<Json<AuthExchangeResponse>, ApiError> {
    if !secure_token_eq(&request.token, &state.bootstrap_token) {
        return Err(ApiError::unauthorized());
    }

    Ok(Json(AuthExchangeResponse { ok: true }))
}
