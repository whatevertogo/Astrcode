use astrcode_core::{CapabilityRouter, ToolRegistry};

pub(crate) use astrcode_core::test_support::{
    env_lock, test_home_dir, TestEnvGuard, TEST_HOME_ENV,
};

pub(crate) fn empty_capabilities() -> CapabilityRouter {
    CapabilityRouter::builder()
        .build()
        .expect("empty capability router should build")
}

pub(crate) fn capabilities_from_tools(tools: ToolRegistry) -> CapabilityRouter {
    let mut builder = CapabilityRouter::builder();
    for invoker in tools
        .into_capability_invokers()
        .expect("tool descriptors should build")
    {
        builder = builder.register_invoker(invoker);
    }
    builder
        .build()
        .expect("tool-derived capability router should build")
}
