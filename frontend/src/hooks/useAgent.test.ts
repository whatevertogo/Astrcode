import { describe, expect, it } from 'vitest';

import type { ConversationSnapshotState } from '../lib/api/conversation';
import { processConversationStreamEnvelope } from './useAgent';

const baseState: ConversationSnapshotState = {
  cursor: '1.0',
  phase: 'idle',
  control: {
    phase: 'idle',
    canSubmitPrompt: true,
    canRequestCompact: true,
    compactPending: false,
    compacting: false,
    currentModeId: 'code',
  },
  stepProgress: {
    durable: null,
    live: null,
  },
  blocks: [],
  childSummaries: [],
};

describe('processConversationStreamEnvelope', () => {
  it('signals snapshot reload when the stream requests rehydration', () => {
    const state: ConversationSnapshotState = {
      ...baseState,
      control: {
        ...baseState.control,
      },
      stepProgress: {
        ...baseState.stepProgress,
      },
      blocks: [...baseState.blocks],
      childSummaries: [...baseState.childSummaries],
    };

    const result = processConversationStreamEnvelope(
      state,
      JSON.stringify({
        kind: 'rehydrate_required',
        cursor: '5.0',
        requestedCursor: '43.1',
        latestCursor: '5.0',
      })
    );

    expect(result).toEqual({ kind: 'rehydrate_required' });
    expect(state.cursor).toBe('1.0');
    expect(state.blocks).toHaveLength(0);
  });

  it('still projects ordinary envelopes into conversation state', () => {
    const state: ConversationSnapshotState = {
      ...baseState,
      control: {
        ...baseState.control,
      },
      stepProgress: {
        ...baseState.stepProgress,
      },
      blocks: [...baseState.blocks],
      childSummaries: [...baseState.childSummaries],
    };

    const result = processConversationStreamEnvelope(
      state,
      JSON.stringify({
        kind: 'update_control_state',
        cursor: '2.0',
        control: {
          phase: 'callingTool',
          canSubmitPrompt: false,
          canRequestCompact: true,
          compactPending: false,
          compacting: false,
          currentModeId: 'plan',
        },
      })
    );

    expect(result.kind).toBe('projection');
    expect(state.cursor).toBe('2.0');
    expect(state.phase).toBe('callingTool');
    expect(state.control.currentModeId).toBe('plan');
    if (result.kind === 'projection') {
      expect(result.projection.cursor).toBe('2.0');
      expect(result.projection.control.currentModeId).toBe('plan');
    }
  });
});
