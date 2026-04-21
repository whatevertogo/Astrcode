use astrcode_core::{AgentEvent, SessionEventRecord};
use astrcode_session_runtime::ConversationStreamProjector as RuntimeConversationStreamProjector;

use super::{ConversationDeltaFrameFacts, ConversationStreamReplayFacts, runtime_mapping};

pub struct ConversationStreamProjector {
    projector: RuntimeConversationStreamProjector,
}

impl ConversationStreamProjector {
    pub fn new(last_sent_cursor: Option<String>, facts: &ConversationStreamReplayFacts) -> Self {
        Self {
            projector: RuntimeConversationStreamProjector::new(
                last_sent_cursor,
                &runtime_mapping::into_runtime_stream_replay(facts),
            ),
        }
    }

    pub fn last_sent_cursor(&self) -> Option<&str> {
        self.projector.last_sent_cursor()
    }

    pub fn seed_initial_replay(
        &mut self,
        facts: &ConversationStreamReplayFacts,
    ) -> Vec<ConversationDeltaFrameFacts> {
        self.projector
            .seed_initial_replay(&runtime_mapping::into_runtime_stream_replay(facts))
            .into_iter()
            .map(runtime_mapping::map_frame)
            .collect()
    }

    pub fn project_durable_record(
        &mut self,
        record: &SessionEventRecord,
    ) -> Vec<ConversationDeltaFrameFacts> {
        self.projector
            .project_durable_record(record)
            .into_iter()
            .map(runtime_mapping::map_frame)
            .collect()
    }

    pub fn project_live_event(&mut self, event: &AgentEvent) -> Vec<ConversationDeltaFrameFacts> {
        self.projector
            .project_live_event(event)
            .into_iter()
            .map(runtime_mapping::map_frame)
            .collect()
    }

    pub fn recover_from(
        &mut self,
        recovered: &ConversationStreamReplayFacts,
    ) -> Vec<ConversationDeltaFrameFacts> {
        self.projector
            .recover_from(&runtime_mapping::into_runtime_stream_replay(recovered))
            .into_iter()
            .map(runtime_mapping::map_frame)
            .collect()
    }
}
