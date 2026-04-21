import { describe, expect, it } from 'vitest';

import type { PromptMetricsMessage, ThreadItem } from '../../types';
import { resolvePromptMetricsAttachments } from './promptMetricsAttachments';

function assistant(
  id: string,
  turnId: string,
  stepIndex?: number
): Extract<ThreadItem, { kind: 'message' }> {
  return {
    kind: 'message',
    message: {
      id,
      kind: 'assistant',
      turnId,
      stepIndex,
      text: 'assistant',
      streaming: false,
      timestamp: Date.now(),
    },
  };
}

function promptMetrics(
  id: string,
  stepIndex: number,
  turnId = 'turn-1'
): PromptMetricsMessage & { kind: 'promptMetrics' } {
  return {
    id,
    kind: 'promptMetrics',
    turnId,
    stepIndex,
    estimatedTokens: 512,
    contextWindow: 200_000,
    effectiveWindow: 180_000,
    thresholdTokens: 162_000,
    truncatedToolResults: 0,
    timestamp: Date.now(),
  };
}

function metricsItem(
  id: string,
  stepIndex: number,
  turnId = 'turn-1'
): Extract<ThreadItem, { kind: 'message' }> {
  return {
    kind: 'message',
    message: promptMetrics(id, stepIndex, turnId),
  };
}

function toolCall(id: string, turnId: string): Extract<ThreadItem, { kind: 'message' }> {
  return {
    kind: 'message',
    message: {
      id,
      kind: 'toolCall',
      turnId,
      toolCallId: `${id}-call`,
      toolName: 'readFile',
      args: '{}',
      status: 'ok',
      output: 'done',
      timestamp: Date.now(),
    },
  };
}

describe('resolvePromptMetricsAttachments', () => {
  it('attaches prompt metrics to the assistant with the same step index', () => {
    const items: ThreadItem[] = [
      metricsItem('metrics-1', 1),
      toolCall('tool-1', 'turn-1'),
      assistant('assistant-1', 'turn-1', 1),
    ];

    const attachments = resolvePromptMetricsAttachments(items);

    expect(attachments.get('assistant-1')?.id).toBe('metrics-1');
  });

  it('falls back to positional attachment when no explicit step index is available', () => {
    const items: ThreadItem[] = [
      assistant('assistant-1', 'turn-1'),
      toolCall('tool-1', 'turn-1'),
      metricsItem('metrics-1', 1),
    ];

    const attachments = resolvePromptMetricsAttachments(items);

    expect(attachments.get('assistant-1')?.id).toBe('metrics-1');
  });
});
