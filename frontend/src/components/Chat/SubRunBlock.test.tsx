import type { ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import type { Message, SubRunFinishMessage } from '../../types';
import SubRunBlock from './SubRunBlock';

function renderMessageRow(message: Message): ReactNode {
  return <div key={message.id}>{message.kind}</div>;
}

describe('SubRunBlock result rendering', () => {
  it('renders background running guidance and cancel entry for live sub-runs', () => {
    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-live"
        sessionId="session-1"
        title="reviewer"
        bodyMessages={[]}
        renderMessageRow={renderMessageRow}
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
        bodyMessages={[]}
        renderMessageRow={renderMessageRow}
        onCancelSubRun={async () => {}}
      />
    );

    expect(html).toContain('子会话失败');
    expect(html).toContain('子 Agent 调用模型时网络连接中断，未完成任务。');
    expect(html).toContain('HTTP request error: failed to read anthropic response stream');
    expect(html).not.toContain('传递给主会话');
  });
});
