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
//! - **Review**：审查模式，严格只读，禁止委派，收紧步数

use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use astrcode_core::{
    ActionPolicies, ActionPolicyEffect, ActionPolicyRule, CapabilitySelector, ChildPolicySpec,
    GovernanceModeSpec, ModeExecutionPolicySpec, ModeId, PromptProgramEntry, Result,
    SubmitBusyPolicy, TransitionPolicySpec,
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
        let current = self.snapshot();
        let builtin_modes = current
            .entries
            .values()
            .filter(|entry| entry.builtin)
            .map(|entry| entry.spec.clone())
            .collect::<Vec<_>>();
        let snapshot = build_snapshot(builtin_modes, plugin_modes)?;
        *self.snapshot.write().expect("mode catalog lock poisoned") = snapshot;
        Ok(())
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
        entries.insert(
            spec.id.as_str().to_string(),
            ModeCatalogEntry { spec, builtin },
        );
    }
    Ok(ModeCatalogSnapshot { entries })
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
                content: "You are in execution mode. Prefer direct progress, make concrete code \
                          changes when needed, and use delegation only when isolation or \
                          parallelism materially helps."
                    .to_string(),
                priority_hint: Some(600),
            }],
            transition_policy: transitions.clone(),
        },
        GovernanceModeSpec {
            id: ModeId::plan(),
            name: "Plan".to_string(),
            description: "规划模式，只暴露只读工具并禁止继续委派。".to_string(),
            capability_selector: CapabilitySelector::Difference {
                base: Box::new(CapabilitySelector::AllTools),
                subtract: Box::new(CapabilitySelector::Union(vec![
                    CapabilitySelector::SideEffect(astrcode_core::SideEffect::Local),
                    CapabilitySelector::SideEffect(astrcode_core::SideEffect::Workspace),
                    CapabilitySelector::SideEffect(astrcode_core::SideEffect::External),
                    CapabilitySelector::Tag("agent".to_string()),
                ])),
            },
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
                content: "You are in planning mode. Focus on analysis, proposals, sequencing, and \
                          constraints. Do not write files or perform side-effecting actions in \
                          this turn."
                    .to_string(),
                priority_hint: Some(600),
            }],
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
                content: "You are in review mode. Prioritize findings, risks, regressions, and \
                          verification gaps. Stay read-only and avoid speculative edits."
                    .to_string(),
                priority_hint: Some(600),
            }],
            transition_policy: transitions,
        },
    ]
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilitySelector, ModeId, Result};

    use super::builtin_mode_catalog;

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
}
