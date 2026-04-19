//! 输入候选（composer options）路由。
//!
//! 该接口服务于前端输入框的自动展开面板，返回已经过 runtime 统一投影的候选项。
//! 它故意不直接暴露 `SkillSpec` / `CapabilityWireDescriptor`，避免 UI 反向理解内部装配细节。

use astrcode_application::{ComposerOptionKind, ComposerOptionsRequest};
use astrcode_protocol::http::ComposerOptionsResponseDto;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;

use crate::{
    ApiError, AppState, auth::require_auth, mapper::to_composer_options_response,
    routes::sessions::validate_session_path_id,
};

/// 输入候选查询参数。
///
/// `q` 是大小写不敏感的包含匹配关键字。
/// `kinds` 使用逗号分隔，允许前端只请求自己当前需要的候选类别，避免拉回整套 surface。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComposerOptionsQuery {
    q: Option<String>,
    kinds: Option<String>,
}

/// 获取某个会话上下文下的输入候选项。
pub(crate) async fn session_composer_options(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<ComposerOptionsQuery>,
) -> Result<Json<ComposerOptionsResponseDto>, ApiError> {
    require_auth(&state, &headers, None)?;
    let _session_id = validate_session_path_id(&session_id)?;
    let requested_kinds = parse_composer_option_kinds(query.kinds.as_deref())?;
    let items = state
        .app
        .list_composer_options(
            &session_id,
            ComposerOptionsRequest {
                query: query.q,
                kinds: requested_kinds,
                // 候选面板是交互式 UI，单次响应保持上限可以避免把全部 surface
                // 一股脑推给前端造成首屏抖动。
                limit: 50,
            },
        )
        .await
        .map_err(ApiError::from)?;
    Ok(Json(to_composer_options_response(items)))
}

fn parse_composer_option_kinds(raw: Option<&str>) -> Result<Vec<ComposerOptionKind>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };

    let mut parsed = Vec::new();
    for token in raw
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let kind = match token {
            "command" => ComposerOptionKind::Command,
            "skill" => ComposerOptionKind::Skill,
            "capability" => ComposerOptionKind::Capability,
            _ => {
                return Err(ApiError {
                    status: axum::http::StatusCode::BAD_REQUEST,
                    message: format!("unsupported composer option kind: {token}"),
                });
            },
        };
        if !parsed.contains(&kind) {
            parsed.push(kind);
        }
    }
    Ok(parsed)
}
