//! 内置治理模式定义。
//!
//! 提供 `builtin_mode_specs()` 返回内置 Code / Plan 两种 mode 的 spec，
//! 以及 plan 模板辅助函数。

use astrcode_core::mode::{
    ActionPolicies, ActionPolicyEffect, ActionPolicyRule, ChildPolicySpec, GovernanceModeSpec,
    ModeArtifactDef, ModeExecutionPolicySpec, ModeExitGateDef, ModeId, ModePromptHooks,
    PromptProgramEntry, TransitionPolicySpec,
};

use super::builtin_prompts::{
    code_mode_prompt, plan_mode_exit_prompt, plan_mode_prompt, plan_mode_reentry_prompt,
    plan_template_prompt,
};

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

pub(crate) fn builtin_mode_specs() -> Vec<GovernanceModeSpec> {
    let transitions = TransitionPolicySpec {
        allowed_targets: vec![ModeId::code(), ModeId::plan()],
    };

    vec![
        GovernanceModeSpec {
            id: ModeId::code(),
            name: "Code".to_string(),
            description: "默认执行模式，保留完整能力面与委派能力。".to_string(),
            action_policies: ActionPolicies::default(),
            child_policy: ChildPolicySpec {
                allow_delegation: true,
                allow_recursive_delegation: true,
                default_mode_id: Some(ModeId::code()),
                ..ChildPolicySpec::default()
            },
            execution_policy: ModeExecutionPolicySpec::default(),
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
            action_policies: ActionPolicies {
                default_effect: ActionPolicyEffect::Allow,
                rules: vec![ActionPolicyRule {
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
            execution_policy: ModeExecutionPolicySpec::default(),
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
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        Result,
        mode::{GovernanceModeSpec, ModeId},
    };

    use super::builtin_mode_specs;
    use crate::mode_catalog_service::ServerModeCatalog;

    fn builtin_test_catalog() -> Result<Arc<ServerModeCatalog>> {
        ServerModeCatalog::from_mode_specs(builtin_mode_specs(), Vec::new())
    }

    #[test]
    fn builtin_catalog_contains_two_builtin_modes() -> Result<()> {
        let catalog = builtin_test_catalog()?;
        let ids = catalog
            .list()
            .into_iter()
            .map(|summary| summary.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![ModeId::code(), ModeId::plan()]);
        Ok(())
    }

    #[test]
    fn builtin_plan_mode_declares_mode_contract_fields() -> Result<()> {
        let catalog = builtin_test_catalog()?;
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
        let prompt = &plan.prompt_program[0].content;
        assert!(prompt.contains("Do not repeat the full plan"));
        assert!(prompt.contains("prefer no assistant text at all"));
        assert!(prompt.contains(
            "keep that checkpoint and your internal review reasoning out of user-visible \
             assistant text"
        ));
        assert!(!prompt.contains("summarize the plan plainly"));
        Ok(())
    }

    #[test]
    fn plugin_mode_cannot_shadow_builtin_mode_id() {
        let error = ServerModeCatalog::from_mode_specs(
            builtin_mode_specs(),
            vec![GovernanceModeSpec {
                id: ModeId::plan(),
                name: "Plan Override".to_string(),
                description: "invalid".to_string(),
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
            action_policies: Default::default(),
            child_policy: Default::default(),
            execution_policy: Default::default(),
            prompt_program: Vec::new(),
            artifact: None,
            exit_gate: None,
            prompt_hooks: None,
            transition_policy: Default::default(),
        };
        let error = ServerModeCatalog::from_mode_specs(
            Vec::<GovernanceModeSpec>::new(),
            vec![plugin_mode.clone(), plugin_mode],
        )
        .expect_err("duplicate plugin ids should fail");

        assert!(error.to_string().contains("duplicate mode id"));
    }

    #[test]
    fn preview_plugin_modes_does_not_mutate_catalog_until_committed() -> Result<()> {
        let catalog = builtin_test_catalog()?;
        let preview = catalog.preview_plugin_modes(vec![GovernanceModeSpec {
            id: ModeId::from("plugin.plan-lite"),
            name: "Plan Lite".to_string(),
            description: "valid".to_string(),
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
