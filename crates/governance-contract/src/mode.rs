//! # 声明式治理模式系统
//!
//! 重导出自 `astrcode_core::mode`，消除双重定义。
//! 所有类型定义由 `core` 统一持有。

pub use astrcode_core::mode::{
    ActionPolicies, ActionPolicyEffect, ActionPolicyRule, BUILTIN_MODE_CODE_ID,
    BUILTIN_MODE_PLAN_ID, BoundModeToolContractSnapshot, CapabilitySelector, ChildPolicySpec,
    CompiledModeContracts, GovernanceModeSpec, ModeArtifactDef, ModeExecutionPolicySpec,
    ModeExitGateDef, ModeId, ModePromptHooks, PromptProgramEntry, ResolvedChildPolicy,
    ResolvedTurnEnvelope, SubmitBusyPolicy, TransitionPolicySpec,
};
