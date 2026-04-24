//! # 声明式治理模式系统
//!
//! 定义运行时治理模式（Governance Mode）的完整 DSL：
//!
//! - **ModeId**: 模式唯一标识（内置 code / plan / review）
//! - **CapabilitySelector**: 能力选择器 DSL（支持 AllTools / Name / Kind / Tag / Union /
//!   Intersection / Difference）
//! - **ActionPolicies**: 动作策略规则集（Allow / Deny / Ask 三种裁决效果）
//! - **GovernanceModeSpec**: 完整模式定义（能力表面 + 动作策略 + 子策略 + 执行策略 + 提示词程序 +
//!   转换策略）
//! - **ResolvedTurnEnvelope**: 当前命名沿用 envelope，但语义上是治理 compile 阶段产物
//!
//! 模式由声明式配置文件加载，运行时通过 `GovernanceModeSpec::validate()` 校验后，
//! 由治理层先编译为 `ResolvedTurnEnvelope`，再由 application bind 成 turn 可执行治理快照。

use astrcode_core::{
    AstrError, CapabilityKind, ForkMode, Result, SideEffect, normalize_non_empty_unique_string_list,
};
use astrcode_prompt_contract::PromptDeclaration;
use serde::{Deserialize, Serialize};

pub const BUILTIN_MODE_CODE_ID: &str = "code";
pub const BUILTIN_MODE_PLAN_ID: &str = "plan";
pub const BUILTIN_MODE_REVIEW_ID: &str = "review";

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

    pub fn review() -> Self {
        Self(BUILTIN_MODE_REVIEW_ID.to_string())
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

impl From<ModeId> for astrcode_core::mode::ModeId {
    fn from(value: ModeId) -> Self {
        Self::from(value.0)
    }
}

impl From<astrcode_core::mode::ModeId> for ModeId {
    fn from(value: astrcode_core::mode::ModeId) -> Self {
        Self::from(value.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubmitBusyPolicy {
    #[default]
    BranchOnBusy,
    RejectOnBusy,
}

impl From<SubmitBusyPolicy> for astrcode_core::mode::SubmitBusyPolicy {
    fn from(value: SubmitBusyPolicy) -> Self {
        match value {
            SubmitBusyPolicy::BranchOnBusy => Self::BranchOnBusy,
            SubmitBusyPolicy::RejectOnBusy => Self::RejectOnBusy,
        }
    }
}

impl From<astrcode_core::mode::SubmitBusyPolicy> for SubmitBusyPolicy {
    fn from(value: astrcode_core::mode::SubmitBusyPolicy) -> Self {
        match value {
            astrcode_core::mode::SubmitBusyPolicy::BranchOnBusy => Self::BranchOnBusy,
            astrcode_core::mode::SubmitBusyPolicy::RejectOnBusy => Self::RejectOnBusy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilitySelector {
    AllTools,
    Name(String),
    Kind(CapabilityKind),
    SideEffect(SideEffect),
    Tag(String),
    Union(Vec<CapabilitySelector>),
    Intersection(Vec<CapabilitySelector>),
    Difference {
        base: Box<CapabilitySelector>,
        subtract: Box<CapabilitySelector>,
    },
}

impl CapabilitySelector {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::AllTools => Ok(()),
            Self::Name(name) => validate_non_empty_trimmed("capabilitySelector.name", name),
            Self::Kind(_) | Self::SideEffect(_) => Ok(()),
            Self::Tag(tag) => validate_non_empty_trimmed("capabilitySelector.tag", tag),
            Self::Union(selectors) | Self::Intersection(selectors) => {
                if selectors.is_empty() {
                    return Err(AstrError::Validation(
                        "capabilitySelector 组合操作不能为空".to_string(),
                    ));
                }
                for selector in selectors {
                    selector.validate()?;
                }
                Ok(())
            },
            Self::Difference { base, subtract } => {
                base.validate()?;
                subtract.validate()?;
                Ok(())
            },
        }
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
    pub selector: CapabilitySelector,
    pub effect: ActionPolicyEffect,
}

impl ActionPolicyRule {
    pub fn validate(&self) -> Result<()> {
        self.selector.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActionPolicies {
    #[serde(default = "default_action_policy_effect")]
    pub default_effect: ActionPolicyEffect,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ActionPolicyRule>,
}

impl ActionPolicies {
    pub fn validate(&self) -> Result<()> {
        for rule in &self.rules {
            rule.validate()?;
        }
        Ok(())
    }

    pub fn requires_approval(&self) -> bool {
        self.rules
            .iter()
            .any(|rule| matches!(rule.effect, ActionPolicyEffect::Ask))
            || matches!(self.default_effect, ActionPolicyEffect::Ask)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_selector: Option<CapabilitySelector>,
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
        if let Some(selector) = &self.capability_selector {
            selector.validate()?;
        }
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
    pub submit_busy_policy: Option<SubmitBusyPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
}

impl ModeExecutionPolicySpec {
    pub fn validate(&self) -> Result<()> {
        Ok(())
    }
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

impl From<ModeArtifactDef> for astrcode_core::mode::ModeArtifactDef {
    fn from(value: ModeArtifactDef) -> Self {
        Self {
            artifact_type: value.artifact_type,
            file_template: value.file_template,
            schema_template: value.schema_template,
            required_headings: value.required_headings,
            actionable_sections: value.actionable_sections,
        }
    }
}

impl From<astrcode_core::mode::ModeArtifactDef> for ModeArtifactDef {
    fn from(value: astrcode_core::mode::ModeArtifactDef) -> Self {
        Self {
            artifact_type: value.artifact_type,
            file_template: value.file_template,
            schema_template: value.schema_template,
            required_headings: value.required_headings,
            actionable_sections: value.actionable_sections,
        }
    }
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

impl From<ModeExitGateDef> for astrcode_core::mode::ModeExitGateDef {
    fn from(value: ModeExitGateDef) -> Self {
        Self {
            review_passes: value.review_passes,
            review_checklist: value.review_checklist,
        }
    }
}

impl From<astrcode_core::mode::ModeExitGateDef> for ModeExitGateDef {
    fn from(value: astrcode_core::mode::ModeExitGateDef) -> Self {
        Self {
            review_passes: value.review_passes,
            review_checklist: value.review_checklist,
        }
    }
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

impl From<ModePromptHooks> for astrcode_core::mode::ModePromptHooks {
    fn from(value: ModePromptHooks) -> Self {
        Self {
            reentry_prompt: value.reentry_prompt,
            initial_template: value.initial_template,
            exit_prompt: value.exit_prompt,
            facts_template: value.facts_template,
        }
    }
}

impl From<astrcode_core::mode::ModePromptHooks> for ModePromptHooks {
    fn from(value: astrcode_core::mode::ModePromptHooks) -> Self {
        Self {
            reentry_prompt: value.reentry_prompt,
            initial_template: value.initial_template,
            exit_prompt: value.exit_prompt,
            facts_template: value.facts_template,
        }
    }
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

impl From<CompiledModeContracts> for astrcode_core::mode::CompiledModeContracts {
    fn from(value: CompiledModeContracts) -> Self {
        Self {
            artifact: value.artifact.map(Into::into),
            exit_gate: value.exit_gate.map(Into::into),
            prompt_hooks: value.prompt_hooks.map(Into::into),
        }
    }
}

impl From<astrcode_core::mode::CompiledModeContracts> for CompiledModeContracts {
    fn from(value: astrcode_core::mode::CompiledModeContracts) -> Self {
        Self {
            artifact: value.artifact.map(Into::into),
            exit_gate: value.exit_gate.map(Into::into),
            prompt_hooks: value.prompt_hooks.map(Into::into),
        }
    }
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

impl From<BoundModeToolContractSnapshot> for astrcode_core::mode::BoundModeToolContractSnapshot {
    fn from(value: BoundModeToolContractSnapshot) -> Self {
        Self {
            mode_id: value.mode_id.into(),
            artifact: value.artifact.map(Into::into),
            exit_gate: value.exit_gate.map(Into::into),
        }
    }
}

impl From<astrcode_core::mode::BoundModeToolContractSnapshot> for BoundModeToolContractSnapshot {
    fn from(value: astrcode_core::mode::BoundModeToolContractSnapshot) -> Self {
        Self {
            mode_id: value.mode_id.into(),
            artifact: value.artifact.map(Into::into),
            exit_gate: value.exit_gate.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceModeSpec {
    pub id: ModeId,
    pub name: String,
    pub description: String,
    pub capability_selector: CapabilitySelector,
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
        self.capability_selector.validate()?;
        self.action_policies.validate()?;
        self.child_policy.validate()?;
        self.execution_policy.validate()?;
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
    pub action_policies: ActionPolicies,
    #[serde(default)]
    pub child_policy: ResolvedChildPolicy,
    #[serde(default)]
    pub submit_busy_policy: SubmitBusyPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

impl ResolvedTurnEnvelope {
    pub fn approval_mode(&self) -> String {
        if self.action_policies.requires_approval() {
            "required".to_string()
        } else {
            "inherit".to_string()
        }
    }

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
    use astrcode_core::{CapabilityKind, SideEffect};
    use astrcode_prompt_contract::{
        PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
        PromptDeclarationSource, SystemPromptLayer,
    };

    use super::{
        ActionPolicies, BUILTIN_MODE_CODE_ID, BoundModeToolContractSnapshot, CapabilitySelector,
        CompiledModeContracts, GovernanceModeSpec, ModeArtifactDef, ModeExitGateDef, ModeId,
        ModePromptHooks, PromptProgramEntry, ResolvedTurnEnvelope, SubmitBusyPolicy,
    };

    #[test]
    fn mode_id_defaults_to_builtin_code() {
        assert_eq!(ModeId::default().as_str(), BUILTIN_MODE_CODE_ID);
    }

    #[test]
    fn capability_selector_validation_rejects_empty_union() {
        let error = CapabilitySelector::Union(Vec::new())
            .validate()
            .expect_err("empty union should be rejected");
        assert!(error.to_string().contains("不能为空"));
    }

    #[test]
    fn governance_mode_spec_round_trips_and_validates() {
        let spec = GovernanceModeSpec {
            id: ModeId::plan(),
            name: "Plan".to_string(),
            description: "只读规划".to_string(),
            capability_selector: CapabilitySelector::Intersection(vec![
                CapabilitySelector::Kind(CapabilityKind::Tool),
                CapabilitySelector::Difference {
                    base: Box::new(CapabilitySelector::AllTools),
                    subtract: Box::new(CapabilitySelector::SideEffect(SideEffect::Workspace)),
                },
            ]),
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
    fn resolved_turn_envelope_reports_required_approval_mode_when_rule_asks() {
        let envelope = ResolvedTurnEnvelope {
            mode_id: ModeId::review(),
            prompt_declarations: vec![PromptDeclaration {
                block_id: "mode.review".to_string(),
                title: "Review".to_string(),
                content: "review only".to_string(),
                render_target: PromptDeclarationRenderTarget::System,
                layer: SystemPromptLayer::Dynamic,
                kind: PromptDeclarationKind::ExtensionInstruction,
                priority_hint: Some(600),
                always_include: true,
                source: PromptDeclarationSource::Builtin,
                capability_name: None,
                origin: None,
            }],
            mode_contracts: CompiledModeContracts::default(),
            action_policies: ActionPolicies {
                default_effect: crate::ActionPolicyEffect::Ask,
                rules: Vec::new(),
            },
            child_policy: Default::default(),
            submit_busy_policy: SubmitBusyPolicy::RejectOnBusy,
            fork_mode: None,
            diagnostics: Vec::new(),
        };

        assert_eq!(envelope.approval_mode(), "required");
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
            action_policies: ActionPolicies::default(),
            child_policy: Default::default(),
            submit_busy_policy: SubmitBusyPolicy::BranchOnBusy,
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
