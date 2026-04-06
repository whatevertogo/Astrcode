import { describe, expect, it } from 'vitest';

import type { Message } from '../types';
import { buildSubRunPathView, buildSubRunThreadTree, buildSubRunView } from './subRunView';

describe('buildSubRunView', () => {
  it('extracts lifecycle and direct-child messages for a nested sub-run view', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        id: 'subrun-a-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        agentProfile: 'planner',
        resolvedOverrides: {
          storageMode: 'sharedSession',
          inheritSystemInstructions: true,
          inheritProjectInstructions: true,
          inheritWorkingDir: true,
          inheritPolicyUpperBound: true,
          inheritCancelToken: true,
          includeCompactSummary: false,
          includeRecentTail: true,
          includeRecoveryRefs: false,
          includeParentFindings: false,
        },
        resolvedLimits: {
          allowedTools: ['readFile'],
        },
        timestamp: 2,
      },
      {
        id: 'subrun-a-assistant-1',
        kind: 'assistant',
        turnId: 'turn-a',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        agentProfile: 'planner',
        text: 'a-1',
        streaming: false,
        timestamp: 3,
      },
      {
        id: 'subrun-b-start',
        kind: 'subRunStart',
        turnId: 'turn-a',
        parentTurnId: 'turn-a',
        subRunId: 'subrun-b',
        agentProfile: 'coder',
        resolvedOverrides: {
          storageMode: 'sharedSession',
          inheritSystemInstructions: true,
          inheritProjectInstructions: true,
          inheritWorkingDir: true,
          inheritPolicyUpperBound: true,
          inheritCancelToken: true,
          includeCompactSummary: false,
          includeRecentTail: true,
          includeRecoveryRefs: false,
          includeParentFindings: false,
        },
        resolvedLimits: {
          allowedTools: ['readFile'],
        },
        timestamp: 4,
      },
      {
        id: 'subrun-b-assistant-1',
        kind: 'assistant',
        turnId: 'turn-b',
        parentTurnId: 'turn-a',
        subRunId: 'subrun-b',
        agentProfile: 'coder',
        text: 'b-1',
        streaming: false,
        timestamp: 5,
      },
      {
        id: 'subrun-c-start',
        kind: 'subRunStart',
        turnId: 'turn-b',
        parentTurnId: 'turn-b',
        subRunId: 'subrun-c',
        agentProfile: 'reviewer',
        resolvedOverrides: {
          storageMode: 'sharedSession',
          inheritSystemInstructions: true,
          inheritProjectInstructions: true,
          inheritWorkingDir: true,
          inheritPolicyUpperBound: true,
          inheritCancelToken: true,
          includeCompactSummary: false,
          includeRecentTail: true,
          includeRecoveryRefs: false,
          includeParentFindings: false,
        },
        resolvedLimits: {
          allowedTools: ['readFile'],
        },
        timestamp: 6,
      },
      {
        id: 'subrun-c-assistant-1',
        kind: 'assistant',
        turnId: 'turn-c',
        parentTurnId: 'turn-b',
        subRunId: 'subrun-c',
        agentProfile: 'reviewer',
        text: 'c-1',
        streaming: false,
        timestamp: 7,
      },
      {
        id: 'subrun-c-finish',
        kind: 'subRunFinish',
        turnId: 'turn-b',
        parentTurnId: 'turn-b',
        subRunId: 'subrun-c',
        result: { status: 'completed' },
        stepCount: 1,
        estimatedTokens: 10,
        timestamp: 8,
      },
      {
        id: 'subrun-b-assistant-2',
        kind: 'assistant',
        turnId: 'turn-b',
        parentTurnId: 'turn-a',
        subRunId: 'subrun-b',
        agentProfile: 'coder',
        text: 'b-2',
        streaming: false,
        timestamp: 9,
      },
      {
        id: 'subrun-b-finish',
        kind: 'subRunFinish',
        turnId: 'turn-a',
        parentTurnId: 'turn-a',
        subRunId: 'subrun-b',
        result: { status: 'completed' },
        stepCount: 2,
        estimatedTokens: 20,
        timestamp: 10,
      },
      {
        id: 'subrun-a-assistant-2',
        kind: 'assistant',
        turnId: 'turn-a',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        agentProfile: 'planner',
        text: 'a-2',
        streaming: false,
        timestamp: 11,
      },
      {
        id: 'subrun-a-finish',
        kind: 'subRunFinish',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        result: { status: 'completed' },
        stepCount: 3,
        estimatedTokens: 30,
        timestamp: 12,
      },
    ];

    const tree = buildSubRunThreadTree(messages);
    const view = buildSubRunView(tree, 'subrun-a');
    expect(view).not.toBeNull();
    expect(view?.title).toBe('planner');
    expect(view?.directChildSubRunIds).toEqual(['subrun-b']);
    expect(view?.bodyMessages.map((message) => message.id)).toEqual([
      'subrun-a-assistant-1',
      'subrun-a-assistant-2',
    ]);
    expect(tree.rootThreadItems).toEqual([
      {
        kind: 'message',
        message: messages[0],
      },
      {
        kind: 'subRun',
        subRunId: 'subrun-a',
      },
    ]);
    expect(view?.threadItems).toEqual([
      {
        kind: 'message',
        message: messages[2],
      },
      {
        kind: 'subRun',
        subRunId: 'subrun-b',
      },
      {
        kind: 'message',
        message: messages[10],
      },
    ]);
  });

  it('builds a validated breadcrumb path for nested sub-runs', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        id: 'subrun-a-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        agentProfile: 'planner',
        resolvedOverrides: {
          storageMode: 'sharedSession',
          inheritSystemInstructions: true,
          inheritProjectInstructions: true,
          inheritWorkingDir: true,
          inheritPolicyUpperBound: true,
          inheritCancelToken: true,
          includeCompactSummary: false,
          includeRecentTail: true,
          includeRecoveryRefs: false,
          includeParentFindings: false,
        },
        resolvedLimits: {
          allowedTools: ['readFile'],
        },
        timestamp: 2,
      },
      {
        id: 'subrun-a-assistant',
        kind: 'assistant',
        turnId: 'turn-a',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        text: 'a',
        streaming: false,
        timestamp: 3,
      },
      {
        id: 'subrun-b-start',
        kind: 'subRunStart',
        turnId: 'turn-a',
        parentTurnId: 'turn-a',
        subRunId: 'subrun-b',
        agentProfile: 'coder',
        resolvedOverrides: {
          storageMode: 'sharedSession',
          inheritSystemInstructions: true,
          inheritProjectInstructions: true,
          inheritWorkingDir: true,
          inheritPolicyUpperBound: true,
          inheritCancelToken: true,
          includeCompactSummary: false,
          includeRecentTail: true,
          includeRecoveryRefs: false,
          includeParentFindings: false,
        },
        resolvedLimits: {
          allowedTools: ['readFile'],
        },
        timestamp: 4,
      },
      {
        id: 'subrun-b-assistant',
        kind: 'assistant',
        turnId: 'turn-b',
        parentTurnId: 'turn-a',
        subRunId: 'subrun-b',
        text: 'b',
        streaming: false,
        timestamp: 5,
      },
    ];

    const tree = buildSubRunThreadTree(messages);
    const pathView = buildSubRunPathView(tree, ['subrun-a', 'subrun-b', 'missing']);
    expect(pathView.validPath).toEqual(['subrun-a', 'subrun-b']);
    expect(pathView.views.map((view) => view.title)).toEqual(['planner', 'coder']);
    expect(pathView.activeView?.subRunId).toBe('subrun-b');
  });

  it('returns null when the sub-run does not exist', () => {
    expect(buildSubRunView([], 'missing')).toBeNull();
  });
});
