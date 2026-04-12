//! 子会话策略校验模块。
//!
//! 负责解析和验证 `SubagentContextOverrides`，确保子会话的行为符合当前配置约束。
//! 该模块独立于执行装配逻辑，方便后续扩展策略规则。
//!
//! 新架构中不再依赖 runtime-config，改为纯值校验。
//! 组合根在构造时把配置值显式注入。

use astrcode_core::{AstrError, ResolvedSubagentContextOverrides, SubagentContextOverrides};

/// 策略校验失败的分类标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyViolation {
    InconsistentInstructionInheritance,
    WorkingDirIsolation,
    CancelTokenIsolation,
    RecoveryRefsEnabled,
    ParentFindingsEnabled,
}

impl PolicyViolation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InconsistentInstructionInheritance => "inconsistent_instruction_inheritance",
            Self::WorkingDirIsolation => "working_dir_isolation",
            Self::CancelTokenIsolation => "cancel_token_isolation",
            Self::RecoveryRefsEnabled => "recovery_refs_enabled",
            Self::ParentFindingsEnabled => "parent_findings_enabled",
        }
    }
}

/// 解析并校验子会话上下文覆盖。
///
/// 如果 overrides 为 None，返回默认的强隔离策略快照。
/// 如果校验失败，返回描述性错误信息。
pub fn resolve_subagent_overrides(
    overrides: Option<&SubagentContextOverrides>,
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

    validate_resolved_overrides(&resolved)?;

    Ok(resolved)
}

/// 统一校验已解析的 override 快照是否符合约束。
fn validate_resolved_overrides(
    resolved: &ResolvedSubagentContextOverrides,
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

    use super::resolve_subagent_overrides;

    #[test]
    fn resolve_subagent_overrides_accepts_independent_session_by_default() {
        let overrides = SubagentContextOverrides {
            storage_mode: Some(SubRunStorageMode::IndependentSession),
            ..SubagentContextOverrides::default()
        };

        let resolved =
            resolve_subagent_overrides(Some(&overrides)).expect("independent session accepted");

        assert_eq!(resolved.storage_mode, SubRunStorageMode::IndependentSession);
    }

    #[test]
    fn resolve_subagent_overrides_rejects_inconsistent_instruction_inheritance() {
        let overrides = SubagentContextOverrides {
            inherit_system_instructions: Some(true),
            inherit_project_instructions: Some(false),
            ..SubagentContextOverrides::default()
        };

        let result = resolve_subagent_overrides(Some(&overrides));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("inheritSystemInstructions")
        );
    }

    #[test]
    fn resolve_subagent_overrides_preserves_fork_mode() {
        let overrides = SubagentContextOverrides {
            fork_mode: Some(ForkMode::LastNTurns(3)),
            ..SubagentContextOverrides::default()
        };

        let resolved = resolve_subagent_overrides(Some(&overrides)).expect("fork mode preserved");

        assert_eq!(resolved.fork_mode, Some(ForkMode::LastNTurns(3)));
    }
}
