import { describe, expect, it } from 'vitest';

import type { ConversationSnapshotState } from './conversation';
import { applyConversationEnvelope, projectConversationState } from './conversation';

const baseControl = {
  phase: 'idle' as const,
  canSubmitPrompt: true,
  canRequestCompact: true,
  compactPending: false,
  compacting: false,
  currentModeId: 'code',
  activePlan: undefined,
};

describe('projectConversationState', () => {
  it('keeps thinking blocks visible even when the same turn also has assistant output', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-1',
      phase: 'streaming',
      blocks: [
        {
          id: 'thinking-1',
          kind: 'thinking',
          turnId: 'turn-1',
          markdown: '先分析问题，再给结论。',
          status: 'streaming',
        },
        {
          id: 'assistant-1',
          kind: 'assistant',
          turnId: 'turn-1',
          markdown: '这是答案。',
          status: 'streaming',
        },
      ],
      control: baseControl,
      childSummaries: [],
    };

    const projection = projectConversationState(state);

    expect(projection.messages).toHaveLength(2);
    expect(projection.messages[0]).toMatchObject({
      kind: 'assistant',
      turnId: 'turn-1',
      text: '',
      reasoningText: '先分析问题，再给结论。',
      streaming: true,
    });
    expect(projection.messages[1]).toMatchObject({
      kind: 'assistant',
      turnId: 'turn-1',
      text: '这是答案。',
      reasoningText: undefined,
      streaming: true,
    });
  });

  it('projects tool call blocks as authoritative messages with embedded streams', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-2',
      phase: 'callingTool',
      blocks: [
        {
          id: 'tool-call-1',
          kind: 'tool_call',
          turnId: 'turn-2',
          toolCallId: 'call-1',
          toolName: 'web',
          status: 'streaming',
          input: {
            query: 'codex tui architecture',
            maxResults: 5,
          },
          streams: {
            stdout: 'opening page\n',
            stderr: '',
          },
        },
      ],
      control: { ...baseControl, phase: 'callingTool' as const },
      childSummaries: [],
    };

    const projection = projectConversationState(state);

    expect(projection.messages).toHaveLength(1);
    expect(projection.messages[0]).toMatchObject({
      kind: 'toolCall',
      toolCallId: 'call-1',
      toolName: 'web',
      status: 'running',
      args: {
        query: 'codex tui architecture',
        maxResults: 5,
      },
      streams: {
        stdout: 'opening page\n',
        stderr: '',
      },
    });
  });

  it('keeps concurrent tool calls stable without creating sibling tool stream messages', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-3',
      phase: 'callingTool',
      blocks: [
        {
          id: 'tool-call-1',
          kind: 'tool_call',
          turnId: 'turn-3',
          toolCallId: 'call-1',
          toolName: 'shell',
          status: 'streaming',
          input: {
            command: 'rg conversation',
          },
          streams: {
            stdout: 'conversation.ts\n',
            stderr: 'warning: binary file skipped\n',
          },
        },
        {
          id: 'tool-call-2',
          kind: 'tool_call',
          turnId: 'turn-3',
          toolCallId: 'call-2',
          toolName: 'spawn',
          status: 'failed',
          summary: 'permission denied',
          error: 'permission denied',
          childRef: {
            agentId: 'agent-child-1',
            sessionId: 'session-root',
            subRunId: 'subrun-child-1',
            parentAgentId: 'agent-root',
            parentSubRunId: 'subrun-root',
            lineageKind: 'spawn',
            status: 'running',
            openSessionId: 'session-child-1',
          },
          streams: {
            stdout: '',
            stderr: 'permission denied\n',
          },
        },
      ],
      control: { ...baseControl, phase: 'callingTool' as const },
      childSummaries: [],
    };

    const projection = projectConversationState(state);

    expect(projection.messages).toHaveLength(2);
    expect(projection.messages[0]).toMatchObject({
      kind: 'toolCall',
      toolCallId: 'call-1',
      status: 'running',
      streams: {
        stdout: 'conversation.ts\n',
        stderr: 'warning: binary file skipped\n',
      },
    });
    expect(projection.messages[1]).toMatchObject({
      kind: 'toolCall',
      toolCallId: 'call-2',
      status: 'fail',
      output: 'permission denied',
      error: 'permission denied',
      childRef: {
        subRunId: 'subrun-child-1',
        openSessionId: 'session-child-1',
      },
      streams: {
        stdout: '',
        stderr: 'permission denied\n',
      },
    });
  });

  it('treats append_block as an idempotent upsert keyed by block id', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-1',
      phase: 'callingTool',
      blocks: [
        {
          id: 'tool-call-1',
          kind: 'tool_call',
          turnId: 'turn-2',
          toolCallId: 'call-1',
          toolName: 'web',
          status: 'streaming',
          input: {
            query: 'codex tui architecture',
          },
          streams: {
            stdout: '',
            stderr: '',
          },
        },
      ],
      control: { ...baseControl, phase: 'callingTool' as const },
      childSummaries: [],
    };

    applyConversationEnvelope(state, {
      cursor: 'cursor-2',
      kind: 'append_block',
      block: {
        id: 'tool-call-1',
        kind: 'tool_call',
        turnId: 'turn-2',
        toolCallId: 'call-1',
        toolName: 'web',
        status: 'complete',
        input: {
          query: 'codex tui architecture',
        },
        summary: '3 results',
        streams: {
          stdout: 'result line\n',
          stderr: '',
        },
      },
    });

    expect(state.cursor).toBe('cursor-2');
    expect(state.blocks).toHaveLength(1);
    expect(state.blocks[0]).toMatchObject({
      id: 'tool-call-1',
      status: 'complete',
      summary: '3 results',
      streams: {
        stdout: 'result line\n',
        stderr: '',
      },
    });
  });

  it('applies tool block patches to streams, metadata, child refs and terminal fields', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-1',
      phase: 'callingTool',
      blocks: [
        {
          id: 'tool-call-1',
          kind: 'tool_call',
          turnId: 'turn-2',
          toolCallId: 'call-1',
          toolName: 'spawn',
          status: 'streaming',
          input: {
            prompt: 'explore repo',
          },
          streams: {
            stdout: '',
            stderr: '',
          },
        },
      ],
      control: { ...baseControl, phase: 'callingTool' as const },
      childSummaries: [],
    };

    applyConversationEnvelope(state, {
      cursor: 'cursor-2',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'append_tool_stream',
        stream: 'stdout',
        chunk: 'child launched\n',
      },
    });
    applyConversationEnvelope(state, {
      cursor: 'cursor-3',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'replace_error',
        error: 'sub-agent launch failed',
      },
    });
    applyConversationEnvelope(state, {
      cursor: 'cursor-4',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'replace_duration',
        durationMs: 88,
      },
    });
    applyConversationEnvelope(state, {
      cursor: 'cursor-5',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'set_truncated',
        truncated: true,
      },
    });
    applyConversationEnvelope(state, {
      cursor: 'cursor-6',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'replace_metadata',
        metadata: {
          openSessionId: 'session-child-1',
          display: {
            kind: 'terminal',
            command: 'python worker.py',
          },
        },
      },
    });
    applyConversationEnvelope(state, {
      cursor: 'cursor-7',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'replace_child_ref',
        childRef: {
          agentId: 'agent-child-1',
          sessionId: 'session-root',
          subRunId: 'subrun-child-1',
          parentAgentId: 'agent-root',
          parentSubRunId: 'subrun-root',
          lineageKind: 'spawn',
          status: 'running',
          openSessionId: 'session-child-1',
        },
      },
    });

    const projection = projectConversationState(state);

    expect(state.cursor).toBe('cursor-7');
    expect(projection.messages[0]).toMatchObject({
      kind: 'toolCall',
      toolCallId: 'call-1',
      streams: {
        stdout: 'child launched\n',
        stderr: '',
      },
      error: 'sub-agent launch failed',
      durationMs: 88,
      truncated: true,
      metadata: {
        openSessionId: 'session-child-1',
      },
      childRef: {
        subRunId: 'subrun-child-1',
        openSessionId: 'session-child-1',
      },
    });
  });

  it('rehydrates from an authoritative snapshot without reintroducing sibling stream semantics', () => {
    const liveState: ConversationSnapshotState = {
      cursor: 'cursor-live-1',
      phase: 'callingTool',
      blocks: [
        {
          id: 'tool-call-1',
          kind: 'tool_call',
          turnId: 'turn-9',
          toolCallId: 'call-9',
          toolName: 'shell',
          status: 'streaming',
          input: {
            command: 'rg conversation',
          },
          streams: {
            stdout: 'conversation.ts\n',
            stderr: '',
          },
        },
      ],
      control: { ...baseControl, phase: 'callingTool' as const },
      childSummaries: [],
    };

    applyConversationEnvelope(liveState, {
      cursor: 'cursor-live-2',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'append_tool_stream',
        stream: 'stdout',
        chunk: 'conversation.test.ts\n',
      },
    });
    applyConversationEnvelope(liveState, {
      cursor: 'cursor-live-3',
      kind: 'patch_block',
      blockId: 'tool-call-1',
      patch: {
        kind: 'replace_child_ref',
        childRef: {
          agentId: 'agent-child-9',
          sessionId: 'session-root',
          subRunId: 'subrun-child-9',
          parentAgentId: 'agent-root',
          parentSubRunId: 'subrun-root',
          lineageKind: 'spawn',
          status: 'running',
          openSessionId: 'session-child-9',
        },
      },
    });

    const liveProjection = projectConversationState(liveState);

    const rehydratedState: ConversationSnapshotState = {
      cursor: 'cursor-rehydrate-1',
      phase: 'callingTool',
      blocks: [
        {
          id: 'tool-call-1',
          kind: 'tool_call',
          turnId: 'turn-9',
          toolCallId: 'call-9',
          toolName: 'shell',
          status: 'streaming',
          input: {
            command: 'rg conversation',
          },
          streams: {
            stdout: 'conversation.ts\nconversation.test.ts\n',
            stderr: '',
          },
          childRef: {
            agentId: 'agent-child-9',
            sessionId: 'session-root',
            subRunId: 'subrun-child-9',
            parentAgentId: 'agent-root',
            parentSubRunId: 'subrun-root',
            lineageKind: 'spawn',
            status: 'running',
            openSessionId: 'session-child-9',
          },
        },
      ],
      control: { ...baseControl, phase: 'callingTool' as const },
      childSummaries: [],
    };

    const rehydratedProjection = projectConversationState(rehydratedState);

    expect(liveProjection.messages).toHaveLength(1);
    expect(rehydratedProjection.messages).toHaveLength(1);
    expect(rehydratedProjection.messages[0]).toMatchObject({
      kind: 'toolCall',
      toolCallId: 'call-9',
      streams: {
        stdout: 'conversation.ts\nconversation.test.ts\n',
        stderr: '',
      },
      childRef: {
        subRunId: 'subrun-child-9',
        openSessionId: 'session-child-9',
      },
    });
    expect(rehydratedProjection.messages[0]).toMatchObject(liveProjection.messages[0]);
  });

  it('projects compact system notes with the explicit auto trigger from compactMeta', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-compact-1',
      phase: 'done',
      blocks: [
        {
          id: 'compact-1',
          kind: 'system_note',
          turnId: 'turn-compact-1',
          noteKind: 'compact',
          markdown: '压缩摘要',
          compactMeta: {
            trigger: 'auto',
            mode: 'incremental',
            instructionsPresent: false,
            fallbackUsed: false,
            retryCount: 0,
            inputUnits: 4,
            outputSummaryChars: 12,
          },
        },
      ],
      control: {
        ...baseControl,
        lastCompactMeta: {
          trigger: 'manual',
          meta: {
            mode: 'full',
            instructionsPresent: false,
            fallbackUsed: false,
            retryCount: 0,
            inputUnits: 0,
            outputSummaryChars: 0,
          },
        },
      },
      childSummaries: [],
    };

    const projection = projectConversationState(state);

    expect(projection.messages).toHaveLength(1);
    expect(projection.messages[0]).toMatchObject({
      kind: 'compact',
      trigger: 'auto',
      meta: {
        mode: 'incremental',
        inputUnits: 4,
      },
    });
  });

  it('falls back to control lastCompactMeta trigger when compact block omits it', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-compact-2',
      phase: 'done',
      blocks: [
        {
          id: 'compact-2',
          kind: 'system_note',
          turnId: 'turn-compact-2',
          noteKind: 'compact',
          markdown: '压缩摘要',
          compactMeta: {
            mode: 'full',
            instructionsPresent: false,
            fallbackUsed: false,
            retryCount: 1,
            inputUnits: 7,
            outputSummaryChars: 24,
          },
        },
      ],
      control: {
        ...baseControl,
        lastCompactMeta: {
          trigger: 'auto',
          meta: {
            mode: 'full',
            instructionsPresent: false,
            fallbackUsed: false,
            retryCount: 1,
            inputUnits: 7,
            outputSummaryChars: 24,
          },
        },
      },
      childSummaries: [],
    };

    const projection = projectConversationState(state);

    expect(projection.messages).toHaveLength(1);
    expect(projection.messages[0]).toMatchObject({
      kind: 'compact',
      trigger: 'auto',
    });
  });

  it('projects plan blocks as first-class plan messages', () => {
    const state: ConversationSnapshotState = {
      cursor: 'cursor-plan-1',
      phase: 'done',
      blocks: [
        {
          id: 'plan-1',
          kind: 'plan',
          turnId: 'turn-plan-1',
          toolCallId: 'call-plan-exit',
          eventKind: 'review_pending',
          title: 'Cleanup crates',
          planPath:
            'D:/GitObjectsOwn/Astrcode/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md',
          summary: '正在做退出前自审',
          review: {
            kind: 'final_review',
            checklist: ['Re-check assumptions against the code you already inspected.'],
          },
          blockers: {
            missingHeadings: ['## Verification'],
            invalidSections: ['session plan needs more verification detail'],
          },
        },
      ],
      control: { ...baseControl, phase: 'done' as const },
      childSummaries: [],
    };

    const projection = projectConversationState(state);

    expect(projection.messages).toHaveLength(1);
    expect(projection.messages[0]).toMatchObject({
      kind: 'plan',
      toolCallId: 'call-plan-exit',
      eventKind: 'review_pending',
      title: 'Cleanup crates',
      blockers: {
        missingHeadings: ['## Verification'],
      },
      review: {
        kind: 'final_review',
      },
    });
  });
});
