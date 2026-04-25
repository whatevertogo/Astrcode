//! 治理模式转换校验。
//!
//! 校验 session 从一个 mode 切换到另一个 mode 是否合法：
//! 检查当前 mode 的 `transition_policy.allowed_targets` 是否包含目标 mode。

use astrcode_core::{AstrError, Result};
use astrcode_governance_contract::{GovernanceModeSpec, ModeId};

use crate::mode_catalog_service::ServerModeCatalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeTransitionDecision {
    pub from_mode_id: ModeId,
    pub to_mode_id: ModeId,
    pub target: GovernanceModeSpec,
}

pub fn validate_mode_transition(
    catalog: &ServerModeCatalog,
    from_mode_id: &ModeId,
    to_mode_id: &ModeId,
) -> Result<ModeTransitionDecision> {
    let current = catalog
        .get(from_mode_id)
        .ok_or_else(|| AstrError::Validation(format!("unknown current mode '{}'", from_mode_id)))?;
    let target = catalog
        .get(to_mode_id)
        .ok_or_else(|| AstrError::Validation(format!("unknown target mode '{}'", to_mode_id)))?;
    if !current
        .transition_policy
        .allowed_targets
        .iter()
        .any(|candidate| candidate == to_mode_id)
    {
        return Err(AstrError::Validation(format!(
            "mode transition '{}' -> '{}' is not allowed",
            from_mode_id, to_mode_id
        )));
    }
    Ok(ModeTransitionDecision {
        from_mode_id: from_mode_id.clone(),
        to_mode_id: to_mode_id.clone(),
        target,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::Result;
    use astrcode_governance_contract::ModeId;

    use super::validate_mode_transition;
    use crate::{mode::catalog::builtin_mode_specs, mode_catalog_service::ServerModeCatalog};

    fn builtin_test_catalog() -> Result<Arc<ServerModeCatalog>> {
        ServerModeCatalog::from_mode_specs(builtin_mode_specs(), Vec::new())
    }

    #[test]
    fn builtin_transition_accepts_known_target() -> Result<()> {
        let catalog = builtin_test_catalog()?;
        let decision = validate_mode_transition(&catalog, &ModeId::code(), &ModeId::plan())
            .expect("transition should succeed");
        assert_eq!(decision.to_mode_id, ModeId::plan());
        Ok(())
    }

    #[test]
    fn unknown_target_is_rejected() -> Result<()> {
        let catalog = builtin_test_catalog()?;
        let error = validate_mode_transition(&catalog, &ModeId::code(), &ModeId::from("missing"))
            .expect_err("unknown target should fail");
        assert!(error.to_string().contains("unknown target mode"));
        Ok(())
    }
}
