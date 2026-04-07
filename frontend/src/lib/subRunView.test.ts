import { describe, expect, it } from 'vitest';

import type { Message } from '../types';
import {
  buildSubRunPathView,
  buildSubRunThreadTree,
  buildSubRunView,
  listRootSubRunViews,
} from './subRunView';

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
        agentId: 'agent-a',
        subRunId: 'subrun-a',
        descriptor: {
          subRunId: 'subrun-a',
          parentTurnId: 'turn-root',
          depth: 1,
        },
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
        agentId: 'agent-b',
        subRunId: 'subrun-b',
        descriptor: {
          subRunId: 'subrun-b',
          parentTurnId: 'turn-a',
          parentAgentId: 'agent-a',
          depth: 2,
        },
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
        agentId: 'agent-c',
        subRunId: 'subrun-c',
        descriptor: {
          subRunId: 'subrun-c',
          parentTurnId: 'turn-b',
          parentAgentId: 'agent-b',
          depth: 3,
        },
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
        agentId: 'agent-a',
        subRunId: 'subrun-a',
        descriptor: {
          subRunId: 'subrun-a',
          parentTurnId: 'turn-root',
          depth: 1,
        },
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
        agentId: 'agent-b',
        subRunId: 'subrun-b',
        descriptor: {
          subRunId: 'subrun-b',
          parentTurnId: 'turn-a',
          parentAgentId: 'agent-a',
          depth: 2,
        },
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

  it('treats descriptor-missing legacy sub-runs as root entries', () => {
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
    ];

    const tree = buildSubRunThreadTree(messages);
    expect(listRootSubRunViews(tree).map((view) => view.subRunId)).toEqual([
      'subrun-legacy-a',
      'subrun-legacy-b',
    ]);
    expect(buildSubRunView(tree, 'subrun-legacy-a')?.directChildSubRunIds).toEqual([]);
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
    ];

    expect(listRootSubRunViews(messages).map((view) => view.subRunId)).toEqual([
      'subrun-a',
      'subrun-b',
    ]);
  });

  it('handles mixed descriptor and legacy sub-runs in the same tree', () => {
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
        descriptor: {
          subRunId: 'subrun-modern',
          parentTurnId: 'turn-root',
          depth: 1,
        },
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
        id: 'subrun-legacy-start',
        kind: 'subRunStart',
        turnId: 'turn-root',
        parentTurnId: 'turn-root',
        agentId: 'agent-legacy',
        subRunId: 'subrun-legacy',
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
        timestamp: 3,
      },
      {
        id: 'subrun-modern-child-start',
        kind: 'subRunStart',
        turnId: 'turn-modern',
        parentTurnId: 'turn-modern',
        agentId: 'agent-modern-child',
        subRunId: 'subrun-modern-child',
        descriptor: {
          subRunId: 'subrun-modern-child',
          parentTurnId: 'turn-modern',
          parentAgentId: 'agent-modern',
          depth: 2,
        },
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
    ];

    const tree = buildSubRunThreadTree(messages);
    const rootViews = listRootSubRunViews(tree);

    // Modern sub-run with descriptor should have its child
    const modernView = buildSubRunView(tree, 'subrun-modern');
    expect(modernView?.directChildSubRunIds).toEqual(['subrun-modern-child']);

    // Legacy sub-run without descriptor should be treated as root
    expect(rootViews.map((view) => view.subRunId)).toEqual(['subrun-modern', 'subrun-legacy']);

    // Legacy sub-run should have no children (can't infer lineage)
    const legacyView = buildSubRunView(tree, 'subrun-legacy');
    expect(legacyView?.directChildSubRunIds).toEqual([]);
  });

  it('handles deep nesting with descriptor-based lineage (depth > 3)', () => {
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
        descriptor: {
          subRunId: 'subrun-l1',
          parentTurnId: 'turn-root',
          depth: 1,
        },
        agentProfile: 'level-1',
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
        id: 'subrun-l2-start',
        kind: 'subRunStart',
        turnId: 'turn-l1',
        parentTurnId: 'turn-l1',
        agentId: 'agent-l2',
        subRunId: 'subrun-l2',
        descriptor: {
          subRunId: 'subrun-l2',
          parentTurnId: 'turn-l1',
          parentAgentId: 'agent-l1',
          depth: 2,
        },
        agentProfile: 'level-2',
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
        timestamp: 3,
      },
      {
        id: 'subrun-l3-start',
        kind: 'subRunStart',
        turnId: 'turn-l2',
        parentTurnId: 'turn-l2',
        agentId: 'agent-l3',
        subRunId: 'subrun-l3',
        descriptor: {
          subRunId: 'subrun-l3',
          parentTurnId: 'turn-l2',
          parentAgentId: 'agent-l2',
          depth: 3,
        },
        agentProfile: 'level-3',
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
        id: 'subrun-l4-start',
        kind: 'subRunStart',
        turnId: 'turn-l3',
        parentTurnId: 'turn-l3',
        agentId: 'agent-l4',
        subRunId: 'subrun-l4',
        descriptor: {
          subRunId: 'subrun-l4',
          parentTurnId: 'turn-l3',
          parentAgentId: 'agent-l3',
          depth: 4,
        },
        agentProfile: 'level-4',
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
        descriptor: {
          subRunId: 'subrun-orphan',
          parentTurnId: 'turn-root',
          parentAgentId: 'agent-missing',
          depth: 2,
        },
        agentProfile: 'orphan',
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
    ];

    const tree = buildSubRunThreadTree(messages);

    // Orphaned sub-run should be treated as root when parent agent is not found
    expect(listRootSubRunViews(tree).map((view) => view.subRunId)).toEqual(['subrun-orphan']);

    const orphanView = buildSubRunView(tree, 'subrun-orphan');
    expect(orphanView?.parentSubRunId).toBeNull();
  });
});
