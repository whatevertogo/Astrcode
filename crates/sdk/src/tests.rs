use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    CapabilityDescriptor, CapabilityKind, HookRegistry, PluginContext, PolicyDecision, PolicyHook,
    PolicyHookChain, SdkError, SideEffectLevel, StreamWriter, ToolFuture, ToolHandler,
    ToolRegistration, ToolSerdeStage,
};

/// Minimal synchronous executor for tests.
fn block_on<F: Future>(future: F) -> F::Output {
    fn noop_raw_waker() -> RawWaker {
        fn clone(_: *const ()) -> RawWaker {
            noop_raw_waker()
        }
        fn wake(_: *const ()) {}
        fn wake_by_ref(_: *const ()) {}
        fn drop(_: *const ()) {}
        RawWaker::new(
            std::ptr::null(),
            &RawWakerVTable::new(clone, wake, wake_by_ref, drop),
        )
    }
    let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
    let mut future = pin!(future);
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[derive(Default)]
struct SampleTool;

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct SampleInput {
    value: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SampleOutput {
    echoed: String,
}

impl ToolHandler<SampleInput, SampleOutput> for SampleTool {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::builder("tool.sample", CapabilityKind::tool())
            .description("Sample tool")
            .schema(json!({ "type": "object" }), json!({ "type": "object" }))
            .streaming(true)
            .profile("coding")
            .tag("sample")
            .side_effect(SideEffectLevel::None)
            .build()
            .expect("sample descriptor should build")
    }

    fn execute(
        &self,
        input: SampleInput,
        _context: PluginContext,
        stream: StreamWriter,
    ) -> ToolFuture<'_, SampleOutput> {
        Box::pin(async move {
            stream.message_delta("running")?;
            Ok(SampleOutput {
                echoed: input.value,
            })
        })
    }
}

struct TrackingPolicyHook {
    name: &'static str,
    allowed: bool,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl PolicyHook for TrackingPolicyHook {
    fn before_invoke(
        &self,
        _capability: &CapabilityDescriptor,
        _context: &PluginContext,
    ) -> PolicyDecision {
        self.calls
            .lock()
            .expect("tracking policy calls")
            .push(self.name);
        if self.allowed {
            PolicyDecision::allow()
        } else {
            PolicyDecision::deny(format!("{} denied the request", self.name))
        }
    }
}

// --- Critical contract: type-erased tool execution ---

#[test]
fn tool_registration_decodes_input_and_encodes_output_automatically() {
    let registration = ToolRegistration::new(SampleTool);
    let output = block_on(registration.handler().execute_value(
        json!({ "value": "hello" }),
        PluginContext::default(),
        StreamWriter::default(),
    ))
    .expect("typed tool execution should succeed");

    assert_eq!(output, json!({ "echoed": "hello" }));
}

#[test]
fn tool_registration_reports_typed_decode_errors() {
    let registration = ToolRegistration::new(SampleTool);
    let error = block_on(registration.handler().execute_value(
        json!({ "value": 42 }),
        PluginContext::default(),
        StreamWriter::default(),
    ))
    .expect_err("invalid typed payload should fail");

    let payload = error.to_error_payload();
    match error {
        SdkError::Serde {
            capability,
            stage,
            rust_type,
            ..
        } => {
            assert_eq!(capability, "tool.sample");
            assert_eq!(stage, ToolSerdeStage::DecodeInput);
            assert!(rust_type.contains("SampleInput"));
        }
        other => panic!("expected serde decode error, got {other:?}"),
    }
    assert_eq!(payload.code, "invalid_input");
    assert!(!payload.retriable);
}

// --- Critical contract: hook chain composition and short-circuit ---

#[test]
fn hook_registry_composes_policy_hooks_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = HookRegistry::default()
        .with_policy_hook(
            "first",
            TrackingPolicyHook {
                name: "first",
                allowed: true,
                calls: Arc::clone(&calls),
            },
        )
        .and_then(|registry| {
            registry.with_policy_hook(
                "second",
                TrackingPolicyHook {
                    name: "second",
                    allowed: true,
                    calls: Arc::clone(&calls),
                },
            )
        })
        .expect("policy hooks should register");

    let decision = registry
        .policy_hook_chain()
        .before_invoke(&SampleTool.descriptor(), &PluginContext::default());

    assert!(decision.allowed);
    assert_eq!(
        calls.lock().expect("tracking policy calls").as_slice(),
        ["first", "second"]
    );
}

#[test]
fn policy_hook_chain_short_circuits_after_first_deny() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let chain = PolicyHookChain::default()
        .with_hook(
            "allow",
            TrackingPolicyHook {
                name: "allow",
                allowed: true,
                calls: Arc::clone(&calls),
            },
        )
        .and_then(|chain| {
            chain.with_hook(
                "deny",
                TrackingPolicyHook {
                    name: "deny",
                    allowed: false,
                    calls: Arc::clone(&calls),
                },
            )
        })
        .and_then(|chain| {
            chain.with_hook(
                "never-runs",
                TrackingPolicyHook {
                    name: "never-runs",
                    allowed: true,
                    calls: Arc::clone(&calls),
                },
            )
        })
        .expect("policy chain should register");

    let decision = chain.before_invoke(&SampleTool.descriptor(), &PluginContext::default());

    assert!(!decision.allowed);
    assert_eq!(decision.reason.as_deref(), Some("deny denied the request"));
    assert_eq!(
        calls.lock().expect("tracking policy calls").as_slice(),
        ["allow", "deny"]
    );
}

// --- Critical contract: error to protocol payload mapping ---

#[test]
fn sdk_error_maps_to_protocol_payload() {
    let error = SdkError::permission_denied("filesystem.write requires approval");
    let payload = error.to_error_payload();

    assert_eq!(payload.code, "permission_denied");
    assert_eq!(
        payload.message,
        "permission denied: filesystem.write requires approval"
    );
    assert_eq!(payload.details, serde_json::Value::Null);
    assert!(!payload.retriable);
}
