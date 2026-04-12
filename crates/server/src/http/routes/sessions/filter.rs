use astrcode_core::SessionEventRecord;
use astrcode_runtime::{SessionEventFilterSpec, SubRunEventScope};
use serde::Deserialize;

use super::validate_path_id;
use crate::{ApiError, mapper::parse_event_id};

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SessionEventScopeQuery {
    #[serde(rename = "self")]
    SelfOnly,
    #[default]
    Subtree,
    DirectChildren,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionEventFilterQuery {
    pub(crate) sub_run_id: Option<String>,
    pub(crate) scope: Option<SessionEventScopeQuery>,
}

impl SessionEventFilterQuery {
    pub(crate) fn into_runtime_filter_spec(
        self,
    ) -> Result<Option<SessionEventFilterSpec>, ApiError> {
        match (self.sub_run_id, self.scope) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(ApiError::bad_request("scope requires subRunId".to_string())),
            (Some(sub_run_id), scope) => Ok(Some(SessionEventFilterSpec {
                target_sub_run_id: validate_subrun_query_id(&sub_run_id)?,
                scope: map_scope(scope.unwrap_or_default()),
            })),
        }
    }
}

pub(crate) fn record_is_after_cursor(record: &SessionEventRecord, cursor: Option<&str>) -> bool {
    let Some(cursor) = cursor else {
        return true;
    };
    match (parse_event_id(&record.event_id), parse_event_id(cursor)) {
        (Some(current), Some(after)) => current > after,
        (Some(_), None) => true,
        _ => false,
    }
}

fn map_scope(scope: SessionEventScopeQuery) -> SubRunEventScope {
    match scope {
        SessionEventScopeQuery::SelfOnly => SubRunEventScope::SelfOnly,
        SessionEventScopeQuery::Subtree => SubRunEventScope::Subtree,
        SessionEventScopeQuery::DirectChildren => SubRunEventScope::DirectChildren,
    }
}

fn validate_subrun_query_id(raw_sub_run_id: &str) -> Result<String, ApiError> {
    validate_path_id(raw_sub_run_id, None, false, "sub-run")
}
