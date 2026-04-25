//! # 声明式治理模式系统
//!
//! 定义运行时治理模式（Governance Mode）的完整 DSL：
//!
//! - **ModeId**: 模式唯一标识（内置 code / plan）
//! - **ActionPolicies**: 动作策略规则集（Allow / Deny / Ask 三种裁决效果）
//! - **GovernanceModeSpec**: 完整模式定义（动作策略 + 子策略 + 执行策略 + 提示词程序 + 转换策略）
//! - **ResolvedTurnEnvelope**: 当前命名沿用 envelope，但语义上是治理 compile 阶段产物
//!
//! 模式由声明式配置文件加载，运行时通过 `GovernanceModeSpec::validate()` 校验后，
//! 由治理层先编译为 `ResolvedTurnEnvelope`，再由 application bind 成 turn 可执行治理快照。

use serde::{Deserialize, Serialize};

use crate::{
    AstrError, ForkMode, Result, normalize_non_empty_unique_string_list, prompt::PromptDeclaration,
};

pub const BUILTIN_MODE_CODE_ID: &str = "code";
pub const BUILTIN_MODE_PLAN_ID: &str = "plan";
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModeId(String);

impl ModeId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(AstrError::Validation("modeId 不能为空".to_string()));
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn code() -> Self {
        Self(BUILTIN_MODE_CODE_ID.to_string())
    }

    pub fn plan() -> Self {
        Self(BUILTIN_MODE_PLAN_ID.to_string())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for ModeId {
    fn default() -> Self {
        Self::code()
    }
}

impl From<&str> for ModeId {
    fn from(value: &str) -> Self {
        Self(value.trim().to_string())
    }
}

impl From<String> for ModeId {
    fn from(value: String) -> Self {
        Self(value.trim().to_string())
    }
}

impl std::fmt::Display for ModeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionPolicyEffect {
    #[default]
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPolicyRule {
    pub effect: ActionPolicyEffect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActionPolicies {
    #[serde(default = "default_action_policy_effect")]
    pub default_effect: ActionPolicyEffect,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ActionPolicyRule>,
}

fn default_action_policy_effect() -> ActionPolicyEffect {
    ActionPolicyEffect::Allow
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChildPolicySpec {
    #[serde(default = "default_true")]
    pub allow_delegation: bool,
    #[serde(default = "default_true")]
    pub allow_recursive_delegation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode_id: Option<ModeId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_profile_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
    #[serde(default)]
    pub restricted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse_scope_summary: Option<String>,
}

impl ChildPolicySpec {
    pub fn validate(&self) -> Result<()> {
        normalize_non_empty_unique_string_list(
            &self.allowed_profile_ids,
            "childPolicy.allowedProfileIds",
        )?;
        if let Some(summary) = &self.reuse_scope_summary {
            validate_non_empty_trimmed("childPolicy.reuseScopeSummary", summary)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModeExecutionPolicySpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptProgramEntry {
    pub block_id: String,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_hint: Option<i32>,
}

impl PromptProgramEntry {
    pub fn validate(&self) -> Result<()> {
        validate_non_empty_trimmed("promptProgram.blockId", &self.block_id)?;
        validate_non_empty_trimmed("promptProgram.title", &self.title)?;
        validate_non_empty_trimmed("promptProgram.content", &self.content)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPolicySpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_targets: Vec<ModeId>,
}

impl TransitionPolicySpec {
    pub fn validate(&self) -> Result<()> {
        let values = self
            .allowed_targets
            .iter()
            .map(|mode_id| mode_id.as_str().to_string())
            .collect::<Vec<_>>();
        normalize_non_empty_unique_string_list(&values, "transitionPolicy.allowedTargets")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModeArtifactDef {
    pub artifact_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_template: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_headings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actionable_sections: Vec<String>,
}

impl ModeArtifactDef {
    pub fn validate(&self) -> Result<()> {
        validate_non_empty_trimmed("mode.artifact.artifactType", &self.artifact_type)?;
        if let Some(template) = &self.file_template {
            validate_non_empty_trimmed("mode.artifact.fileTemplate", template)?;
        }
        if let Some(template) = &self.schema_template {
            validate_non_empty_trimmed("mode.artifact.schemaTemplate", template)?;
        }
        normalize_non_empty_unique_string_list(
            &self.required_headings,
            "mode.artifact.requiredHeadings",
        )?;
        normalize_non_empty_unique_string_list(
            &self.actionable_sections,
            "mode.artifact.actionableSections",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModeExitGateDef {
    pub review_passes: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_checklist: Vec<String>,
}

impl ModeExitGateDef {
    pub fn validate(&self) -> Result<()> {
        if self.review_passes == 0 {
            return Err(AstrError::Validation(
                "mode.exitGate.reviewPasses 必须大于 0".to_string(),
            ));
        }
        normalize_non_empty_unique_string_list(
            &self.review_checklist,
            "mode.exitGate.reviewChecklist",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModePromptHooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reentry_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts_template: Option<String>,
}

impl ModePromptHooks {
    pub fn validate(&self) -> Result<()> {
        let mut has_any = false;
        for (field, value) in [
            (
                "mode.promptHooks.reentryPrompt",
                self.reentry_prompt.as_ref(),
            ),
            (
                "mode.promptHooks.initialTemplate",
                self.initial_template.as_ref(),
            ),
            ("mode.promptHooks.exitPrompt", self.exit_prompt.as_ref()),
            (
                "mode.promptHooks.factsTemplate",
                self.facts_template.as_ref(),
            ),
        ] {
            if let Some(value) = value {
                validate_non_empty_trimmed(field, value)?;
                has_any = true;
            }
        }
        if !has_any {
            return Err(AstrError::Validation(
                "mode.promptHooks 至少需要一个非空模板".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompiledModeContracts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ModeArtifactDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_gate: Option<ModeExitGateDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hooks: Option<ModePromptHooks>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoundModeToolContractSnapshot {
    pub mode_id: ModeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ModeArtifactDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_gate: Option<ModeExitGateDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceModeSpec {
    pub id: ModeId,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub action_policies: ActionPolicies,
    #[serde(default)]
    pub child_policy: ChildPolicySpec,
    #[serde(default)]
    pub execution_policy: ModeExecutionPolicySpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_program: Vec<PromptProgramEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ModeArtifactDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_gate: Option<ModeExitGateDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hooks: Option<ModePromptHooks>,
    #[serde(default)]
    pub transition_policy: TransitionPolicySpec,
}

impl GovernanceModeSpec {
    pub fn validate(&self) -> Result<()> {
        validate_non_empty_trimmed("mode.id", self.id.as_str())?;
        validate_non_empty_trimmed("mode.name", &self.name)?;
        validate_non_empty_trimmed("mode.description", &self.description)?;
        self.child_policy.validate()?;
        if let Some(artifact) = &self.artifact {
            artifact.validate()?;
        }
        if let Some(exit_gate) = &self.exit_gate {
            exit_gate.validate()?;
        }
        if let Some(prompt_hooks) = &self.prompt_hooks {
            prompt_hooks.validate()?;
        }
        self.transition_policy.validate()?;
        for entry in &self.prompt_program {
            entry.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedChildPolicy {
    pub mode_id: ModeId,
    #[serde(default = "default_true")]
    pub allow_delegation: bool,
    #[serde(default = "default_true")]
    pub allow_recursive_delegation: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_profile_ids: Vec<String>,
    #[serde(default)]
    pub restricted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse_scope_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTurnEnvelope {
    pub mode_id: ModeId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_declarations: Vec<PromptDeclaration>,
    #[serde(default)]
    pub mode_contracts: CompiledModeContracts,
    #[serde(default)]
    pub child_policy: ResolvedChildPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

impl ResolvedTurnEnvelope {
    pub fn bound_tool_contract_snapshot(&self) -> BoundModeToolContractSnapshot {
        BoundModeToolContractSnapshot {
            mode_id: self.mode_id.clone(),
            artifact: self.mode_contracts.artifact.clone(),
            exit_gate: self.mode_contracts.exit_gate.clone(),
        }
    }
}

fn validate_non_empty_trimmed(field: &str, value: impl AsRef<str>) -> Result<()> {
    if value.as_ref().trim().is_empty() {
        return Err(AstrError::Validation(format!("{field} 不能为空")));
    }
    Ok(())
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{
        ActionPolicies, BUILTIN_MODE_CODE_ID, BoundModeToolContractSnapshot, CompiledModeContracts,
        GovernanceModeSpec, ModeArtifactDef, ModeExitGateDef, ModeId, ModePromptHooks,
        PromptProgramEntry, ResolvedTurnEnvelope,
    };

    #[test]
    fn mode_id_defaults_to_builtin_code() {
        assert_eq!(ModeId::default().as_str(), BUILTIN_MODE_CODE_ID);
    }

    #[test]
    fn governance_mode_spec_round_trips_and_validates() {
        let spec = GovernanceModeSpec {
            id: ModeId::plan(),
            name: "Plan".to_string(),
            description: "只读规划".to_string(),
            action_policies: ActionPolicies::default(),
            child_policy: Default::default(),
            execution_policy: Default::default(),
            prompt_program: vec![PromptProgramEntry {
                block_id: "mode.plan".to_string(),
                title: "Plan".to_string(),
                content: "plan first".to_string(),
                priority_hint: Some(600),
            }],
            artifact: Some(ModeArtifactDef {
                artifact_type: "canonical-plan".to_string(),
                file_template: Some("# Plan".to_string()),
                schema_template: Some("markdown-plan-v1".to_string()),
                required_headings: vec!["Context".to_string(), "Implementation Steps".to_string()],
                actionable_sections: vec!["Implementation Steps".to_string()],
            }),
            exit_gate: Some(ModeExitGateDef {
                review_passes: 1,
                review_checklist: vec!["验证实现步骤".to_string()],
            }),
            prompt_hooks: Some(ModePromptHooks {
                reentry_prompt: Some("read the plan first".to_string()),
                initial_template: Some("## Implementation Steps".to_string()),
                exit_prompt: Some("use approved plan".to_string()),
                facts_template: Some("targetPlanPath={{target_plan_path}}".to_string()),
            }),
            transition_policy: Default::default(),
        };

        spec.validate().expect("spec should be valid");
        let encoded = serde_json::to_string(&spec).expect("spec should serialize");
        let decoded: GovernanceModeSpec =
            serde_json::from_str(&encoded).expect("spec should deserialize");
        assert_eq!(decoded.id, ModeId::plan());
    }

    #[test]
    fn mode_artifact_def_rejects_blank_artifact_type() {
        let error = ModeArtifactDef {
            artifact_type: "  ".to_string(),
            ..ModeArtifactDef::default()
        }
        .validate()
        .expect_err("blank artifact type should fail");
        assert!(error.to_string().contains("artifactType"));
    }

    #[test]
    fn mode_exit_gate_def_rejects_zero_review_passes() {
        let error = ModeExitGateDef {
            review_passes: 0,
            review_checklist: vec!["检查计划".to_string()],
        }
        .validate()
        .expect_err("zero review passes should fail");
        assert!(error.to_string().contains("reviewPasses"));
    }

    #[test]
    fn mode_prompt_hooks_require_at_least_one_non_empty_template() {
        let error = ModePromptHooks::default()
            .validate()
            .expect_err("empty hooks should fail");
        assert!(error.to_string().contains("至少需要一个"));
    }

    #[test]
    fn resolved_turn_envelope_projects_bound_tool_contract_snapshot() {
        let envelope = ResolvedTurnEnvelope {
            mode_id: ModeId::plan(),
            prompt_declarations: Vec::new(),
            mode_contracts: CompiledModeContracts {
                artifact: Some(ModeArtifactDef {
                    artifact_type: "canonical-plan".to_string(),
                    file_template: None,
                    schema_template: None,
                    required_headings: vec!["Implementation Steps".to_string()],
                    actionable_sections: vec!["Implementation Steps".to_string()],
                }),
                exit_gate: Some(ModeExitGateDef {
                    review_passes: 1,
                    review_checklist: vec!["检查验证步骤".to_string()],
                }),
                prompt_hooks: None,
            },
            child_policy: Default::default(),
            fork_mode: None,
            diagnostics: Vec::new(),
        };

        assert_eq!(
            envelope.bound_tool_contract_snapshot(),
            BoundModeToolContractSnapshot {
                mode_id: ModeId::plan(),
                artifact: envelope.mode_contracts.artifact.clone(),
                exit_gate: envelope.mode_contracts.exit_gate.clone(),
            }
        );
    }
}
