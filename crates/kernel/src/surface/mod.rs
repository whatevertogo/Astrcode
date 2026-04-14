use std::sync::{Arc, RwLock};

use astrcode_core::{CapabilityInvoker, CapabilitySpec, support};

use crate::events::{EventHub, KernelEvent};

/// Kernel 对外暴露的能力面快照。
///
/// 这层不持有执行器本身，只保留稳定的 capability 元信息，
/// 供上层查看“当前 kernel 能做什么”。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SurfaceSnapshot {
    pub capability_specs: Vec<CapabilitySpec>,
}

impl SurfaceSnapshot {
    fn from_invokers(invokers: &[Arc<dyn CapabilityInvoker>]) -> Self {
        Self {
            capability_specs: invokers
                .iter()
                .map(|invoker| invoker.capability_spec())
                .collect(),
        }
    }

    fn capability_count(&self) -> usize {
        self.capability_specs.len()
    }
}

/// 维护当前 capability surface 的只读快照，并在刷新时发出事件。
#[derive(Clone, Default)]
pub struct SurfaceManager {
    snapshot: Arc<RwLock<SurfaceSnapshot>>,
}

impl SurfaceManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> SurfaceSnapshot {
        support::with_read_lock_recovery(&self.snapshot, "kernel.surface", Clone::clone)
    }

    pub fn replace_capabilities(
        &self,
        invokers: &[Arc<dyn CapabilityInvoker>],
        events: &EventHub,
    ) -> SurfaceSnapshot {
        let next = SurfaceSnapshot::from_invokers(invokers);
        support::with_write_lock_recovery(&self.snapshot, "kernel.surface", |snapshot| {
            *snapshot = next.clone();
        });
        events.publish(KernelEvent::SurfaceRefreshed {
            capability_count: next.capability_count(),
        });
        next
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, CapabilityKind,
        CapabilitySpec, Result,
    };
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::SurfaceManager;
    use crate::events::{EventHub, KernelEvent};

    struct FakeInvoker {
        spec: CapabilitySpec,
    }

    #[async_trait]
    impl CapabilityInvoker for FakeInvoker {
        fn capability_spec(&self) -> CapabilitySpec {
            self.spec.clone()
        }

        async fn invoke(
            &self,
            _payload: Value,
            _ctx: &CapabilityContext,
        ) -> Result<CapabilityExecutionResult> {
            unreachable!("surface tests only inspect capability metadata")
        }
    }

    fn fake_invoker(name: &str) -> Arc<dyn CapabilityInvoker> {
        Arc::new(FakeInvoker {
            spec: CapabilitySpec::builder(name, CapabilityKind::Tool)
                .description(format!("tool {name}"))
                .input_schema(json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }))
                .output_schema(json!({
                    "type": "string"
                }))
                .build()
                .expect("fake capability spec should build"),
        })
    }

    #[test]
    fn replace_capabilities_updates_snapshot_and_publishes_event() {
        let manager = SurfaceManager::new();
        let events = EventHub::new(8);
        let mut receiver = events.subscribe();

        let snapshot = manager
            .replace_capabilities(&[fake_invoker("list_dir"), fake_invoker("grep")], &events);

        assert_eq!(snapshot.capability_specs.len(), 2);
        assert_eq!(snapshot.capability_specs[0].name.as_str(), "list_dir");
        assert_eq!(snapshot.capability_specs[1].name.as_str(), "grep");
        assert_eq!(manager.snapshot(), snapshot);
        assert_eq!(
            receiver
                .try_recv()
                .expect("surface refresh event should publish"),
            KernelEvent::SurfaceRefreshed {
                capability_count: 2
            }
        );
    }
}
