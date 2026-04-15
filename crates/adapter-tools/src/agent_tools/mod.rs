mod close_tool;
mod collab_result_mapping;
mod collaboration_executor;
mod executor;
mod observe_tool;
mod result_mapping;
mod send_tool;
mod spawn_tool;

pub use astrcode_core::{
    CloseAgentParams, CollaborationResult, CollaborationResultKind, ObserveParams, SendAgentParams,
    SendToChildParams, SendToParentParams, SpawnAgentParams,
};
pub use close_tool::CloseAgentTool;
pub use collaboration_executor::CollaborationExecutor;
pub use executor::SubAgentExecutor;
pub use observe_tool::ObserveAgentTool;
pub use send_tool::SendAgentTool;
pub use spawn_tool::SpawnAgentTool;

#[cfg(test)]
mod tests;
