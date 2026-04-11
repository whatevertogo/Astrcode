import { describe, expect, it } from 'vitest';

import { normalizeAgentEvent } from './agentEvent';

function makeChildSessionSubRunStartedFixture() {
  return {
    protocolVersion: 1,
    event: 'subRunStarted',
    data: {
      turn_id: 'turn-parent',
      sub_run_id: 'sub-1',
      parent_turn_id: 'turn-parent',
      parent_agent_id: 'agent-parent',
      tool_call_id: 'call-1',
      resolved_overrides: {
        storage_mode: 'independentSession',
        inherit_system_instructions: true,
        inherit_project_instructions: true,
        inherit_working_dir: true,
        inherit_policy_upper_bound: true,
        inherit_cancel_token: true,
        include_compact_summary: true,
        include_recent_tail: true,
        include_recovery_refs: false,
        include_parent_findings: false,
      },
      resolved_limits: {
        allowed_tools: ['readFile', 'grep'],
      },
    },
  };
}

function makeChildSessionNotificationFixture() {
  return {
    protocolVersion: 1,
    event: 'childSessionNotification',
    data: {
      turn_id: 'turn-parent',
      agent_id: 'agent-parent',
      child_session_id: 'session-child-1',
      child_ref: {
        agent_id: 'agent-child',
        session_id: 'session-parent',
        sub_run_id: 'subrun-child',
        lineage_kind: 'spawn',
        status: 'completed',
        open_session_id: 'session-child-1',
      },
      kind: 'delivered',
      summary: '子会话已完成',
      status: 'completed',
      open_session_id: 'session-child-1',
      source_tool_call_id: 'call-child',
      final_reply_excerpt: '最终摘要',
    },
  };
}

function makeLegacyUnsupportedErrorFixture() {
  return {
    protocolVersion: 1,
    event: 'error',
    data: {
      turn_id: 'turn-legacy',
      code: 'unsupported_legacy_shared_history',
      message: '该会话使用旧共享历史结构，必须升级后才能继续查看。',
      child_session_id: 'session-child-legacy',
    },
  };
}

