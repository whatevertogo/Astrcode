use astrcode_protocol::plugin::CapabilityDescriptor;
use serde_json::Value;

use crate::PluginContext;

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

pub trait PolicyHook: Send + Sync {
    fn before_invoke(
        &self,
        capability: &CapabilityDescriptor,
        context: &PluginContext,
    ) -> PolicyDecision;
}
