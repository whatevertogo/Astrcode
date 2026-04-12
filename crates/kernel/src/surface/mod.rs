use std::sync::{Arc, RwLock};

use astrcode_core::{CapabilityInvoker, CapabilitySpec, support};

use crate::events::{EventHub, KernelEvent};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SurfaceSnapshot {
    pub capability_specs: Vec<CapabilitySpec>,
}

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
        let capability_specs = invokers
            .iter()
            .map(|invoker| invoker.capability_spec())
            .collect::<Vec<_>>();
        let next = SurfaceSnapshot { capability_specs };
        support::with_write_lock_recovery(&self.snapshot, "kernel.surface", |snapshot| {
            *snapshot = next.clone();
        });
        events.publish(KernelEvent::SurfaceRefreshed {
            capability_count: next.capability_specs.len(),
        });
        next
    }
}
