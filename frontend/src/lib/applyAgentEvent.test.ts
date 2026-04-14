import { describe, expect, it } from 'vitest';

import { reducer, makeInitialState } from '../store/reducer';
import type { AgentEventPayload, AppState, Phase } from '../types';
import { createEmptySubRunThreadTree } from './subRunView';
import { applyAgentEvent } from './applyAgentEvent';

function buildState(): AppState {
  return {
    ...makeInitialState(),
    projects: [
      {
        id: 'project-1',
        name: 'project',
        workingDir: '',
        isExpanded: true,
        sessions: [
          {
            id: 'session-parent',
            projectId: 'project-1',
            title: 'parent',
            createdAt: 0,
            messages: [],
            subRunThreadTree: createEmptySubRunThreadTree(),
          },
          {
            id: 'session-active',
            projectId: 'project-1',
            title: 'active',
            createdAt: 0,
            messages: [],
            subRunThreadTree: createEmptySubRunThreadTree(),
          },
        ],
      },
    ],
    activeProjectId: 'project-1',
    activeSessionId: 'session-active',
  };
}

describe('applyAgentEvent session routing', () => {
  it('routes sub-run lifecycle events by turn ownership instead of the active session', () => {
    let state = buildState();
    const context = {
      activeSessionIdRef: { current: 'session-active' as string | null },
      pendingSubmitSessionRef: { current: [] as string[] },
      turnSessionMapRef: { current: { 'turn-parent': 'session-parent' } },
      phaseRef: { current: 'idle' as Phase },
      dispatch: (action: Parameters<typeof reducer>[1]) => {
        state = reducer(state, action);
      },
    };
    const started: AgentEventPayload = {
      event: 'subRunStarted',
      data: {
        turnId: 'turn-parent',
        subRunId: 'subrun-1',
        agentId: 'agent-child',
        agentProfile: 'explore',
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
      },
    };
    const finished: AgentEventPayload = {
      event: 'subRunFinished',
      data: {
        turnId: 'turn-parent',
        subRunId: 'subrun-1',
        agentId: 'agent-child',
        result: {
          status: 'completed',
          handoff: {
            summary: 'child finished',
            findings: [],
            artifacts: [],
          },
        },
        stepCount: 1,
        estimatedTokens: 32,
      },
    };
    const notification: AgentEventPayload = {
      event: 'childSessionNotification',
      data: {
        turnId: 'turn-parent',
        subRunId: 'subrun-1',
        agentId: 'agent-child',
        childRef: {
          agentId: 'agent-child',
          sessionId: 'session-parent',
          subRunId: 'subrun-1',
          executionId: 'execution-1',
          lineageKind: 'spawn',
          status: 'idle',
          openSessionId: 'session-child',
        },
        kind: 'delivered',
        summary: 'child delivered',
        status: 'idle',
        finalReplyExcerpt: 'final excerpt',
      },
    };

    applyAgentEvent(context, started);
    applyAgentEvent(context, finished);
    applyAgentEvent(context, notification);

    const parentSession = state.projects[0]?.sessions.find(
      (session) => session.id === 'session-parent'
    );
    const activeSession = state.projects[0]?.sessions.find(
      (session) => session.id === 'session-active'
    );

    expect(parentSession?.messages.map((message) => message.kind)).toEqual([
      'subRunStart',
      'subRunFinish',
      'childSessionNotification',
    ]);
    expect(activeSession?.messages).toHaveLength(0);
  });
});
