import { describe, expect, it } from 'vitest';

import type { Message } from '../types';
import {
  buildSubRunPathView,
  buildSubRunThreadTree,
  buildSubRunView,
  listRootSubRunViews,
} from './subRunView';

const DEFAULT_RESOLVED_OVERRIDES = {
  storageMode: 'independentSession' as const,
  inheritSystemInstructions: true,
  inheritProjectInstructions: true,
  inheritWorkingDir: true,
  inheritPolicyUpperBound: true,
  inheritCancelToken: true,
  includeCompactSummary: false,
  includeRecentTail: true,
  includeRecoveryRefs: false,
  includeParentFindings: false,
};

function makeSubRunStartFixture(input: {
  id: string;
  turnId: string;
  parentTurnId: string;
  agentId: string;
  subRunId: string;
  agentProfile: string;
  depth: number;
  timestamp: number;
  parentAgentId?: string;
}): Message {
  return {
    id: input.id,
    kind: 'subRunStart',
    turnId: input.turnId,
    parentTurnId: input.parentTurnId,
    agentId: input.agentId,
    subRunId: input.subRunId,
    agentProfile: input.agentProfile,
    resolvedOverrides: { ...DEFAULT_RESOLVED_OVERRIDES },
    resolvedLimits: {
      allowedTools: ['readFile'],
    },
    timestamp: input.timestamp,
  };
}

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
        ...makeSubRunStartFixture({
          id: 'subrun-a-start',
          turnId: 'turn-root',
          parentTurnId: 'turn-root',
          agentId: 'agent-a',
          subRunId: 'subrun-a',
          agentProfile: 'planner',
          depth: 1,
          timestamp: 2,
        }),
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
        ...makeSubRunStartFixture({
          id: 'subrun-b-start',
          turnId: 'turn-a',
          parentTurnId: 'turn-a',
          agentId: 'agent-b',
          subRunId: 'subrun-b',
          parentAgentId: 'agent-a',
          agentProfile: 'coder',
          depth: 2,
          timestamp: 4,
        }),
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
        ...makeSubRunStartFixture({
          id: 'subrun-c-start',
          turnId: 'turn-b',
          parentTurnId: 'turn-b',
          agentId: 'agent-c',
          subRunId: 'subrun-c',
          parentAgentId: 'agent-b',
          agentProfile: 'reviewer',
          depth: 3,
          timestamp: 6,
        }),
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
        agentId: 'agent-a',
        subRunId: 'subrun-a',
        agentProfile: 'planner',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        agentId: 'agent-b',
        subRunId: 'subrun-b',
        agentProfile: 'coder',
        resolvedOverrides: {
          storageMode: 'independentSession',
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

  it('treats sub-runs with parentTurnId as correctly nested', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        id: 'subrun-legacy-a-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-legacy-a',
        subRunId: 'subrun-legacy-a',
        agentProfile: 'planner',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        id: 'subrun-legacy-a-assistant',
        kind: 'assistant',
        turnId: 'turn-a',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-legacy-a',
        text: 'a',
        streaming: false,
        timestamp: 3,
      },
      {
        id: 'subrun-legacy-b-start',
        kind: 'subRunStart',
        turnId: 'turn-a',
        parentTurnId: 'turn-a',
        agentId: 'agent-legacy-b',
        subRunId: 'subrun-legacy-b',
        agentProfile: 'coder',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
    ];

    const tree = buildSubRunThreadTree(messages);
    // subrun-legacy-a 是根，subrun-legacy-b 是其子（通过栈推导）
    expect(listRootSubRunViews(tree).map((view) => view.subRunId)).toEqual(['subrun-legacy-a']);
    expect(buildSubRunView(tree, 'subrun-legacy-a')?.directChildSubRunIds).toEqual([
      'subrun-legacy-b',
    ]);
  });

  it('lists root-level sub-runs in render order for a directory view', () => {
    const messages: Message[] = [
      {
        id: 'subrun-a-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        agentProfile: 'planner',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        timestamp: 1,
      },
      {
        id: 'subrun-b-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-b',
        agentProfile: 'reviewer',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
    ];

    expect(listRootSubRunViews(messages).map((view) => view.subRunId)).toEqual([
      'subrun-a',
      'subrun-b',
    ]);
  });

  it('handles mixed parentTurnId-based and stack-derived sub-runs in the same tree', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        id: 'subrun-modern-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-modern',
        subRunId: 'subrun-modern',
        agentProfile: 'planner',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        id: 'subrun-modern-assistant',
        kind: 'assistant',
        turnId: 'turn-modern',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-modern',
        agentProfile: 'planner',
        text: 'planning',
        streaming: false,
        timestamp: 3,
      },
      {
        id: 'subrun-modern-child-start',
        kind: 'subRunStart',
        turnId: 'turn-modern',
        parentTurnId: 'turn-modern',
        agentId: 'agent-modern-child',
        subRunId: 'subrun-modern-child',
        agentProfile: 'coder',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        id: 'subrun-modern-child-finish',
        kind: 'subRunFinish',
        turnId: 'turn-modern',
        parentTurnId: 'turn-modern',
        subRunId: 'subrun-modern-child',
        result: { status: 'completed' },
        stepCount: 1,
        estimatedTokens: 20,
        timestamp: 5,
      },
      {
        id: 'subrun-modern-finish',
        kind: 'subRunFinish',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-modern',
        result: { status: 'completed' },
        stepCount: 2,
        estimatedTokens: 50,
        timestamp: 6,
      },
      {
        id: 'subrun-legacy-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-legacy',
        subRunId: 'subrun-legacy',
        agentProfile: 'reviewer',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
    ];

    const tree = buildSubRunThreadTree(messages);
    const rootViews = listRootSubRunViews(tree);

    // Modern sub-run with body messages should have its child
    const modernView = buildSubRunView(tree, 'subrun-modern');
    expect(modernView?.directChildSubRunIds).toEqual(['subrun-modern-child']);

    // Both sub-runs should appear as roots (siblings at the same level)
    expect(rootViews.map((view) => view.subRunId)).toEqual(['subrun-modern', 'subrun-legacy']);

    // Legacy sub-run should have no children
    const legacyView = buildSubRunView(tree, 'subrun-legacy');
    expect(legacyView?.directChildSubRunIds).toEqual([]);
  });

  it('handles deep nesting with parentTurnId-based lineage (depth > 3)', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        id: 'subrun-l1-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-l1',
        subRunId: 'subrun-l1',
        agentProfile: 'level-1',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        id: 'subrun-l2-start',
        kind: 'subRunStart',
        turnId: 'turn-l1',
        parentTurnId: 'turn-l1',
        agentId: 'agent-l2',
        subRunId: 'subrun-l2',
        agentProfile: 'level-2',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        timestamp: 3,
      },
      {
        id: 'subrun-l3-start',
        kind: 'subRunStart',
        turnId: 'turn-l2',
        parentTurnId: 'turn-l2',
        agentId: 'agent-l3',
        subRunId: 'subrun-l3',
        agentProfile: 'level-3',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        id: 'subrun-l4-start',
        kind: 'subRunStart',
        turnId: 'turn-l3',
        parentTurnId: 'turn-l3',
        agentId: 'agent-l4',
        subRunId: 'subrun-l4',
        agentProfile: 'level-4',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        timestamp: 5,
      },
    ];

    const tree = buildSubRunThreadTree(messages);

    // Verify the entire chain is correctly linked
    expect(buildSubRunView(tree, 'subrun-l1')?.directChildSubRunIds).toEqual(['subrun-l2']);
    expect(buildSubRunView(tree, 'subrun-l2')?.directChildSubRunIds).toEqual(['subrun-l3']);
    expect(buildSubRunView(tree, 'subrun-l3')?.directChildSubRunIds).toEqual(['subrun-l4']);
    expect(buildSubRunView(tree, 'subrun-l4')?.directChildSubRunIds).toEqual([]);

    // Verify only the top-level sub-run is a root
    expect(listRootSubRunViews(tree).map((view) => view.subRunId)).toEqual(['subrun-l1']);

    // Verify path view can traverse the entire chain
    const pathView = buildSubRunPathView(tree, [
      'subrun-l1',
      'subrun-l2',
      'subrun-l3',
      'subrun-l4',
    ]);
    expect(pathView.validPath).toEqual(['subrun-l1', 'subrun-l2', 'subrun-l3', 'subrun-l4']);
    expect(pathView.views.map((view) => view.title)).toEqual([
      'level-1',
      'level-2',
      'level-3',
      'level-4',
    ]);
  });

  it('handles orphaned sub-runs when parentAgentId does not match any known agent', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        id: 'subrun-orphan-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-orphan',
        subRunId: 'subrun-orphan',
        agentProfile: 'orphan',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
    ];

    const tree = buildSubRunThreadTree(messages);

    // Orphaned sub-run should be treated as root when parent agent is not found
    expect(listRootSubRunViews(tree).map((view) => view.subRunId)).toEqual(['subrun-orphan']);

    const orphanView = buildSubRunView(tree, 'subrun-orphan');
    expect(orphanView?.parentSubRunId).toBeNull();
  });

  it('breaks self-referential parent links from notification lineage', () => {
    const messages: Message[] = [
      {
        id: 'subrun-self-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-self',
        subRunId: 'subrun-self',
        agentProfile: 'self',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        timestamp: 1,
      },
      {
        id: 'subrun-self-notification',
        kind: 'childSessionNotification',
        turnId: 'turn-root',
        subRunId: 'subrun-self',
        childRef: {
          agentId: 'agent-self',
          sessionId: 'session-child',
          subRunId: 'subrun-self',
          executionId: 'subrun-self',
          parentAgentId: 'agent-self',
          lineageKind: 'spawn',
          status: 'running',
          openSessionId: 'session-child',
        },
        notificationKind: 'started',
        status: 'running',
        summary: 'started',
        timestamp: 2,
      },
    ];

    const tree = buildSubRunThreadTree(messages);
    expect(listRootSubRunViews(tree).map((view) => view.subRunId)).toEqual(['subrun-self']);
    expect(buildSubRunView(tree, 'subrun-self')?.parentSubRunId).toBeNull();
    expect(buildSubRunView(tree, 'subrun-self')?.directChildSubRunIds).toEqual([]);
  });

  it('breaks cyclic parent links without blowing the stack', () => {
    const messages: Message[] = [
      {
        ...makeSubRunStartFixture({
          id: 'subrun-a-start',
          turnId: 'turn-root',
          parentTurnId: 'turn-root',
          agentId: 'agent-a',
          subRunId: 'subrun-a',
          agentProfile: 'planner',
          depth: 1,
          timestamp: 1,
        }),
      },
      {
        ...makeSubRunStartFixture({
          id: 'subrun-b-start',
          turnId: 'turn-a',
          parentTurnId: 'turn-a',
          agentId: 'agent-b',
          subRunId: 'subrun-b',
          agentProfile: 'reviewer',
          depth: 2,
          timestamp: 2,
        }),
      },
      {
        id: 'subrun-a-notification',
        kind: 'childSessionNotification',
        turnId: 'turn-root',
        subRunId: 'subrun-a',
        childRef: {
          agentId: 'agent-a',
          sessionId: 'session-a',
          subRunId: 'subrun-a',
          executionId: 'subrun-a',
          parentAgentId: 'agent-b',
          lineageKind: 'spawn',
          status: 'running',
          openSessionId: 'session-a',
        },
        notificationKind: 'started',
        status: 'running',
        summary: 'a started',
        timestamp: 3,
      },
      {
        id: 'subrun-b-notification',
        kind: 'childSessionNotification',
        turnId: 'turn-a',
        subRunId: 'subrun-b',
        childRef: {
          agentId: 'agent-b',
          sessionId: 'session-b',
          subRunId: 'subrun-b',
          executionId: 'subrun-b',
          parentAgentId: 'agent-a',
          lineageKind: 'spawn',
          status: 'running',
          openSessionId: 'session-b',
        },
        notificationKind: 'started',
        status: 'running',
        summary: 'b started',
        timestamp: 4,
      },
    ];

    const tree = buildSubRunThreadTree(messages);

    expect(listRootSubRunViews(tree).map((view) => view.subRunId)).toEqual(['subrun-a']);
    expect(buildSubRunView(tree, 'subrun-a')?.directChildSubRunIds).toEqual(['subrun-b']);
    expect(buildSubRunView(tree, 'subrun-b')?.parentSubRunId).toBe('subrun-a');
  });

  // 父摘要投影测试 — 确保根级子执行可作为父视图摘要卡片使用
  it('projects root sub-runs as parent summary cards with title and status', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        ...makeSubRunStartFixture({
          id: 'subrun-a-start',
          turnId: 'turn-root',
          parentTurnId: 'turn-root',
          agentId: 'agent-a',
          subRunId: 'subrun-a',
          agentProfile: 'explorer',
          depth: 1,
          timestamp: 2,
        }),
      },
      {
        id: 'subrun-a-assistant',
        kind: 'assistant',
        turnId: 'turn-a',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        agentProfile: 'explorer',
        text: 'result a',
        streaming: false,
        timestamp: 3,
      },
      {
        id: 'subrun-a-finish',
        kind: 'subRunFinish',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-a',
        result: {
          status: 'completed',
          handoff: {
            summary: '完成了文件探索',
            findings: ['发现三个风险点'],
            artifacts: [],
          },
        },
        stepCount: 2,
        estimatedTokens: 50,
        timestamp: 4,
      },
      {
        ...makeSubRunStartFixture({
          id: 'subrun-b-start',
          turnId: 'turn-root',
          parentTurnId: 'turn-root',
          agentId: 'agent-b',
          subRunId: 'subrun-b',
          agentProfile: 'reviewer',
          depth: 1,
          timestamp: 5,
        }),
      },
      {
        id: 'subrun-b-finish',
        kind: 'subRunFinish',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-b',
        result: {
          status: 'failed',
          failure: {
            code: 'transport',
            displayMessage: '连接中断',
            technicalMessage: 'HTTP timeout',
            retryable: true,
          },
        },
        stepCount: 1,
        estimatedTokens: 20,
        timestamp: 6,
      },
    ];

    const tree = buildSubRunThreadTree(messages);
    const summaryCards = listRootSubRunViews(tree);

    // 两个根级子执行都应出现在摘要列表中
    expect(summaryCards.length).toBe(2);
    expect(summaryCards.map((card) => card.subRunId)).toEqual(['subrun-a', 'subrun-b']);

    // 第一个：成功完成，摘要应可获取
    const cardOk = summaryCards[0];
    expect(cardOk.title).toBe('explorer');
    expect(cardOk.finishMessage?.result.status).toBe('completed');
    expect(cardOk.finishMessage?.result.handoff?.summary).toBe('完成了文件探索');

    // 第二个：失败，错误信息应可获取
    const cardFail = summaryCards[1];
    expect(cardFail.title).toBe('reviewer');
    expect(cardFail.finishMessage?.result.status).toBe('failed');
    expect(cardFail.finishMessage?.result.failure?.displayMessage).toBe('连接中断');
  });

  // T020: 子会话独立可查看 — 具有 childSessionId 的子执行应标记为可独立打开
  it('marks independent-session sub-runs as openable via childSessionId', () => {
    const messages: Message[] = [
      {
        id: 'root-user',
        kind: 'user',
        turnId: 'turn-root',
        text: 'start',
        timestamp: 1,
      },
      {
        id: 'subrun-independent-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-ind',
        subRunId: 'subrun-ind',
        agentProfile: 'independent-explorer',
        childSessionId: 'session-child-ind',
        resolvedOverrides: {
          storageMode: 'independentSession',
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
        id: 'subrun-ind-finish',
        kind: 'subRunFinish',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-ind',
        childSessionId: 'session-child-ind',
        result: { status: 'completed' },
        stepCount: 1,
        estimatedTokens: 30,
        timestamp: 3,
      },
    ];

    const tree = buildSubRunThreadTree(messages);
    const view = buildSubRunView(tree, 'subrun-ind');

    // 独立子会话应包含 childSessionId，前端可据此直接打开子会话
    expect(view?.childSessionId).toBe('session-child-ind');
  });

  it('recovers root sub-runs from spawn tool metadata when lifecycle events are missing', () => {
    const messages: Message[] = [
      {
        id: 'spawn-tool-call-a',
        kind: 'toolCall',
        turnId: 'turn-root',
        toolCallId: 'call-a',
        toolName: 'spawn',
        status: 'ok',
        args: { prompt: 'task-a' },
        output: 'spawn 已在后台启动。',
        metadata: {
          agentRef: {
            agentId: 'agent-1',
            subRunId: 'subrun-1',
            openSessionId: 'session-child-1',
          },
        },
        durationMs: 12,
        timestamp: 1,
      },
      {
        id: 'spawn-tool-call-b',
        kind: 'toolCall',
        turnId: 'turn-root',
        toolCallId: 'call-b',
        toolName: 'spawn',
        status: 'ok',
        args: { prompt: 'task-b' },
        output: 'spawn 已在后台启动。',
        metadata: {
          agentRef: {
            agentId: 'agent-2',
            subRunId: 'subrun-2',
            openSessionId: 'session-child-2',
          },
        },
        durationMs: 15,
        timestamp: 2,
      },
    ];

    const tree = buildSubRunThreadTree(messages);
    const rootViews = listRootSubRunViews(tree);

    expect(rootViews.map((view) => view.subRunId)).toEqual(['subrun-1', 'subrun-2']);
    expect(rootViews.map((view) => view.title)).toEqual(['agent-1', 'agent-2']);
    expect(rootViews.map((view) => view.childSessionId)).toEqual([
      'session-child-1',
      'session-child-2',
    ]);
  });

  it('recovers child-session entry ids from child notifications when lifecycle is hidden', () => {
    const messages: Message[] = [
      {
        id: 'child-notify-running',
        kind: 'childSessionNotification',
        turnId: 'turn-root',
        agentId: 'agent-child',
        parentTurnId: 'turn-root',
        agentProfile: 'explore',
        subRunId: 'subrun-child',
        childSessionId: 'session-child-hidden',
        childRef: {
          agentId: 'agent-child',
          sessionId: 'session-parent',
          subRunId: 'subrun-child',
          parentAgentId: 'agent-parent',
          lineageKind: 'spawn',
          status: 'running',
          openSessionId: 'session-child-hidden',
        },
        notificationKind: 'started',
        status: 'running',
        summary: '子会话已启动',
        timestamp: 1,
      },
    ];

    const view = buildSubRunView(messages, 'subrun-child');

    expect(view?.childSessionId).toBe('session-child-hidden');
    expect(view?.title).toBe('explore');
  });

  it('merges spawn fallback refs with real lifecycle records without duplication', () => {
    const messages: Message[] = [
      {
        id: 'spawn-tool-call-a',
        kind: 'toolCall',
        turnId: 'turn-root',
        toolCallId: 'call-a',
        toolName: 'spawn',
        status: 'ok',
        args: { prompt: 'task-a' },
        output: 'spawn 已在后台启动。',
        metadata: {
          agentRef: {
            agentId: 'agent-1',
            subRunId: 'subrun-1',
          },
        },
        durationMs: 12,
        timestamp: 1,
      },
      {
        ...makeSubRunStartFixture({
          id: 'subrun-a-start',
          turnId: 'turn-root',
          parentTurnId: 'turn-root',
          agentId: 'agent-1',
          subRunId: 'subrun-1',
          agentProfile: 'planner',
          depth: 1,
          timestamp: 2,
        }),
      },
      {
        id: 'subrun-a-assistant',
        kind: 'assistant',
        turnId: 'turn-a',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-1',
        agentProfile: 'planner',
        text: 'done',
        streaming: false,
        timestamp: 3,
      },
    ];

    const rootViews = listRootSubRunViews(messages);

    expect(rootViews).toHaveLength(1);
    expect(rootViews[0]?.subRunId).toBe('subrun-1');
    expect(rootViews[0]?.title).toBe('planner');
  });
});
