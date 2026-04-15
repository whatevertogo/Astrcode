import { describe, expect, it } from 'vitest';

import type { ConversationSnapshotState } from './conversation';
import { projectConversationState } from './conversation';

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
    expect(projection.messageTree.rootThreadItems).toHaveLength(2);
  });

  it('preserves tool call and tool stream blocks as separate realtime messages', () => {
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
        },
        {
          id: 'tool-stream-1',
          kind: 'tool_stream',
          turnId: 'turn-2',
          parentToolCallId: 'call-1',
          stream: 'stdout',
          status: 'streaming',
          content: 'opening page\n',
        },
      ],
      childSummaries: [],
    };

    const projection = projectConversationState(state);

    expect(projection.messages).toHaveLength(2);
    expect(projection.messages[0]).toMatchObject({
      kind: 'toolCall',
      toolCallId: 'call-1',
      toolName: 'web',
      status: 'running',
      output: undefined,
    });
    expect(projection.messages[1]).toMatchObject({
      kind: 'toolStream',
      toolCallId: 'call-1',
      stream: 'stdout',
      status: 'running',
      content: 'opening page\n',
    });
    expect(projection.messageTree.rootThreadItems).toHaveLength(2);
  });
});
