import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import type { ChatScreenContextValue } from './ChatScreenContext';
import { ChatScreenProvider } from './ChatScreenContext';
import MessageList from './MessageList';

const chatContextValue: ChatScreenContextValue = {
  projectName: 'Astrcode',
  sessionId: 'session-1',
  sessionTitle: 'Test Session',
  currentModeId: 'code',
  isChildSession: false,
  workingDir: 'D:/GitObjectsOwn/Astrcode',
  phase: 'idle',
  conversationControl: null,
  activeSubRunPath: [],
  activeSubRunTitle: null,
  activeSubRunBreadcrumbs: [],
  isSidebarOpen: true,
  toggleSidebar: () => {},
  onOpenSubRun: () => {},
  onCloseSubRun: () => {},
  onNavigateSubRunPath: () => {},
  onOpenChildSession: () => {},
  onForkFromTurn: () => {},
  onSubmitPrompt: () => {},
  onSwitchMode: () => {},
  onInterrupt: () => {},
  onCancelSubRun: () => {},
  listComposerOptions: () => Promise.resolve([]),
  modelRefreshKey: 0,
  getCurrentModel: () =>
    Promise.resolve({
      profileName: 'default',
      model: 'test-model',
      providerKind: 'openai',
    }),
  listAvailableModels: () => Promise.resolve([]),
  setModel: async () => {},
};

describe('MessageList', () => {
  it('keeps child-launcher tool calls visible alongside the sub-run block', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <MessageList
          threadItems={[
            {
              kind: 'message',
              message: {
                id: 'tool-call-1',
                kind: 'toolCall',
                toolCallId: 'call-1',
                toolName: 'spawn',
                status: 'ok',
                args: {
                  description: '探索当前项目',
                },
                output: '子 Agent 已启动',
                childRef: {
                  agentId: 'agent-child-1',
                  sessionId: 'session-root',
                  subRunId: 'subrun-child-1',
                  parentAgentId: 'agent-root',
                  parentSubRunId: 'subrun-root',
                  lineageKind: 'spawn',
                  status: 'running',
                  openSessionId: 'session-child-1',
                },
                streams: {
                  stdout: '',
                  stderr: '',
                },
                timestamp: Date.now(),
              },
            },
            {
              kind: 'subRun',
              subRunId: 'subrun-child-1',
            },
          ]}
          childSubRuns={[]}
          subRunViews={
            new Map([
              [
                'subrun-child-1',
                {
                  subRunId: 'subrun-child-1',
                  title: 'agent-child-1',
                  bodyMessages: [],
                  threadItems: [],
                  streamFingerprint: 'subrun-child-1',
                  childSessionId: 'session-child-1',
                  parentSubRunId: null,
                  directChildSubRunIds: [],
                  hasDescriptorLineage: false,
                },
              ],
            ])
          }
          stepProgress={{ durable: null, live: null }}
          contentFingerprint="message-list-test"
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('spawn');
    expect(html).toContain('子 Agent agent-child-1');
    expect(html).toContain('打开子会话');
  });

  it('keeps ordinary child-launcher tool calls visible when no sub-run card is present', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <MessageList
          threadItems={[
            {
              kind: 'message',
              message: {
                id: 'tool-call-1',
                kind: 'toolCall',
                toolCallId: 'call-1',
                toolName: 'spawn',
                status: 'ok',
                args: {
                  description: '探索当前项目',
                },
                output: '子 Agent 已启动',
                childRef: {
                  agentId: 'agent-child-1',
                  sessionId: 'session-root',
                  subRunId: 'subrun-child-1',
                  parentAgentId: 'agent-root',
                  parentSubRunId: 'subrun-root',
                  lineageKind: 'spawn',
                  status: 'running',
                  openSessionId: 'session-child-1',
                },
                streams: {
                  stdout: '',
                  stderr: '',
                },
                timestamp: Date.now(),
              },
            },
          ]}
          childSubRuns={[]}
          subRunViews={new Map()}
          stepProgress={{ durable: null, live: null }}
          contentFingerprint="message-list-tool-only"
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('spawn');
  });

  it('renders a subtle live-only step cursor hint at the tail of the transcript', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <MessageList
          threadItems={[]}
          childSubRuns={[]}
          subRunViews={new Map()}
          stepProgress={{
            durable: { turnId: 'turn-1', stepIndex: 1 },
            live: { turnId: 'turn-1', stepIndex: 2 },
          }}
          contentFingerprint="message-list-step-progress"
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('纯 live 增量：Step 3');
    expect(html).toContain('已 durable 到 Step 2');
  });

  it('does not render prompt metrics rows', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <MessageList
          threadItems={[
            {
              kind: 'message',
              message: {
                id: 'prompt-metrics-1',
                kind: 'promptMetrics',
                turnId: 'turn-1',
                stepIndex: 2,
                estimatedTokens: 1024,
                contextWindow: 200000,
                effectiveWindow: 180000,
                thresholdTokens: 162000,
                truncatedToolResults: 1,
                providerInputTokens: 700,
                providerOutputTokens: 64,
                cacheCreationInputTokens: 100,
                cacheReadInputTokens: 500,
                providerCacheMetricsSupported: true,
                promptCacheReuseHits: 3,
                promptCacheReuseMisses: 1,
                promptCacheUnchangedLayers: ['stable', 'inherited'],
                promptCacheDiagnostics: {
                  reasons: ['model_changed'],
                  previousCacheReadInputTokens: 12000,
                  currentCacheReadInputTokens: 4000,
                  expectedDrop: false,
                  cacheBreakDetected: true,
                },
                timestamp: Date.now(),
              },
            },
          ]}
          childSubRuns={[]}
          subRunViews={new Map()}
          stepProgress={{ durable: null, live: null }}
          contentFingerprint="message-list-prompt-metrics"
        />
      </ChatScreenProvider>
    );

    expect(html).not.toContain('Prompt 指标');
    expect(html).not.toContain('检测到 Cache Break');
    expect(html).not.toContain('未变化层 stable / inherited');
    expect(html).not.toContain('原因 模型变化');
  });

  it('hides the step cursor hint when there is no live-only tail', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <MessageList
          threadItems={[]}
          childSubRuns={[]}
          subRunViews={new Map()}
          stepProgress={{
            durable: { turnId: 'turn-1', stepIndex: 1 },
            live: null,
          }}
          contentFingerprint="message-list-step-progress-durable-only"
        />
      </ChatScreenProvider>
    );

    expect(html).not.toContain('纯 live 增量');
    expect(html).not.toContain('已 durable 到');
  });
});
