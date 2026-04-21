//! 治理模式注册目录。
//!
//! `ModeCatalog` 管理所有可用的治理模式（内置 + 插件扩展），提供：
//! - 按 ModeId 查找 mode spec
//! - 列出所有可用 mode 的摘要（供 API 返回）
//! - 热替换插件 mode（bootstrap / reload 时调用 `replace_plugin_modes`）
//!
//! 内置三种 mode：
//! - **Code**：默认执行模式，保留完整能力面与委派能力
//! - **Plan**：规划模式，只暴露只读工具，禁止委派
//! - **Review**：审查模式，严格只读，禁止委派，收紧步数（未完成）

use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use astrcode_core::{
    ActionPolicies, ActionPolicyEffect, ActionPolicyRule, CapabilitySelector, ChildPolicySpec,
    GovernanceModeSpec, ModeArtifactDef, ModeExecutionPolicySpec, ModeExitGateDef, ModeId,
    ModePromptHooks, PromptProgramEntry, Result, SubmitBusyPolicy, TransitionPolicySpec,
};

use super::builtin_prompts::{
    code_mode_prompt, plan_mode_exit_prompt, plan_mode_prompt, plan_mode_reentry_prompt,
    plan_template_prompt, review_mode_prompt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeSummary {
    pub id: ModeId,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ModeCatalogEntry {
    pub spec: GovernanceModeSpec,
    pub builtin: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ModeCatalogSnapshot {
    pub entries: BTreeMap<String, ModeCatalogEntry>,
}

impl ModeCatalogSnapshot {
    pub fn get(&self, mode_id: &ModeId) -> Option<&ModeCatalogEntry> {
        self.entries.get(mode_id.as_str())
    }

    pub fn list(&self) -> Vec<ModeSummary> {
        self.entries
            .values()
            .map(|entry| ModeSummary {
                id: entry.spec.id.clone(),
                name: entry.spec.name.clone(),
                description: entry.spec.description.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ModeCatalog {
    snapshot: Arc<RwLock<ModeCatalogSnapshot>>,
}

impl ModeCatalog {
    pub fn new(
        builtin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
        plugin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
    ) -> Result<Self> {
        let snapshot = build_snapshot(builtin_modes, plugin_modes)?;
        Ok(Self {
            snapshot: Arc::new(RwLock::new(snapshot)),
        })
    }

    pub fn snapshot(&self) -> ModeCatalogSnapshot {
        self.snapshot
            .read()
            .expect("mode catalog lock poisoned")
            .clone()
    }

    pub fn list(&self) -> Vec<ModeSummary> {
        self.snapshot().list()
    }

    pub fn get(&self, mode_id: &ModeId) -> Option<GovernanceModeSpec> {
        self.snapshot().get(mode_id).map(|entry| entry.spec.clone())
    }

    pub fn replace_plugin_modes(
        &self,
        plugin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
    ) -> Result<()> {
        let snapshot = self.preview_plugin_modes(plugin_modes)?;
        self.replace_snapshot(snapshot);
        Ok(())
    }

    pub fn preview_plugin_modes(
        &self,
        plugin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
    ) -> Result<ModeCatalogSnapshot> {
        let current = self.snapshot();
        let builtin_modes = current
            .entries
            .values()
            .filter(|entry| entry.builtin)
            .map(|entry| entry.spec.clone())
            .collect::<Vec<_>>();
        build_snapshot(builtin_modes, plugin_modes)
    }

    pub fn replace_snapshot(&self, snapshot: ModeCatalogSnapshot) {
        *self.snapshot.write().expect("mode catalog lock poisoned") = snapshot;
    }
}

pub type BuiltinModeCatalog = ModeCatalog;

pub fn builtin_mode_catalog() -> Result<ModeCatalog> {
    ModeCatalog::new(builtin_mode_specs(), Vec::new())
}

fn build_snapshot(
    builtin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
    plugin_modes: impl IntoIterator<Item = GovernanceModeSpec>,
) -> Result<ModeCatalogSnapshot> {
    let mut entries = BTreeMap::new();
    for (builtin, spec) in builtin_modes
        .into_iter()
        .map(|spec| (true, spec))
        .chain(plugin_modes.into_iter().map(|spec| (false, spec)))
    {
        spec.validate()?;
        let mode_id = spec.id.as_str().to_string();
        if entries.contains_key(&mode_id) {
            return Err(astrcode_core::AstrError::Validation(format!(
                "duplicate mode id '{}'",
                mode_id
            )));
        }
        entries.insert(mode_id, ModeCatalogEntry { spec, builtin });
    }
    Ok(ModeCatalogSnapshot { entries })
}

fn plan_artifact_schema_template() -> String {
    [
        "Session plan markdown schema:",
        "- Context",
        "- Goal",
        "- Scope",
        "- Non-Goals",
        "- Existing Code To Reuse",
        "- Implementation Steps",
        "- Verification",
        "- Open Questions",
    ]
    .join("\n")
}

fn plan_facts_template() -> String {
    [
        "Session plan facts:",
        "- targetPlanPath: {{targetPlanPath}}",
        "- targetPlanExists: {{targetPlanExists}}",
        "- targetPlanSlug: {{targetPlanSlug}}",
        "- activePlan: {{activePlanSummary}}",
        "",
        "Use `upsertSessionPlan` as the only canonical write path for the session plan artifact.",
    ]
    .join("\n")
}

fn builtin_mode_specs() -> Vec<GovernanceModeSpec> {
    let transitions = TransitionPolicySpec {
        allowed_targets: vec![ModeId::code(), ModeId::plan(), ModeId::review()],
    };

    vec![
        GovernanceModeSpec {
            id: ModeId::code(),
            name: "Code".to_string(),
            description: "默认执行模式，保留完整能力面与委派能力。".to_string(),
            capability_selector: CapabilitySelector::AllTools,
            action_policies: ActionPolicies::default(),
            child_policy: ChildPolicySpec {
                allow_delegation: true,
                allow_recursive_delegation: true,
                default_mode_id: Some(ModeId::code()),
                ..ChildPolicySpec::default()
            },
            execution_policy: ModeExecutionPolicySpec {
                submit_busy_policy: Some(SubmitBusyPolicy::BranchOnBusy),
                ..ModeExecutionPolicySpec::default()
            },
            prompt_program: vec![PromptProgramEntry {
                block_id: "mode.code".to_string(),
                title: "Execution Mode".to_string(),
                content: code_mode_prompt().to_string(),
                priority_hint: Some(600),
            }],
            artifact: None,
            exit_gate: None,
            prompt_hooks: None,
            transition_policy: transitions.clone(),
        },
        GovernanceModeSpec {
            id: ModeId::plan(),
            name: "Plan".to_string(),
            description: "规划模式，只暴露只读工具并禁止继续委派。".to_string(),
            capability_selector: CapabilitySelector::Union(vec![
                CapabilitySelector::Difference {
                    base: Box::new(CapabilitySelector::AllTools),
                    subtract: Box::new(CapabilitySelector::Union(vec![
                        CapabilitySelector::SideEffect(astrcode_core::SideEffect::Local),
                        CapabilitySelector::SideEffect(astrcode_core::SideEffect::Workspace),
                        CapabilitySelector::SideEffect(astrcode_core::SideEffect::External),
                        CapabilitySelector::Tag("agent".to_string()),
                    ])),
                },
                CapabilitySelector::Name("exitPlanMode".to_string()),
                CapabilitySelector::Name("upsertSessionPlan".to_string()),
            ]),
            action_policies: ActionPolicies {
                default_effect: ActionPolicyEffect::Allow,
                rules: vec![ActionPolicyRule {
                    selector: CapabilitySelector::Tag("agent".to_string()),
                    effect: ActionPolicyEffect::Deny,
                }],
            },
            child_policy: ChildPolicySpec {
                allow_delegation: false,
                allow_recursive_delegation: false,
                default_mode_id: Some(ModeId::code()),
                restricted: true,
                reuse_scope_summary: Some(
                    "当前 mode 不允许继续派生 child branch；先完成规划，再由后续执行 turn \
                     决定是否委派。"
                        .to_string(),
                ),
                ..ChildPolicySpec::default()
            },
            execution_policy: ModeExecutionPolicySpec {
                submit_busy_policy: Some(SubmitBusyPolicy::RejectOnBusy),
                ..ModeExecutionPolicySpec::default()
            },
            prompt_program: vec![PromptProgramEntry {
                block_id: "mode.plan".to_string(),
                title: "Planning Mode".to_string(),
                content: plan_mode_prompt().to_string(),
                priority_hint: Some(600),
            }],
            artifact: Some(ModeArtifactDef {
                artifact_type: "canonical-plan".to_string(),
                file_template: Some(plan_template_prompt().to_string()),
                schema_template: Some(plan_artifact_schema_template()),
                required_headings: vec![
                    "Context".to_string(),
                    "Goal".to_string(),
                    "Scope".to_string(),
                    "Non-Goals".to_string(),
                    "Existing Code To Reuse".to_string(),
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                    "Open Questions".to_string(),
                ],
                actionable_sections: vec![
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                    "Open Questions".to_string(),
                ],
            }),
            exit_gate: Some(ModeExitGateDef {
                review_passes: 1,
                review_checklist: vec![
                    "检查计划中的假设是否成立".to_string(),
                    "检查是否遗漏边界情况或受影响文件".to_string(),
                    "检查验证步骤是否足够具体".to_string(),
                    "确认计划已经可执行".to_string(),
                ],
            }),
            prompt_hooks: Some(ModePromptHooks {
                reentry_prompt: Some(plan_mode_reentry_prompt().to_string()),
                initial_template: Some(plan_template_prompt().to_string()),
                exit_prompt: Some(plan_mode_exit_prompt().to_string()),
                facts_template: Some(plan_facts_template()),
            }),
            transition_policy: transitions.clone(),
        },
        GovernanceModeSpec {
            id: ModeId::review(),
            name: "Review".to_string(),
            description: "审查模式，只保留严格只读工具并收紧步数。".to_string(),
            capability_selector: CapabilitySelector::Intersection(vec![
                CapabilitySelector::AllTools,
                CapabilitySelector::SideEffect(astrcode_core::SideEffect::None),
                CapabilitySelector::Difference {
                    base: Box::new(CapabilitySelector::AllTools),
                    subtract: Box::new(CapabilitySelector::Tag("agent".to_string())),
                },
            ]),
            action_policies: ActionPolicies {
                default_effect: ActionPolicyEffect::Allow,
                rules: vec![ActionPolicyRule {
                    selector: CapabilitySelector::Tag("agent".to_string()),
                    effect: ActionPolicyEffect::Deny,
                }],
            },
            child_policy: ChildPolicySpec {
                allow_delegation: false,
                allow_recursive_delegation: false,
                default_mode_id: Some(ModeId::review()),
                restricted: true,
                reuse_scope_summary: Some(
                    "当前 mode 仅允许本地审查，不允许在同一 turn 内扩张成新的执行分支。"
                        .to_string(),
                ),
                ..ChildPolicySpec::default()
            },
            execution_policy: ModeExecutionPolicySpec {
                submit_busy_policy: Some(SubmitBusyPolicy::RejectOnBusy),
                ..ModeExecutionPolicySpec::default()
            },
            prompt_program: vec![PromptProgramEntry {
                block_id: "mode.review".to_string(),
                title: "Review Mode".to_string(),
                content: review_mode_prompt().to_string(),
                priority_hint: Some(600),
            }],
            artifact: None,
            exit_gate: None,
            prompt_hooks: None,
            transition_policy: transitions,
        },
    ]
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilitySelector, GovernanceModeSpec, ModeId, Result};

    use super::{ModeCatalog, builtin_mode_catalog, builtin_mode_specs};

    #[test]
    fn builtin_catalog_contains_three_builtin_modes() -> Result<()> {
        let catalog = builtin_mode_catalog()?;
        let ids = catalog
            .list()
            .into_iter()
            .map(|summary| summary.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![ModeId::code(), ModeId::plan(), ModeId::review()]);
        Ok(())
    }

    #[test]
    fn builtin_review_mode_uses_read_only_selector_shape() -> Result<()> {
        let catalog = builtin_mode_catalog()?;
        let review = catalog
            .get(&ModeId::review())
            .expect("review mode should exist");
        assert!(matches!(
            review.capability_selector,
            CapabilitySelector::Intersection(_)
        ));
        Ok(())
    }

    #[test]
    fn builtin_plan_mode_declares_mode_contract_fields() -> Result<()> {
        let catalog = builtin_mode_catalog()?;
        let plan = catalog
            .get(&ModeId::plan())
            .expect("plan mode should exist");

        assert_eq!(
            plan.artifact
                .as_ref()
                .map(|value| value.artifact_type.as_str()),
            Some("canonical-plan")
        );
        assert_eq!(
            plan.exit_gate.as_ref().map(|value| value.review_passes),
            Some(1)
        );
        assert!(
            plan.prompt_hooks
                .as_ref()
                .and_then(|value| value.reentry_prompt.as_ref())
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn plugin_mode_cannot_shadow_builtin_mode_id() {
        let error = ModeCatalog::new(
            builtin_mode_specs(),
            vec![GovernanceModeSpec {
                id: ModeId::plan(),
                name: "Plan Override".to_string(),
                description: "invalid".to_string(),
                capability_selector: CapabilitySelector::AllTools,
                action_policies: Default::default(),
                child_policy: Default::default(),
                execution_policy: Default::default(),
                prompt_program: Vec::new(),
                artifact: None,
                exit_gate: None,
                prompt_hooks: None,
                transition_policy: Default::default(),
            }],
        )
        .expect_err("duplicate builtin mode id should fail");

        assert!(error.to_string().contains("duplicate mode id"));
    }

    #[test]
    fn duplicate_plugin_mode_ids_are_rejected() {
        let plugin_mode = GovernanceModeSpec {
            id: ModeId::from("plugin.plan-lite"),
            name: "Plan Lite".to_string(),
            description: "invalid".to_string(),
            capability_selector: CapabilitySelector::AllTools,
            action_policies: Default::default(),
            child_policy: Default::default(),
            execution_policy: Default::default(),
            prompt_program: Vec::new(),
            artifact: None,
            exit_gate: None,
            prompt_hooks: None,
            transition_policy: Default::default(),
        };
        let error = ModeCatalog::new(
            Vec::<GovernanceModeSpec>::new(),
            vec![plugin_mode.clone(), plugin_mode],
        )
        .expect_err("duplicate plugin ids should fail");

        assert!(error.to_string().contains("duplicate mode id"));
    }

    #[test]
    fn preview_plugin_modes_does_not_mutate_catalog_until_committed() -> Result<()> {
        let catalog = builtin_mode_catalog()?;
        let preview = catalog.preview_plugin_modes(vec![GovernanceModeSpec {
            id: ModeId::from("plugin.plan-lite"),
            name: "Plan Lite".to_string(),
            description: "valid".to_string(),
            capability_selector: CapabilitySelector::AllTools,
            action_policies: Default::default(),
            child_policy: Default::default(),
            execution_policy: Default::default(),
            prompt_program: Vec::new(),
            artifact: None,
            exit_gate: None,
            prompt_hooks: None,
            transition_policy: Default::default(),
        }])?;

        assert!(catalog.get(&ModeId::from("plugin.plan-lite")).is_none());

        catalog.replace_snapshot(preview);

        assert!(catalog.get(&ModeId::from("plugin.plan-lite")).is_some());
        Ok(())
    }
}
