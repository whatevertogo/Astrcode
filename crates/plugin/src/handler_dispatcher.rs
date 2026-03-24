use std::collections::BTreeMap;

use astrcode_core::{AstrError, Result};
use astrcode_protocol::plugin::HandlerDescriptor;

#[derive(Debug, Default, Clone)]
pub struct HandlerDispatcher {
    handlers: BTreeMap<String, HandlerDescriptor>,
}

impl HandlerDispatcher {
    pub fn register(&mut self, handler: HandlerDescriptor) -> Result<()> {
        if self.handlers.contains_key(&handler.id) {
            return Err(AstrError::Validation(format!(
                "duplicate handler registration: {}",
                handler.id
            )));
        }
        self.handlers.insert(handler.id.clone(), handler);
        Ok(())
    }

    pub fn descriptors(&self) -> Vec<HandlerDescriptor> {
        self.handlers.values().cloned().collect()
    }
}
