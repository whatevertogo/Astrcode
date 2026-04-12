use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum KernelEvent {
    SurfaceRefreshed { capability_count: usize },
}

#[derive(Clone)]
pub struct EventHub {
    sender: broadcast::Sender<KernelEvent>,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    pub fn publish(&self, event: KernelEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<KernelEvent> {
        self.sender.subscribe()
    }
}
