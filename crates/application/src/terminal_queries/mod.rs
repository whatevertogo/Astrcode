mod cursor;
mod resume;
mod snapshot;
mod summary;
#[cfg(test)]
mod tests;

use astrcode_session_runtime::SessionControlStateSnapshot;

use crate::terminal::{TerminalControlFacts, TerminalLastCompactMetaFacts};

fn map_control_facts(control: SessionControlStateSnapshot) -> TerminalControlFacts {
    TerminalControlFacts {
        phase: control.phase,
        active_turn_id: control.active_turn_id,
        manual_compact_pending: control.manual_compact_pending,
        compacting: control.compacting,
        last_compact_meta: control
            .last_compact_meta
            .map(|meta| TerminalLastCompactMetaFacts {
                trigger: meta.trigger,
                meta: meta.meta,
            }),
    }
}
