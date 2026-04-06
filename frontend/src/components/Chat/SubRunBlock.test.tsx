import type { ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import type { ThreadItem } from '../../lib/subRunView';
import type { SubRunFinishMessage, SubRunStartMessage } from '../../types';
import SubRunBlock from './SubRunBlock';

function renderThreadItems(items: ThreadItem[]): ReactNode[] {
  return items.map((item, index) =>
    item.kind === 'message' ? (
      <div key={item.message.id}>{item.message.kind}</div>
    ) : (
      <div key={`${item.subRunId}-${index}`}>subRun</div>
    )
  );
}

describe('SubRunBlock result rendering', () => {
  it('renders background running guidance and cancel entry for live sub-runs', () => {
    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-live"
        sessionId="session-1"
        title="reviewer"
        threadItems={[]}
        streamFingerprint=""
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
      />
    );

    expect(html).toContain('后台子会话已启动，可点击查看实时流。');
    expect(html).toContain('取消子会话');
  });

  it('renders failure details without parent handoff section for failed sub-runs', () => {
    const finishMessage: SubRunFinishMessage = {
      id: 'subrun-finish-1',
      kind: 'subRunFinish',
      subRunId: 'subrun-1',
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
      timestamp: Date.now(),
    };

    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-1"
        sessionId="session-1"
        title="reviewer"
        finishMessage={finishMessage}
        threadItems={[]}
        streamFingerprint=""
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
      />
    );

    expect(html).toContain('子会话失败');
    expect(html).toContain('子 Agent 调用模型时网络连接中断，未完成任务。');
    expect(html).toContain('HTTP request error: failed to read anthropic response stream');
    expect(html).not.toContain('传递给主会话');
  });

  it('renders focused-view entry for shared-session sub-runs', () => {
    const startMessage: SubRunStartMessage = {
      id: 'subrun-start-1',
      kind: 'subRunStart',
      subRunId: 'subrun-1',
      agentProfile: 'explore',
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
      timestamp: Date.now(),
    };

    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-1"
        sessionId="session-1"
        title="explore"
        startMessage={startMessage}
        threadItems={[]}
        streamFingerprint=""
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        onFocusSubRun={() => {}}
      />
    );

    expect(html).toContain('查看子执行');
  });

  it('renders child-session navigation entry for independent sub-runs', () => {
    const startMessage: SubRunStartMessage = {
      id: 'subrun-start-2',
      kind: 'subRunStart',
      subRunId: 'subrun-2',
      agentProfile: 'review',
      childSessionId: 'session-child',
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
      timestamp: Date.now(),
    };

    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-2"
        sessionId="session-1"
        title="review"
        startMessage={startMessage}
        threadItems={[]}
        streamFingerprint=""
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        onOpenChildSession={async () => {}}
      />
    );

    expect(html).toContain('打开独立会话');
  });
});