describe('normalizeAgentEvent protocol gate', () => {
  it('rejects payload when protocolVersion is missing', () => {
    const normalized = normalizeAgentEvent({
      event: 'phaseChanged',
      data: { phase: 'idle', turnId: null },
    });

    expect(normalized.event).toBe('error');
    if (normalized.event === 'error') {
      expect(normalized.data.code).toBe('invalid_agent_event');
      expect(normalized.data.message).toContain('protocolVersion field is missing');
    }
  });

  it('rejects payload when protocolVersion is incompatible', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 2,
      event: 'phaseChanged',
      data: { phase: 'idle', turnId: null },
    });

    expect(normalized.event).toBe('error');
    if (normalized.event === 'error') {
      expect(normalized.data.message).toContain('unsupported protocolVersion');
    }
  });

  it('accepts payload when protocolVersion is 1', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'modelDelta',
      data: { turnId: 'turn-1', delta: 'hello' },
    });

    expect(normalized).toEqual({
      event: 'modelDelta',
      data: { turnId: 'turn-1', delta: 'hello' },
    });
  });

  it('accepts userMessage payloads', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'userMessage',
      data: { turn_id: 'turn-user', content: 'hello' },
    });

    expect(normalized).toEqual({
      event: 'userMessage',
      data: { turnId: 'turn-user', content: 'hello' },
    });
  });

  it('accepts assistantMessage when content is empty but reasoning exists', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'assistantMessage',
      data: {
        turn_id: 'turn-2',
        content: '',
        reasoning_content: '先检查相关文件。',
      },
    });

    expect(normalized).toEqual({
      event: 'assistantMessage',
      data: {
        turnId: 'turn-2',
        content: '',
        reasoningContent: '先检查相关文件。',
      },
    });
  });

  it('accepts assistantMessage when both content and reasoning are empty', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'assistantMessage',
      data: {
        turn_id: 'turn-3',
        content: '',
      },
    });

    expect(normalized).toEqual({
      event: 'assistantMessage',
      data: {
        turnId: 'turn-3',
        content: '',
      },
    });
  });

  it('accepts empty string deltas for streaming events', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'modelDelta',
      data: {
        turnId: 'turn-empty',
        delta: '',
      },
    });

    expect(normalized).toEqual({
      event: 'modelDelta',
      data: {
        turnId: 'turn-empty',
        delta: '',
      },
    });
  });

  it('accepts toolCallDelta payloads and normalizes stream names', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'toolCallDelta',
      data: {
        turn_id: 'turn-shell',
        tool_call_id: 'call-1',
        tool_name: 'shell',
        stream: 'stderr',
        delta: 'boom\\n',
      },
    });

    expect(normalized).toEqual({
      event: 'toolCallDelta',
      data: {
        turnId: 'turn-shell',
        toolCallId: 'call-1',
        toolName: 'shell',
        stream: 'stderr',
        delta: 'boom\\n',
      },
    });
  });

  it('accepts compactApplied payloads', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'compactApplied',
      data: {
        turn_id: null,
        trigger: 'manual',
        summary: '保留最近两轮上下文',
        preserved_recent_turns: 2,
      },
    });

    expect(normalized).toEqual({
      event: 'compactApplied',
      data: {
        turnId: null,
        trigger: 'manual',
        summary: '保留最近两轮上下文',
        preservedRecentTurns: 2,
      },
    });
  });

  it('accepts promptMetrics payloads with cache fields', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'promptMetrics',
      data: {
        turn_id: 'turn-metrics',
        step_index: 1,
        estimated_tokens: 4096,
        context_window: 200000,
        effective_window: 180000,
        threshold_tokens: 162000,
        truncated_tool_results: 2,
        provider_input_tokens: 3200,
        provider_output_tokens: 120,
        cache_creation_input_tokens: 2800,
        cache_read_input_tokens: 2500,
        provider_cache_metrics_supported: true,
        prompt_cache_reuse_hits: 3,
        prompt_cache_reuse_misses: 1,
      },
    });

    expect(normalized).toEqual({
      event: 'promptMetrics',
      data: {
        turnId: 'turn-metrics',
        stepIndex: 1,
        estimatedTokens: 4096,
        contextWindow: 200000,
        effectiveWindow: 180000,
        thresholdTokens: 162000,
        truncatedToolResults: 2,
        providerInputTokens: 3200,
        providerOutputTokens: 120,
        cacheCreationInputTokens: 2800,
        cacheReadInputTokens: 2500,
        providerCacheMetricsSupported: true,
        promptCacheReuseHits: 3,
        promptCacheReuseMisses: 1,
      },
    });
  });

  it('accepts childSessionNotification payloads with stable child-session entry fields', () => {
    const normalized = normalizeAgentEvent(makeChildSessionNotificationFixture());

    expect(normalized).toEqual({
      event: 'childSessionNotification',
      data: {
        turnId: 'turn-parent',
        agentId: 'agent-parent',
        childSessionId: 'session-child-1',
        childRef: {
          agentId: 'agent-child',
          sessionId: 'session-parent',
          subRunId: 'subrun-child',
          executionId: 'subrun-child',
          parentAgentId: undefined,
          lineageKind: 'spawn',
          status: 'idle',
          openSessionId: 'session-child-1',
        },
        kind: 'delivered',
        summary: '子会话已完成',
        status: 'idle',
        sourceToolCallId: 'call-child',
        finalReplyExcerpt: '最终摘要',
      },
    });
  });

  it('accepts unsupported legacy error payloads for explicit UI display', () => {
    const normalized = normalizeAgentEvent(makeLegacyUnsupportedErrorFixture());

    expect(normalized).toEqual({
      event: 'error',
      data: {
        turnId: 'turn-legacy',
        code: 'unsupported_legacy_shared_history',
        message: '该会话使用旧共享历史结构，必须升级后才能继续查看。',
        childSessionId: 'session-child-legacy',
      },
    });
  });

  it('accepts failed subRunFinished payloads with structured failure details', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'subRunFinished',
      data: {
        turn_id: 'turn-subrun',
        result: {
          status: 'failed',
          failure: {
            code: 'transport',
            display_message: '子 Agent 调用模型时网络连接中断，未完成任务。',
            technical_message: 'HTTP request error: failed to read anthropic response stream',
            retryable: true,
          },
        },
        step_count: 3,
        estimated_tokens: 120,
      },
    });

    expect(normalized).toEqual({
      event: 'subRunFinished',
      data: {
        turnId: 'turn-subrun',
        result: {
          status: 'failed',
          failure: {
            code: 'transport',
            displayMessage: '子 Agent 调用模型时网络连接中断，未完成任务。',
            technicalMessage: 'HTTP request error: failed to read anthropic response stream',
            retryable: true,
          },
        },
        stepCount: 3,
        estimatedTokens: 120,
      },
    });
  });

  it('normalizes subRunStarted payloads with toolCallId and snake_case overrides', () => {
    const normalized = normalizeAgentEvent(makeChildSessionSubRunStartedFixture());

    expect(normalized).toEqual({
      event: 'subRunStarted',
      data: {
        turnId: 'turn-parent',
        subRunId: 'sub-1',
        executionId: 'sub-1',
        parentTurnId: 'turn-parent',
        toolCallId: 'call-1',
        resolvedOverrides: {
          storageMode: 'independentSession',
          inheritSystemInstructions: true,
          inheritProjectInstructions: true,
          inheritWorkingDir: true,
          inheritPolicyUpperBound: true,
          inheritCancelToken: true,
          includeCompactSummary: true,
          includeRecentTail: true,
          includeRecoveryRefs: false,
          includeParentFindings: false,
        },
        resolvedLimits: {
          allowedTools: ['readFile', 'grep'],
        },
      },
    });
  });

  it('normalizes legacy subRunStarted payloads', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'subRunStarted',
      data: {
        turn_id: 'turn-legacy',
        parent_turn_id: 'turn-legacy',
        sub_run_id: 'sub-legacy',
        resolved_overrides: {
          storage_mode: 'independentSession',
          inherit_system_instructions: true,
          inherit_project_instructions: true,
          inherit_working_dir: true,
          inherit_policy_upper_bound: true,
          inherit_cancel_token: true,
          include_compact_summary: false,
          include_recent_tail: true,
          include_recovery_refs: false,
          include_parent_findings: false,
        },
        resolved_limits: {
          allowed_tools: ['readFile'],
        },
      },
    });

    expect(normalized).toMatchObject({
      event: 'subRunStarted',
      data: {
        resolvedOverrides: {
          storageMode: 'independentSession',
        },
        resolvedLimits: {
          allowedTools: ['readFile'],
        },
      },
    });
  });

  it('normalizes legacy subRunFinished payloads', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'subRunFinished',
      data: {
        turn_id: 'turn-legacy',
        parent_turn_id: 'turn-legacy',
        sub_run_id: 'sub-legacy',
        result: {
          status: 'completed',
        },
        step_count: 1,
        estimated_tokens: 20,
      },
    });

    expect(normalized).toMatchObject({
      event: 'subRunFinished',
      data: {
        result: {
          status: 'completed',
        },
        stepCount: 1,
        estimatedTokens: 20,
      },
    });
  });

  describe('executionId preservation', () => {
    it('preserves executionId from subRunStarted event', () => {
      const normalized = normalizeAgentEvent({
        protocolVersion: 1,
        event: 'subRunStarted',
        data: {
          turn_id: 'turn-1',
          agent_id: 'agent-1',
          agent_profile: 'profile-1',
          execution_id: 'exec-1',
          tool_call_id: 'call-1',
          resolved_overrides: {
            storage_mode: 'independentSession',
            inherit_system_instructions: true,
            inherit_project_instructions: true,
            inherit_working_dir: true,
            inherit_policy_upper_bound: true,
            inherit_cancel_token: true,
            include_compact_summary: false,
            include_recent_tail: false,
            include_recovery_refs: false,
            include_parent_findings: false,
          },
          resolved_limits: {
            allowed_tools: [],
          },
        },
      });

      expect(normalized).toMatchObject({
        event: 'subRunStarted',
        data: {
          executionId: 'exec-1',
        },
      });
    });

    it('preserves executionId from userMessage event', () => {
      const normalized = normalizeAgentEvent({
        protocolVersion: 1,
        event: 'userMessage',
        data: {
          turn_id: 'turn-1',
          content: 'hello',
          execution_id: 'exec-1',
        },
      });

      expect(normalized).toMatchObject({
        event: 'userMessage',
        data: {
          executionId: 'exec-1',
        },
      });
    });
  });
});
