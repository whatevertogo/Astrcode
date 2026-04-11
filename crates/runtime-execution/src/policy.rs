//! 子会话策略校验模块。
//!
//! 负责解析和验证 `SubagentContextOverrides`，确保子会话的行为符合当前
//! runtime 配置约束。该模块独立于执行装配逻辑，方便后续扩展策略规则。

use astrcode_core::{AstrError, ResolvedSubagentContextOverrides, SubagentContextOverrides};

/// 解析并校验子会话上下文覆盖。
///
/// 如果 overrides 为 None，返回默认的强隔离策略快照。
/// 如果校验失败，返回描述性错误信息。
pub fn resolve_subagent_overrides(
    overrides: Option<&SubagentContextOverrides>,
    runtime_config: &astrcode_runtime_config::RuntimeConfig,
) -> Result<ResolvedSubagentContextOverrides, AstrError> {
    let mut resolved = ResolvedSubagentContextOverrides::default();
    if let Some(overrides) = overrides {
        if let Some(storage_mode) = overrides.storage_mode {
            resolved.storage_mode = storage_mode;
        }
        if let Some(value) = overrides.inherit_system_instructions {
            resolved.inherit_system_instructions = value;
        }
        if let Some(value) = overrides.inherit_project_instructions {
            resolved.inherit_project_instructions = value;
        }
        if let Some(value) = overrides.inherit_working_dir {
            resolved.inherit_working_dir = value;
        }
        if let Some(value) = overrides.inherit_policy_upper_bound {
            resolved.inherit_policy_upper_bound = value;
        }
        if let Some(value) = overrides.inherit_cancel_token {
            resolved.inherit_cancel_token = value;
        }
        if let Some(value) = overrides.include_compact_summary {
            resolved.include_compact_summary = value;
        }
        if let Some(value) = overrides.include_recent_tail {
            resolved.include_recent_tail = value;
        }
        if let Some(value) = overrides.include_recovery_refs {
            resolved.include_recovery_refs = value;
        }
        if let Some(value) = overrides.include_parent_findings {
            resolved.include_parent_findings = value;
        }
        if let Some(fork_mode) = overrides.fork_mode.clone() {
            resolved.fork_mode = Some(fork_mode);
        }
    }

    validate_resolved_overrides(&resolved, runtime_config)?;

    Ok(resolved)
}

/// 统一校验已解析的 override 快照是否符合当前 runtime 约束。
fn validate_resolved_overrides(
    resolved: &ResolvedSubagentContextOverrides,
    _runtime_config: &astrcode_runtime_config::RuntimeConfig,
) -> Result<(), AstrError> {
    if resolved.inherit_system_instructions != resolved.inherit_project_instructions {
        return Err(AstrError::Validation(
            "inheritSystemInstructions and inheritProjectInstructions must currently resolve to \
             the same value"
                .to_string(),
        ));
    }
    if !resolved.inherit_working_dir {
        return Err(AstrError::Validation(
            "inheritWorkingDir=false is not supported yet; child agents must stay in the parent \
             workspace"
                .to_string(),
        ));
    }
    if !resolved.inherit_cancel_token {
        return Err(AstrError::Validation(
            "inheritCancelToken=false is not supported yet; child agents must stay linked to the \
             parent cancellation chain"
                .to_string(),
        ));
    }
    if resolved.include_recovery_refs {
        return Err(AstrError::Validation(
            "includeRecoveryRefs=true is not supported yet; recovery refs are not exposed to \
             sub-agent context overrides in this release"
                .to_string(),
        ));
    }
    if resolved.include_parent_findings {
        return Err(AstrError::Validation(
            "includeParentFindings=true is not supported yet; parent findings are not exposed to \
             sub-agent context overrides in this release"
                .to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use astrcode_core::{ForkMode, SubRunStorageMode, SubagentContextOverrides};
    use astrcode_runtime_config::RuntimeConfig;

    use super::resolve_subagent_overrides;

    /// 独立子会话模式已从实验特性升级为默认行为，不再受 experimental 开关约束。
    #[test]
    fn resolve_subagent_overrides_accepts_independent_session_by_default() {
        let overrides = SubagentContextOverrides {
            storage_mode: Some(SubRunStorageMode::IndependentSession),
            ..SubagentContextOverrides::default()
        };

        let resolved = resolve_subagent_overrides(Some(&overrides), &RuntimeConfig::default())
            .expect("independent session should be accepted by default");

        assert_eq!(resolved.storage_mode, SubRunStorageMode::IndependentSession);
    }


    #[test]
    fn resolve_subagent_overrides_rejects_inconsistent_instruction_inheritance() {
        let overrides = SubagentContextOverrides {
            inherit_system_instructions: Some(true),
            inherit_project_instructions: Some(false),
            ..SubagentContextOverrides::default()
        };
        let runtime_config = RuntimeConfig::default();

        let result = resolve_subagent_overrides(Some(&overrides), &runtime_config);

        assert!(result.is_err());
        let message = result.expect_err("must fail").to_string();
        assert!(message.contains("inheritSystemInstructions"));
    }

    #[test]
    fn resolve_subagent_overrides_preserves_fork_mode_for_future_consumers() {
        let overrides = SubagentContextOverrides {
            fork_mode: Some(ForkMode::LastNTurns(3)),
            ..SubagentContextOverrides::default()
        };

        let resolved = resolve_subagent_overrides(Some(&overrides), &RuntimeConfig::default())
            .expect("fork mode should remain visible in resolved overrides");

        assert_eq!(resolved.fork_mode, Some(ForkMode::LastNTurns(3)));
    }
}
