//! 治理模式编译器。
//!
//! 将声明式的 `GovernanceModeSpec` 编译为运行时可消费的 `CompiledModeEnvelope`：
//! - 保留 mode prompt / contracts / child policy 等稳定语义
//! - 生成 mode prompt declarations 和子代理策略

use astrcode_core::Result;
use astrcode_governance_contract::{
    CompiledModeContracts, GovernanceModeSpec, ResolvedChildPolicy, ResolvedTurnEnvelope,
};
use astrcode_prompt_contract::{
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, SystemPromptLayer,
};

#[derive(Clone)]
pub struct CompiledModeEnvelope {
    pub envelope: ResolvedTurnEnvelope,
}

pub fn compile_mode_envelope(
    spec: &GovernanceModeSpec,
    extra_prompt_declarations: Vec<PromptDeclaration>,
) -> Result<CompiledModeEnvelope> {
    let prompt_declarations = mode_prompt_declarations(spec, extra_prompt_declarations);
    let envelope = ResolvedTurnEnvelope {
        mode_id: spec.id.clone(),
        prompt_declarations: prompt_declarations.clone(),
        mode_contracts: compiled_mode_contracts(spec),
        child_policy: ResolvedChildPolicy {
            mode_id: spec
                .child_policy
                .default_mode_id
                .clone()
                .unwrap_or_default(),
            allow_delegation: spec.child_policy.allow_delegation,
            allow_recursive_delegation: spec.child_policy.allow_recursive_delegation,
            allowed_profile_ids: spec.child_policy.allowed_profile_ids.clone(),
            restricted: spec.child_policy.restricted,
            fork_mode: spec
                .child_policy
                .fork_mode
                .clone()
                .or(spec.execution_policy.fork_mode.clone()),
            reuse_scope_summary: spec.child_policy.reuse_scope_summary.clone(),
        },
        fork_mode: spec.execution_policy.fork_mode.clone(),
        diagnostics: Vec::new(),
    };
    Ok(CompiledModeEnvelope { envelope })
}

pub fn compile_mode_envelope_for_child(spec: &GovernanceModeSpec) -> Result<CompiledModeEnvelope> {
    let prompt_declarations = mode_prompt_declarations(spec, Vec::new());
    let envelope = ResolvedTurnEnvelope {
        mode_id: spec.id.clone(),
        prompt_declarations: prompt_declarations.clone(),
        mode_contracts: compiled_mode_contracts(spec),
        child_policy: ResolvedChildPolicy {
            mode_id: spec
                .child_policy
                .default_mode_id
                .clone()
                .unwrap_or_default(),
            allow_delegation: spec.child_policy.allow_delegation,
            allow_recursive_delegation: spec.child_policy.allow_recursive_delegation,
            allowed_profile_ids: spec.child_policy.allowed_profile_ids.clone(),
            restricted: spec.child_policy.restricted,
            fork_mode: spec
                .child_policy
                .fork_mode
                .clone()
                .or(spec.execution_policy.fork_mode.clone()),
            reuse_scope_summary: spec.child_policy.reuse_scope_summary.clone(),
        },
        fork_mode: spec
            .execution_policy
            .fork_mode
            .clone()
            .or(spec.child_policy.fork_mode.clone()),
        diagnostics: Vec::new(),
    };
    Ok(CompiledModeEnvelope { envelope })
}

fn compiled_mode_contracts(spec: &GovernanceModeSpec) -> CompiledModeContracts {
    CompiledModeContracts {
        artifact: spec.artifact.clone(),
        exit_gate: spec.exit_gate.clone(),
        prompt_hooks: spec.prompt_hooks.clone(),
    }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::ForkMode;
    use astrcode_governance_contract::ModeId;

    use super::compile_mode_envelope_for_child;
    use crate::{mode::builtin_mode_specs, mode_catalog_service::ServerModeCatalog};

    fn builtin_test_catalog() -> Arc<ServerModeCatalog> {
        ServerModeCatalog::from_mode_specs(builtin_mode_specs(), Vec::new())
            .expect("builtin catalog should build")
    }

    #[test]
    fn compile_mode_envelope_projects_mode_contracts_into_compile_artifact() {
        let catalog = builtin_test_catalog();
        let plan = catalog.get(&ModeId::plan()).unwrap();

        let compiled =
            super::compile_mode_envelope(&plan, Vec::new()).expect("plan should compile");

        assert_eq!(
            compiled
                .envelope
                .mode_contracts
                .artifact
                .as_ref()
                .map(|value| value.artifact_type.as_str()),
            Some("canonical-plan")
        );
        assert_eq!(
            compiled
                .envelope
                .mode_contracts
                .exit_gate
                .as_ref()
                .map(|value| value.review_passes),
            Some(1)
        );
        assert!(
            compiled.envelope.mode_contracts.prompt_hooks.is_some(),
            "plan compile artifact should carry prompt hooks"
        );
    }

    #[test]
    fn child_mode_compile_uses_child_fork_mode_for_child_execution_fallback() {
        let catalog = builtin_test_catalog();
        let mut mode = catalog.get(&ModeId::code()).unwrap();
        mode.execution_policy.fork_mode = None;
        mode.child_policy.fork_mode = Some(ForkMode::LastNTurns(4));

        let compiled =
            compile_mode_envelope_for_child(&mode).expect("child envelope should compile");

        assert_eq!(
            compiled.envelope.fork_mode,
            Some(ForkMode::LastNTurns(4)),
            "child execution should inherit childPolicy.forkMode when executionPolicy has no \
             forkMode"
        );
        assert_eq!(
            compiled.envelope.child_policy.fork_mode,
            Some(ForkMode::LastNTurns(4))
        );
    }
}
