//! 治理模式编译器。
//!
//! 将声明式的 `GovernanceModeSpec` 编译为运行时可消费的 `CompiledModeEnvelope`：
//! - 通过 `CapabilitySelector` 从全量 capability 中筛选出允许的工具名列表
//! - 递归处理组合选择器（Union / Intersection / Difference）
//! - 为子代理额外计算继承后的工具白名单（parent ∩ mode ∩ grant）
//! - 生成 mode prompt declarations 和子代理策略

use std::collections::BTreeSet;

use astrcode_core::{
    AstrError, CapabilitySelector, CapabilitySpec, GovernanceModeSpec, PromptDeclaration,
    PromptDeclarationKind, PromptDeclarationRenderTarget, PromptDeclarationSource,
    ResolvedTurnEnvelope, Result, SpawnCapabilityGrant, SystemPromptLayer,
};
use astrcode_kernel::CapabilityRouter;

#[derive(Clone)]
pub struct CompiledModeEnvelope {
    pub spec: GovernanceModeSpec,
    pub envelope: ResolvedTurnEnvelope,
    pub capability_router: Option<CapabilityRouter>,
}

pub fn compile_capability_selector(
    capability_specs: &[CapabilitySpec],
    selector: &CapabilitySelector,
) -> Result<Vec<String>> {
    let selected = evaluate_selector(capability_specs, selector)?;
    Ok(selected.into_iter().collect())
}

pub fn compile_mode_envelope(
    base_router: &CapabilityRouter,
    spec: &GovernanceModeSpec,
    extra_prompt_declarations: Vec<PromptDeclaration>,
) -> Result<CompiledModeEnvelope> {
    let allowed_tools =
        compile_capability_selector(&base_router.capability_specs(), &spec.capability_selector)?;
    let child_allowed_tools =
        child_allowed_tools(&base_router.capability_specs(), spec, &allowed_tools, None)?;
    let prompt_declarations = mode_prompt_declarations(spec, extra_prompt_declarations);
    let envelope = ResolvedTurnEnvelope {
        mode_id: spec.id.clone(),
        allowed_tools: allowed_tools.clone(),
        prompt_declarations: prompt_declarations.clone(),
        action_policies: spec.action_policies.clone(),
        child_policy: astrcode_core::ResolvedChildPolicy {
            mode_id: spec
                .child_policy
                .default_mode_id
                .clone()
                .unwrap_or_default(),
            allow_delegation: spec.child_policy.allow_delegation,
            allow_recursive_delegation: spec.child_policy.allow_recursive_delegation,
            allowed_profile_ids: spec.child_policy.allowed_profile_ids.clone(),
            allowed_tools: child_allowed_tools,
            restricted: spec.child_policy.restricted,
            fork_mode: spec
                .child_policy
                .fork_mode
                .clone()
                .or(spec.execution_policy.fork_mode.clone()),
            reuse_scope_summary: spec.child_policy.reuse_scope_summary.clone(),
        },
        submit_busy_policy: spec
            .execution_policy
            .submit_busy_policy
            .unwrap_or(astrcode_core::SubmitBusyPolicy::BranchOnBusy),
        fork_mode: spec.execution_policy.fork_mode.clone(),
        diagnostics: if allowed_tools.is_empty() {
            vec![format!(
                "mode '{}' compiled to an empty tool surface",
                spec.id
            )]
        } else {
            Vec::new()
        },
    };
    let capability_router = subset_router(base_router, &allowed_tools)?;
    Ok(CompiledModeEnvelope {
        spec: spec.clone(),
        envelope,
        capability_router,
    })
}

