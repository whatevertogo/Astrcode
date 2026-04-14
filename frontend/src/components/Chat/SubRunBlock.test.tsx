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
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
      />
    );

    expect(html).toContain('独立子会话正在初始化；会话入口可用后即可直接打开。');
    expect(html).toContain('取消子会话');
    expect(html).toContain('思考与工具');
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
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
      />
    );

    expect(html).toContain('执行失败');
    expect(html).toContain('子 Agent 调用模型时网络连接中断，未完成任务。');
    expect(html).toContain('HTTP request error: failed to read anthropic response stream');
    expect(html).not.toContain('调用参数');
  });

  it('renders focused-view entry for sub-runs without shared-session label', () => {
    const startMessage: SubRunStartMessage = {
      id: 'subrun-start-1',
      kind: 'subRunStart',
      subRunId: 'subrun-1',
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
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        onFocusSubRun={() => {}}
      />
    );

    expect(html).toContain('查看子执行');
    expect(html).toContain('independent session');
    expect(html).not.toContain('调用参数');
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
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        onOpenChildSession={async () => {}}
      />
    );

    expect(html).toContain('打开独立会话');
    expect(html).toContain('independent session');
    expect(html).not.toContain('Object (');
  });

  it('uses projected child-session ids when lifecycle records are unavailable', () => {
    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-projected"
        sessionId="session-1"
        childSessionId="session-child-projected"
        title="explore"
        threadItems={[]}
        streamFingerprint=""
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        onOpenChildSession={async () => {}}
      />
    );

    expect(html).toContain('打开独立会话');
    expect(html).toContain('independent session');
    expect(html).toContain('独立子会话正在后台运行，请打开会话查看实时输出。');
  });

  it('renders directory-mode summary without nested stream copy', () => {
    const finishMessage: SubRunFinishMessage = {
      id: 'subrun-finish-2',
      kind: 'subRunFinish',
      subRunId: 'subrun-3',
      result: {
        status: 'completed',
        handoff: {
          summary: '完成了静态分析并整理出两个风险点。',
          findings: ['问题一', '问题二'],
          artifacts: [],
        },
      },
      stepCount: 2,
      estimatedTokens: 80,
      timestamp: Date.now(),
    };

    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-3"
        sessionId="session-1"
        title="planner"
        finishMessage={finishMessage}
        threadItems={[]}
        streamFingerprint=""
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        onFocusSubRun={() => {}}
        displayMode="directory"
      />
    );

    expect(html).toContain('进入子执行');
    expect(html).toContain('完成了静态分析并整理出两个风险点。');
    expect(html).not.toContain('思考与工具');
  });

  // 子会话视图不展示 raw JSON — 目录模式下不应出现 Object/Array 等 JSON 结构标记
  it('does not render raw JSON structures in directory-mode summary', () => {
    const finishMessage: SubRunFinishMessage = {
      id: 'subrun-finish-json',
      kind: 'subRunFinish',
      subRunId: 'subrun-json',
      result: {
        status: 'completed',
        handoff: {
          summary: '审查完成，发现两个问题。',
          findings: ['问题一', '问题二'],
          artifacts: [],
        },
      },
      stepCount: 1,
      estimatedTokens: 50,
      timestamp: Date.now(),
    };

    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-json"
        sessionId="session-1"
        title="reviewer"
        finishMessage={finishMessage}
        threadItems={[]}
        streamFingerprint=""
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        displayMode="directory"
      />
    );

    // 父视图摘要不应暴露 JSON 容器标记
    expect(html).not.toContain('Object (');
    expect(html).not.toContain('Array (');
    expect(html).toContain('审查完成，发现两个问题。');
  });

  // 成功交付的子执行应展示可读摘要而非内部状态
  it('renders completed handoff summary as readable text instead of internal state', () => {
    const finishMessage: SubRunFinishMessage = {
      id: 'subrun-finish-handoff',
      kind: 'subRunFinish',
      subRunId: 'subrun-handoff',
      result: {
        status: 'completed',
        handoff: {
          summary: '代码审查完成：所有模块通过检查。',
          findings: [],
          artifacts: [],
        },
      },
      stepCount: 3,
      estimatedTokens: 120,
      timestamp: Date.now(),
    };

    const html = renderToStaticMarkup(
      <SubRunBlock
        subRunId="subrun-handoff"
        sessionId="session-1"
        title="explorer"
        finishMessage={finishMessage}
        threadItems={[]}
        streamFingerprint=""
        hasDescriptorLineage={true}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={async () => {}}
        onOpenChildSession={async () => {}}
      />
    );

    // 应展示可读摘要
    expect(html).toContain('代码审查完成：所有模块通过检查。');
    // 不应出现内部 JSON 字段标记
    expect(html).not.toContain('"status"');
    expect(html).not.toContain('"handoff"');
  });
});
