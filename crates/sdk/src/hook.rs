//! Plugin-local policy utilities.
//!
//! These hooks run inside a plugin process and help plugin authors compose reusable allow/deny
//! checks around their own handlers. They are intentionally narrower than the host runtime's
//! global policy contract, which also covers approval, context pressure, and model request
//! rewriting.

use std::sync::Arc;

use astrcode_protocol::plugin::CapabilityDescriptor;
use serde_json::Value;

use crate::{PluginContext, SdkError};

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub reason: Option<String>,
    pub metadata: Value,
}

impl PolicyDecision {
    pub fn allow() -> Self {
        Self {
            allowed: true,
            reason: None,
            metadata: Value::Null,
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: Some(reason.into()),
            metadata: Value::Null,
        }
    }
}

/// A lightweight pre-invocation guard around plugin-owned capabilities.
///
/// Use this for plugin-local validation and gating. Host-level approval, sandbox, or runtime
/// policy should stay in the host runtime rather than being reimplemented here.
pub trait PolicyHook: Send + Sync {
    fn before_invoke(
        &self,
        capability: &CapabilityDescriptor,
        context: &PluginContext,
    ) -> PolicyDecision;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HookShortCircuit {
    Never,
    #[default]
    OnDeny,
}

#[derive(Clone)]
pub struct RegisteredPolicyHook {
    name: String,
    hook: Arc<dyn PolicyHook>,
}

impl RegisteredPolicyHook {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn hook(&self) -> &dyn PolicyHook {
        self.hook.as_ref()
    }
}

#[derive(Clone, Default)]
pub struct HookRegistry {
    policy_hooks: Vec<RegisteredPolicyHook>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_policy_hook<H>(
        &mut self,
        name: impl Into<String>,
        hook: H,
    ) -> Result<&mut Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        let name = normalize_hook_name(name)?;
        if self
            .policy_hooks
            .iter()
            .any(|registered| registered.name == name)
        {
            return Err(SdkError::validation(format!(
                "duplicate policy hook registration '{name}'"
            )));
        }
        self.policy_hooks.push(RegisteredPolicyHook {
            name,
            hook: Arc::new(hook),
        });
        Ok(self)
    }

    pub fn with_policy_hook<H>(mut self, name: impl Into<String>, hook: H) -> Result<Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        self.register_policy_hook(name, hook)?;
        Ok(self)
    }

    pub fn policy_hooks(&self) -> &[RegisteredPolicyHook] {
        &self.policy_hooks
    }

    pub fn policy_hook_chain(&self) -> PolicyHookChain {
        // Registry owns hooks by Arc so plugin authors can build reusable chains
        // without moving or reconstructing the original registrations.
        PolicyHookChain {
            hooks: self.policy_hooks.clone(),
            short_circuit: HookShortCircuit::default(),
        }
    }
}

#[derive(Clone, Default)]
pub struct PolicyHookChain {
    hooks: Vec<RegisteredPolicyHook>,
    short_circuit: HookShortCircuit,
}

impl PolicyHookChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_registry(registry: &HookRegistry) -> Self {
        registry.policy_hook_chain()
    }

    pub fn with_short_circuit(mut self, short_circuit: HookShortCircuit) -> Self {
        self.short_circuit = short_circuit;
        self
    }

    pub fn register<H>(&mut self, name: impl Into<String>, hook: H) -> Result<&mut Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        let name = normalize_hook_name(name)?;
        if self.hooks.iter().any(|registered| registered.name == name) {
            return Err(SdkError::validation(format!(
                "duplicate policy hook registration '{name}'"
            )));
        }
        self.hooks.push(RegisteredPolicyHook {
            name,
            hook: Arc::new(hook),
        });
        Ok(self)
    }

    pub fn with_hook<H>(mut self, name: impl Into<String>, hook: H) -> Result<Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        self.register(name, hook)?;
        Ok(self)
    }

    pub fn hooks(&self) -> &[RegisteredPolicyHook] {
        &self.hooks
    }

    pub fn short_circuit(&self) -> HookShortCircuit {
        self.short_circuit
    }
}

impl PolicyHook for PolicyHookChain {
    fn before_invoke(
        &self,
        capability: &CapabilityDescriptor,
        context: &PluginContext,
    ) -> PolicyDecision {
        let mut final_decision = PolicyDecision::allow();
        for registered in &self.hooks {
            let decision = registered.hook.before_invoke(capability, context);
            // Policies default to fail-fast on deny so a guard hook can veto the
            // invocation before later hooks add more permissive behavior.
            let should_stop =
                matches!(self.short_circuit, HookShortCircuit::OnDeny) && !decision.allowed;
            final_decision = decision;
            if should_stop {
                break;
            }
        }
        final_decision
    }
}

fn normalize_hook_name(name: impl Into<String>) -> Result<String, SdkError> {
    let name = name.into();
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SdkError::validation(
            "policy hook registration requires a non-empty name",
        ));
    }
    Ok(trimmed.to_string())
}