pub fn compile_mode_envelope_for_child(
    base_router: &CapabilityRouter,
    spec: &GovernanceModeSpec,
    parent_allowed_tools: &[String],
    capability_grant: Option<&SpawnCapabilityGrant>,
) -> Result<CompiledModeEnvelope> {
    let mode_allowed_tools =
        compile_capability_selector(&base_router.capability_specs(), &spec.capability_selector)?;
    // 子代理工具 = parent ∩ mode；parent 为空时直接取 mode 全量
    let effective_parent_allowed_tools = if parent_allowed_tools.is_empty() {
        mode_allowed_tools
    } else {
        parent_allowed_tools
            .iter()
            .filter(|tool| {
                mode_allowed_tools
                    .iter()
                    .any(|candidate| candidate == *tool)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let child_tools = child_allowed_tools(
        &base_router.capability_specs(),
        spec,
        &effective_parent_allowed_tools,
        capability_grant,
    )?;
    let prompt_declarations = mode_prompt_declarations(spec, Vec::new());
    let envelope = ResolvedTurnEnvelope {
        mode_id: spec.id.clone(),
        allowed_tools: child_tools.clone(),
        prompt_declarations: prompt_declarations.clone(),
        action_policies: spec.action_policies.clone(),
        child_policy: astrcode_core::ResolvedChildPolicy {
            mode_id: spec
                .child_policy
                .default_mode_id
                .clone()
                .unwrap_or_default(),
            allow_delegation: spec.child_policy.allow_delegation,
            allow_recursive_delegation: spec.child_policy.allow_recursive_delegation,
            allowed_profile_ids: spec.child_policy.allowed_profile_ids.clone(),
            allowed_tools: child_tools.clone(),
            restricted: spec.child_policy.restricted || capability_grant.is_some(),
            fork_mode: spec
                .child_policy
                .fork_mode
                .clone()
                .or(spec.execution_policy.fork_mode.clone()),
            reuse_scope_summary: spec.child_policy.reuse_scope_summary.clone(),
        },
        submit_busy_policy: spec
            .execution_policy
            .submit_busy_policy
            .unwrap_or(astrcode_core::SubmitBusyPolicy::BranchOnBusy),
        fork_mode: spec
            .execution_policy
            .fork_mode
            .clone()
            .or(spec.child_policy.fork_mode.clone()),
        diagnostics: if child_tools.is_empty() {
            vec![format!(
                "child mode '{}' compiled to an empty inheritable tool surface",
                spec.id
            )]
        } else {
            Vec::new()
        },
    };
    let capability_router = subset_router(base_router, &child_tools)?;
    Ok(CompiledModeEnvelope {
        spec: spec.clone(),
        envelope,
        capability_router,
    })
}

fn evaluate_selector(
    capability_specs: &[CapabilitySpec],
    selector: &CapabilitySelector,
) -> Result<BTreeSet<String>> {
    let tools = capability_specs
        .iter()
        .filter(|spec| spec.kind.is_tool())
        .collect::<Vec<_>>();
    Ok(match selector {
        CapabilitySelector::AllTools => tools
            .into_iter()
            .map(|spec| spec.name.to_string())
            .collect(),
        CapabilitySelector::Name(name) => tools
            .into_iter()
            .filter(|spec| spec.name.as_str() == name.as_str())
            .map(|spec| spec.name.to_string())
            .collect(),
        CapabilitySelector::Kind(kind) => tools
            .into_iter()
            .filter(|spec| spec.kind == *kind)
            .map(|spec| spec.name.to_string())
            .collect(),
        CapabilitySelector::SideEffect(side_effect) => tools
            .into_iter()
            .filter(|spec| spec.side_effect == *side_effect)
            .map(|spec| spec.name.to_string())
            .collect(),
        CapabilitySelector::Tag(tag) => tools
            .into_iter()
            .filter(|spec| spec.tags.iter().any(|candidate| candidate == tag))
            .map(|spec| spec.name.to_string())
            .collect(),
        CapabilitySelector::Union(selectors) => {
            let mut combined = BTreeSet::new();
            for selector in selectors {
                combined.extend(evaluate_selector(capability_specs, selector)?);
            }
            combined
        },
        CapabilitySelector::Intersection(selectors) => {
            let mut iter = selectors.iter();
            let Some(first) = iter.next() else {
                return Ok(BTreeSet::new());
            };
            let mut combined = evaluate_selector(capability_specs, first)?;
            for selector in iter {
                let next = evaluate_selector(capability_specs, selector)?;
                combined = combined.intersection(&next).cloned().collect();
            }
            combined
        },
        CapabilitySelector::Difference { base, subtract } => {
            let base = evaluate_selector(capability_specs, base)?;
            let subtract = evaluate_selector(capability_specs, subtract)?;
            base.difference(&subtract).cloned().collect()
        },
    })
}

fn child_allowed_tools(
    capability_specs: &[CapabilitySpec],
    spec: &GovernanceModeSpec,
    parent_allowed_tools: &[String],
    capability_grant: Option<&SpawnCapabilityGrant>,
) -> Result<Vec<String>> {
    if !spec.child_policy.allow_delegation {
        return Ok(Vec::new());
    }
    let mut allowed = if let Some(selector) = &spec.child_policy.capability_selector {
        let selected = evaluate_selector(capability_specs, selector)?;
        parent_allowed_tools
            .iter()
            .filter(|tool| selected.contains(tool.as_str()))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        parent_allowed_tools.to_vec()
    };
    if let Some(grant) = capability_grant {
        grant.validate()?;
        let requested = grant.normalized_allowed_tools()?;
        allowed.retain(|tool| requested.iter().any(|candidate| candidate == tool));
    }
    Ok(allowed)
}

fn mode_prompt_declarations(
    spec: &GovernanceModeSpec,
    extra_prompt_declarations: Vec<PromptDeclaration>,
) -> Vec<PromptDeclaration> {
    let mut declarations = spec
        .prompt_program
        .iter()
        .map(|entry| PromptDeclaration {
            block_id: entry.block_id.clone(),
            title: entry.title.clone(),
            content: entry.content.clone(),
            render_target: PromptDeclarationRenderTarget::System,
            layer: SystemPromptLayer::Dynamic,
            kind: PromptDeclarationKind::ExtensionInstruction,
            priority_hint: entry.priority_hint,
            always_include: true,
            source: PromptDeclarationSource::Builtin,
            capability_name: None,
            origin: Some(format!("mode:{}", spec.id)),
        })
        .collect::<Vec<_>>();
    declarations.extend(extra_prompt_declarations);
    declarations
}

fn subset_router(
    base_router: &CapabilityRouter,
    allowed_tools: &[String],
) -> Result<Option<CapabilityRouter>> {
    let all_tools = base_router.tool_names();
    let allowed_set = allowed_tools
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let all_set = all_tools
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if allowed_set == all_set {
        return Ok(None);
    }
    if allowed_tools.is_empty() {
        return Ok(Some(CapabilityRouter::empty()));
    }
    Ok(Some(
        base_router
            .subset_for_tools_checked(allowed_tools)
            .map_err(|error| AstrError::Validation(error.to_string()))?,
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, CapabilityKind,
        CapabilitySpec, SideEffect,
    };
    use astrcode_kernel::CapabilityRouter;
    use async_trait::async_trait;
    use serde_json::Value;

    use super::compile_capability_selector;
    use crate::mode::builtin_mode_catalog;

    #[derive(Debug)]
    struct FakeCapabilityInvoker {
        spec: CapabilitySpec,
    }

    impl FakeCapabilityInvoker {
        fn new(spec: CapabilitySpec) -> Self {
            Self { spec }
        }
    }

    #[async_trait]
    impl CapabilityInvoker for FakeCapabilityInvoker {
        fn capability_spec(&self) -> CapabilitySpec {
            self.spec.clone()
        }

        async fn invoke(
            &self,
            _payload: Value,
            _ctx: &CapabilityContext,
        ) -> astrcode_core::Result<CapabilityExecutionResult> {
            Ok(CapabilityExecutionResult::ok(
                self.spec.name.to_string(),
                Value::Null,
            ))
        }
    }

    fn router() -> CapabilityRouter {
        CapabilityRouter::builder()
            .register_invoker(Arc::new(FakeCapabilityInvoker::new(
                CapabilitySpec::builder("readFile", CapabilityKind::Tool)
                    .description("read")
                    .schema(
                        serde_json::json!({"type":"object"}),
                        serde_json::json!({"type":"object"}),
                    )
                    .tags(["filesystem", "read"])
                    .side_effect(SideEffect::None)
                    .build()
                    .expect("readFile should build"),
            )))
            .register_invoker(Arc::new(FakeCapabilityInvoker::new(
                CapabilitySpec::builder("writeFile", CapabilityKind::Tool)
                    .description("write")
                    .schema(
                        serde_json::json!({"type":"object"}),
                        serde_json::json!({"type":"object"}),
                    )
                    .tags(["filesystem", "write"])
                    .side_effect(SideEffect::Workspace)
                    .build()
                    .expect("writeFile should build"),
            )))
            .register_invoker(Arc::new(FakeCapabilityInvoker::new(
                CapabilitySpec::builder("spawn", CapabilityKind::Tool)
                    .description("spawn")
                    .schema(
                        serde_json::json!({"type":"object"}),
                        serde_json::json!({"type":"object"}),
                    )
                    .tags(["agent"])
                    .side_effect(SideEffect::None)
                    .build()
                    .expect("spawn should build"),
            )))
            .build()
            .expect("router should build")
    }

    #[test]
    fn builtin_modes_compile_expected_tool_equivalence() {
        let router = router();
        let catalog = builtin_mode_catalog().expect("builtin catalog should build");

        let code = catalog.get(&astrcode_core::ModeId::code()).unwrap();
        let plan = catalog.get(&astrcode_core::ModeId::plan()).unwrap();
        let review = catalog.get(&astrcode_core::ModeId::review()).unwrap();

        assert_eq!(
            compile_capability_selector(&router.capability_specs(), &code.capability_selector)
                .expect("code selector should compile"),
            vec![
                "readFile".to_string(),
                "spawn".to_string(),
                "writeFile".to_string()
            ]
        );
        assert_eq!(
            compile_capability_selector(&router.capability_specs(), &plan.capability_selector)
                .expect("plan selector should compile"),
            vec!["readFile".to_string()]
        );
        assert_eq!(
            compile_capability_selector(&router.capability_specs(), &review.capability_selector)
                .expect("review selector should compile"),
            vec!["readFile".to_string()]
        );
    }
}
