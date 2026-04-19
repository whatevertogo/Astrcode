use serde::{Deserialize, Serialize};

use crate::{
    AstrError, CapabilityKind, ContextStrategy, ForkMode, PromptDeclaration, Result, SideEffect,
    normalize_non_empty_unique_string_list,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubmitBusyPolicy {
    #[default]
    BranchOnBusy,
    RejectOnBusy,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_strategy: Option<ContextStrategy>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
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
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_declarations: Vec<PromptDeclaration>,
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
        ActionPolicies, BUILTIN_MODE_CODE_ID, CapabilitySelector, GovernanceModeSpec, ModeId,
        PromptProgramEntry, ResolvedTurnEnvelope, SubmitBusyPolicy,
    };
    use crate::{CapabilityKind, PromptDeclaration, SideEffect, SystemPromptLayer};

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
            transition_policy: Default::default(),
        };

        spec.validate().expect("spec should be valid");
        let encoded = serde_json::to_string(&spec).expect("spec should serialize");
        let decoded: GovernanceModeSpec =
            serde_json::from_str(&encoded).expect("spec should deserialize");
        assert_eq!(decoded.id, ModeId::plan());
    }

    #[test]
    fn resolved_turn_envelope_reports_required_approval_mode_when_rule_asks() {
        let envelope = ResolvedTurnEnvelope {
            mode_id: ModeId::review(),
            allowed_tools: vec!["readFile".to_string()],
            prompt_declarations: vec![PromptDeclaration {
                block_id: "mode.review".to_string(),
                title: "Review".to_string(),
                content: "review only".to_string(),
                render_target: crate::PromptDeclarationRenderTarget::System,
                layer: SystemPromptLayer::Dynamic,
                kind: crate::PromptDeclarationKind::ExtensionInstruction,
                priority_hint: Some(600),
                always_include: true,
                source: crate::PromptDeclarationSource::Builtin,
                capability_name: None,
                origin: None,
            }],
            action_policies: ActionPolicies {
                default_effect: crate::ActionPolicyEffect::Ask,
                rules: Vec::new(),
                context_strategy: None,
            },
            child_policy: Default::default(),
            submit_busy_policy: SubmitBusyPolicy::RejectOnBusy,
            fork_mode: None,
            diagnostics: Vec::new(),
        };

        assert_eq!(envelope.approval_mode(), "required");
    }
}
