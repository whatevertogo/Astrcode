use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub reason: Option<String>,
}

pub trait PolicyEngine: Send + Sync {
    fn check_tool_call(&self, tool: &str, args: &Value) -> PolicyDecision;
    fn check_capability(&self, capability: &str) -> PolicyDecision;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAllPolicyEngine;

impl PolicyEngine for AllowAllPolicyEngine {
    fn check_tool_call(&self, _tool: &str, _args: &Value) -> PolicyDecision {
        PolicyDecision {
            allowed: true,
            reason: None,
        }
    }

    fn check_capability(&self, _capability: &str) -> PolicyDecision {
        PolicyDecision {
            allowed: true,
            reason: None,
        }
    }
}
