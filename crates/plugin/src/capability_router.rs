use std::collections::BTreeMap;
use std::sync::Arc;

use astrcode_core::{AstrError, CancelToken, Result};
use astrcode_protocol::plugin::{CapabilityDescriptor, InvocationContext};
use async_trait::async_trait;
use serde_json::Value;

use crate::EventEmitter;

#[async_trait]
pub trait CapabilityHandler: Send + Sync {
    fn descriptor(&self) -> CapabilityDescriptor;

    async fn invoke(
        &self,
        input: Value,
        context: InvocationContext,
        events: EventEmitter,
        cancel: CancelToken,
    ) -> Result<Value>;
}

pub trait PermissionChecker: Send + Sync {
    fn check(&self, capability: &CapabilityDescriptor, context: &InvocationContext) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct AllowAllPermissionChecker;

impl PermissionChecker for AllowAllPermissionChecker {
    fn check(
        &self,
        _capability: &CapabilityDescriptor,
        _context: &InvocationContext,
    ) -> Result<()> {
        Ok(())
    }
}

pub struct CapabilityRouter {
    handlers: BTreeMap<String, Arc<dyn CapabilityHandler>>,
    permission_checker: Arc<dyn PermissionChecker>,
}

impl Default for CapabilityRouter {
    fn default() -> Self {
        Self::new(Arc::new(AllowAllPermissionChecker))
    }
}

impl CapabilityRouter {
    pub fn new(permission_checker: Arc<dyn PermissionChecker>) -> Self {
        Self {
            handlers: BTreeMap::new(),
            permission_checker,
        }
    }

    pub fn register<H>(&mut self, handler: H) -> Result<()>
    where
        H: CapabilityHandler + 'static,
    {
        self.register_arc(Arc::new(handler))
    }

    pub fn register_arc(&mut self, handler: Arc<dyn CapabilityHandler>) -> Result<()> {
        let descriptor = handler.descriptor();
        descriptor.validate().map_err(|error| {
            AstrError::Validation(format!(
                "invalid capability descriptor '{}': {}",
                descriptor.name, error
            ))
        })?;
        if self.handlers.contains_key(&descriptor.name) {
            return Err(AstrError::Validation(format!(
                "duplicate capability registration: {}",
                descriptor.name
            )));
        }
        self.handlers.insert(descriptor.name.clone(), handler);
        Ok(())
    }

    pub fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        self.handlers
            .values()
            .map(|handler| handler.descriptor())
            .collect()
    }

    pub async fn invoke(
        &self,
        capability: &str,
        input: Value,
        context: InvocationContext,
        events: EventEmitter,
        cancel: CancelToken,
    ) -> Result<Value> {
        let handler = self
            .handlers
            .get(capability)
            .ok_or_else(|| AstrError::Validation(format!("unknown capability '{capability}'")))?;
        let descriptor = handler.descriptor();
        self.validate_profile(&descriptor, &context)?;
        self.permission_checker.check(&descriptor, &context)?;
        handler.invoke(input, context, events, cancel).await
    }

    fn validate_profile(
        &self,
        descriptor: &CapabilityDescriptor,
        context: &InvocationContext,
    ) -> Result<()> {
        if descriptor.profiles.is_empty() || descriptor.profiles.contains(&context.profile) {
            return Ok(());
        }
        Err(AstrError::Validation(format!(
            "capability '{}' does not support profile '{}'",
            descriptor.name, context.profile
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrcode_protocol::plugin::{CapabilityKind, SideEffectLevel, StabilityLevel};
    use serde_json::json;

    struct SampleHandler;

    #[async_trait]
    impl CapabilityHandler for SampleHandler {
        fn descriptor(&self) -> CapabilityDescriptor {
            CapabilityDescriptor {
                name: "tool.sample".to_string(),
                kind: CapabilityKind::tool(),
                description: "sample".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                streaming: false,
                concurrency_safe: false,
                profiles: vec!["coding".to_string()],
                tags: vec![],
                permissions: vec![],
                side_effect: SideEffectLevel::None,
                stability: StabilityLevel::Stable,
                metadata: Value::Null,
            }
        }

        async fn invoke(
            &self,
            input: Value,
            _context: InvocationContext,
            _events: EventEmitter,
            _cancel: CancelToken,
        ) -> Result<Value> {
            Ok(input)
        }
    }

    struct DenyChecker;

    impl PermissionChecker for DenyChecker {
        fn check(
            &self,
            _capability: &CapabilityDescriptor,
            _context: &InvocationContext,
        ) -> Result<()> {
            Err(AstrError::Validation("denied by checker".to_string()))
        }
    }

    fn context(profile: &str) -> InvocationContext {
        InvocationContext {
            request_id: "req-1".to_string(),
            trace_id: None,
            session_id: None,
            caller: None,
            workspace: None,
            deadline_ms: None,
            budget: None,
            profile: profile.to_string(),
            profile_context: Value::Null,
            metadata: Value::Null,
        }
    }

    #[tokio::test]
    async fn router_rejects_unsupported_profile() {
        let mut router = CapabilityRouter::default();
        router.register(SampleHandler).expect("register handler");

        let error = router
            .invoke(
                "tool.sample",
                json!({}),
                context("workflow"),
                EventEmitter::noop(),
                CancelToken::new(),
            )
            .await
            .expect_err("unsupported profile should fail");
        assert!(matches!(error, AstrError::Validation(_)));
        assert!(error.to_string().contains("does not support profile"));
    }

    #[tokio::test]
    async fn router_applies_permission_checker_before_invocation() {
        let mut router = CapabilityRouter::new(Arc::new(DenyChecker));
        router.register(SampleHandler).expect("register handler");

        let error = router
            .invoke(
                "tool.sample",
                json!({}),
                context("coding"),
                EventEmitter::noop(),
                CancelToken::new(),
            )
            .await
            .expect_err("permission checker should fail");
        assert!(matches!(error, AstrError::Validation(_)));
        assert!(error.to_string().contains("denied by checker"));
    }
}
