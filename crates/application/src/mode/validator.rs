//! 治理模式转换校验。
//!
//! 校验 session 从一个 mode 切换到另一个 mode 是否合法：
//! 检查当前 mode 的 `transition_policy.allowed_targets` 是否包含目标 mode。

use astrcode_core::{AstrError, GovernanceModeSpec, ModeId, Result};

use super::ModeCatalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeTransitionDecision {
    pub from_mode_id: ModeId,
    pub to_mode_id: ModeId,
    pub target: GovernanceModeSpec,
}

pub fn validate_mode_transition(
    catalog: &ModeCatalog,
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
    use super::validate_mode_transition;
    use crate::mode::builtin_mode_catalog;

    #[test]
    fn builtin_transition_accepts_known_target() {
        let catalog = builtin_mode_catalog().expect("builtin catalog should build");
        let decision = validate_mode_transition(
            &catalog,
            &astrcode_core::ModeId::code(),
            &astrcode_core::ModeId::plan(),
        )
        .expect("transition should succeed");
        assert_eq!(decision.to_mode_id, astrcode_core::ModeId::plan());
    }

    #[test]
    fn unknown_target_is_rejected() {
        let catalog = builtin_mode_catalog().expect("builtin catalog should build");
        let error = validate_mode_transition(
            &catalog,
            &astrcode_core::ModeId::code(),
            &astrcode_core::ModeId::from("missing"),
        )
        .expect_err("unknown target should fail");
        assert!(error.to_string().contains("unknown target mode"));
    }
}
