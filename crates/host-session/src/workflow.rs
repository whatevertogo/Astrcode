use std::collections::BTreeMap;

use astrcode_core::mode::ModeId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// workflow 的稳定定义。
///
/// Why: workflow 是跨 turn、跨 mode 的正式编排协议，不应散落在 application 的 if/else 中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initial_phase_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<WorkflowPhaseDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<WorkflowTransitionDef>,
}

/// 单个 workflow phase 的稳定定义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPhaseDef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub phase_id: String,
    #[serde(default)]
    pub mode_id: ModeId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_signals: Vec<WorkflowSignal>,
}

/// 两个 phase 之间的稳定迁移定义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTransitionDef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transition_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_phase_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_phase_id: String,
    pub trigger: WorkflowTransitionTrigger,
}

/// workflow phase 间迁移的触发器。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowTransitionTrigger {
    #[default]
    Manual,
    Signal {
        signal: WorkflowSignal,
    },
    Auto {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        condition_id: String,
    },
}

/// workflow 层消费的 typed 用户/系统信号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSignal {
    #[default]
    Approve,
    RequestChanges,
    Replan,
    Cancel,
}

/// workflow phase 间 bridge 的稳定 envelope。
///
/// Why: core 只定义 envelope，具体 bridge payload 由 application 侧按业务序列化到 `payload`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBridgeState {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bridge_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_phase_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_phase_id: String,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

/// workflow 工件引用的稳定持久化模型。
///
/// Why: `workflow/state.json` 同时被 application 和 adapter-tools 读写，serde 合同必须只有一个
/// owner。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowArtifactRef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artifact_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// workflow instance 的稳定持久化真相。
///
/// Why: session-scoped workflow 状态需要跨 crate 共用同一份磁盘 schema，避免重复定义漂移。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInstanceState {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub current_phase_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artifact_refs: BTreeMap<String, WorkflowArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_state: Option<WorkflowBridgeState>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use astrcode_core::mode::ModeId;
    use serde_json::json;

    use super::{
        WorkflowArtifactRef, WorkflowBridgeState, WorkflowDef, WorkflowInstanceState,
        WorkflowPhaseDef, WorkflowSignal, WorkflowTransitionDef, WorkflowTransitionTrigger,
    };

    #[test]
    fn workflow_def_serializes_with_explicit_transition_shape() {
        let workflow = WorkflowDef {
            workflow_id: "plan_execute".to_string(),
            initial_phase_id: "planning".to_string(),
            phases: vec![
                WorkflowPhaseDef {
                    phase_id: "planning".to_string(),
                    mode_id: ModeId::plan(),
                    role: "planning".to_string(),
                    artifact_kind: Some("canonical-plan".to_string()),
                    accepted_signals: vec![WorkflowSignal::Approve, WorkflowSignal::Cancel],
                },
                WorkflowPhaseDef {
                    phase_id: "executing".to_string(),
                    mode_id: ModeId::code(),
                    role: "executing".to_string(),
                    artifact_kind: None,
                    accepted_signals: vec![WorkflowSignal::Replan],
                },
            ],
            transitions: vec![WorkflowTransitionDef {
                transition_id: "plan-approved".to_string(),
                source_phase_id: "planning".to_string(),
                target_phase_id: "executing".to_string(),
                trigger: WorkflowTransitionTrigger::Signal {
                    signal: WorkflowSignal::Approve,
                },
            }],
        };

        let encoded = serde_json::to_value(&workflow).expect("workflow should serialize");
        assert_eq!(
            encoded,
            json!({
                "workflowId": "plan_execute",
                "initialPhaseId": "planning",
                "phases": [
                    {
                        "phaseId": "planning",
                        "modeId": "plan",
                        "role": "planning",
                        "artifactKind": "canonical-plan",
                        "acceptedSignals": ["approve", "cancel"]
                    },
                    {
                        "phaseId": "executing",
                        "modeId": "code",
                        "role": "executing",
                        "acceptedSignals": ["replan"]
                    }
                ],
                "transitions": [
                    {
                        "transitionId": "plan-approved",
                        "sourcePhaseId": "planning",
                        "targetPhaseId": "executing",
                        "trigger": {
                            "kind": "signal",
                            "signal": "approve"
                        }
                    }
                ]
            })
        );
    }

    #[test]
    fn workflow_protocol_defaults_are_backward_friendly() {
        let workflow = WorkflowDef::default();
        let encoded = serde_json::to_value(&workflow).expect("workflow should serialize");
        assert_eq!(encoded, json!({}));

        let trigger: WorkflowTransitionTrigger =
            serde_json::from_value(json!({"kind": "manual"})).expect("manual trigger");
        assert_eq!(trigger, WorkflowTransitionTrigger::Manual);

        let bridge = WorkflowBridgeState::default();
        let bridge_json = serde_json::to_value(&bridge).expect("bridge should serialize");
        assert_eq!(bridge_json, json!({ "schemaVersion": 0 }));
    }

    #[test]
    fn workflow_bridge_state_preserves_envelope_fields() {
        let bridge = WorkflowBridgeState {
            bridge_kind: "plan_to_execute".to_string(),
            source_phase_id: "planning".to_string(),
            target_phase_id: "executing".to_string(),
            schema_version: 2,
            payload: json!({
                "planRef": "artifact://plan/current",
                "stepCount": 3
            }),
        };

        let encoded = serde_json::to_value(&bridge).expect("bridge should serialize");
        assert_eq!(
            encoded,
            json!({
                "bridgeKind": "plan_to_execute",
                "sourcePhaseId": "planning",
                "targetPhaseId": "executing",
                "schemaVersion": 2,
                "payload": {
                    "planRef": "artifact://plan/current",
                    "stepCount": 3
                }
            })
        );
    }

    #[test]
    fn workflow_instance_state_serializes_shared_disk_schema() {
        let state = WorkflowInstanceState {
            workflow_id: "plan_execute".to_string(),
            current_phase_id: "planning".to_string(),
            artifact_refs: BTreeMap::from([(
                "canonical-plan".to_string(),
                WorkflowArtifactRef {
                    artifact_kind: "canonical-plan".to_string(),
                    path: "/tmp/plan.md".to_string(),
                    content_digest: Some("abc".to_string()),
                },
            )]),
            bridge_state: None,
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-04-21T09:00:00Z")
                .expect("timestamp should parse")
                .with_timezone(&chrono::Utc),
        };

        let encoded = serde_json::to_value(&state).expect("workflow state should serialize");
        assert_eq!(
            encoded,
            json!({
                "workflowId": "plan_execute",
                "currentPhaseId": "planning",
                "artifactRefs": {
                    "canonical-plan": {
                        "artifactKind": "canonical-plan",
                        "path": "/tmp/plan.md",
                        "contentDigest": "abc"
                    }
                },
                "updatedAt": "2026-04-21T09:00:00Z"
            })
        );
    }
}
