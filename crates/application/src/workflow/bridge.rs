use astrcode_core::WorkflowBridgeState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ApplicationError, workflow::state::WorkflowArtifactRef};

pub(crate) const PLAN_TO_EXECUTE_BRIDGE_KIND: &str = "plan_to_execute";
pub(crate) const PLAN_TO_EXECUTE_SCHEMA_VERSION: u32 = 1;

/// planning phase 进入 executing phase 时交接的 typed bridge。
///
/// Why: application 需要一个可测试、可序列化的 handoff 真相，而不是把 approved plan
/// 仅作为自由文本 prompt 暗示传递给 execute phase。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanToExecuteBridgeState {
    pub plan_artifact: WorkflowArtifactRef,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan_title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implementation_steps: Vec<PlanImplementationStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanImplementationStep {
    pub index: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
}

impl PlanToExecuteBridgeState {
    pub(crate) fn into_bridge_state(
        &self,
        source_phase_id: &str,
        target_phase_id: &str,
    ) -> Result<WorkflowBridgeState, ApplicationError> {
        Ok(WorkflowBridgeState {
            bridge_kind: PLAN_TO_EXECUTE_BRIDGE_KIND.to_string(),
            source_phase_id: source_phase_id.to_string(),
            target_phase_id: target_phase_id.to_string(),
            schema_version: PLAN_TO_EXECUTE_SCHEMA_VERSION,
            payload: serde_json::to_value(self).map_err(|error| {
                ApplicationError::Internal(format!(
                    "failed to serialize plan-to-execute bridge payload: {error}"
                ))
            })?,
        })
    }

    pub(crate) fn from_bridge_state(
        bridge_state: &WorkflowBridgeState,
    ) -> Result<Self, ApplicationError> {
        if bridge_state.bridge_kind != PLAN_TO_EXECUTE_BRIDGE_KIND {
            return Err(ApplicationError::InvalidArgument(format!(
                "unsupported bridge kind '{}'",
                bridge_state.bridge_kind
            )));
        }
        serde_json::from_value(bridge_state.payload.clone()).map_err(|error| {
            ApplicationError::Internal(format!(
                "failed to parse plan-to-execute bridge payload: {error}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{PlanImplementationStep, PlanToExecuteBridgeState};
    use crate::workflow::state::WorkflowArtifactRef;

    #[test]
    fn plan_to_execute_bridge_round_trips_through_envelope() {
        let bridge = PlanToExecuteBridgeState {
            plan_artifact: WorkflowArtifactRef {
                artifact_kind: "canonical-plan".to_string(),
                path: "/tmp/plan.md".to_string(),
                content_digest: Some("abc".to_string()),
            },
            plan_title: "Cleanup architecture".to_string(),
            implementation_steps: vec![
                PlanImplementationStep {
                    index: 1,
                    title: "Refactor runtime".to_string(),
                    summary: "收拢 state 与 query 依赖".to_string(),
                },
                PlanImplementationStep {
                    index: 2,
                    title: "补测试".to_string(),
                    summary: "覆盖回归路径".to_string(),
                },
            ],
            approved_at: Some(
                Utc.with_ymd_and_hms(2026, 4, 21, 8, 0, 0)
                    .single()
                    .expect("datetime should be valid"),
            ),
        };

        let encoded = bridge
            .into_bridge_state("planning", "executing")
            .expect("bridge should encode");
        let decoded =
            PlanToExecuteBridgeState::from_bridge_state(&encoded).expect("bridge should decode");

        assert_eq!(decoded, bridge);
        assert_eq!(encoded.bridge_kind, "plan_to_execute");
        assert_eq!(encoded.source_phase_id, "planning");
        assert_eq!(encoded.target_phase_id, "executing");
    }
}
