import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { ChatScreenProvider, type ChatScreenContextValue } from './ChatScreenContext';
import ToolCallBlock from './ToolCallBlock';

const chatContextValue: ChatScreenContextValue = {
  projectName: 'Astrcode',
  sessionId: 'session-1',
  sessionTitle: 'Test Session',
  isChildSession: false,
  workingDir: 'D:/GitObjectsOwn/Astrcode',
  phase: 'idle',
  activeSubRunPath: [],
  activeSubRunTitle: null,
  activeSubRunBreadcrumbs: [],
  isSidebarOpen: true,
  toggleSidebar: () => {},
  onOpenSubRun: () => {},
  onCloseSubRun: () => {},
  onNavigateSubRunPath: () => {},
  onOpenChildSession: () => {},
  onSubmitPrompt: () => {},
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

describe('ToolCallBlock', () => {
  it('renders real tool args in the collapsed summary and grouped result output in the body', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <ToolCallBlock
          message={{
            id: 'tool-call-1',
            kind: 'toolCall',
            toolCallId: 'call-1',
            toolName: 'readFile',
            status: 'ok',
            args: {
              path: 'Cargo.toml',
              limit: 220,
            },
            output: '[workspace]',
            timestamp: Date.now(),
          }}
          streams={[
            {
              id: 'tool-stream-1',
              kind: 'toolStream',
              toolCallId: 'call-1',
              stream: 'stdout',
              status: 'ok',
              content: '[workspace]\nmembers = [\n  "crates/core"\n]\n',
              timestamp: Date.now(),
            },
          ]}
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('已运行 readFile');
    expect(html).toContain('path=&quot;Cargo.toml&quot;');
    expect(html).toContain('limit=220');
    expect(html).toContain('[workspace]');
    expect(html).toContain('调用参数');
    expect(html).toContain('max-h-[min(58vh,560px)]');
  });

  it('renders fallback result surface when no streamed output exists', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <ToolCallBlock
          message={{
            id: 'tool-call-2',
            kind: 'toolCall',
            toolCallId: 'call-2',
            toolName: 'findFiles',
            status: 'ok',
            args: {
              pattern: '*.rs',
            },
            output: '找到 12 个文件',
            timestamp: Date.now(),
          }}
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('找到 12 个文件');
    expect(html).toContain('结果');
  });

  it('renders child session navigation action when spawn metadata exposes an open session', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <ToolCallBlock
          message={{
            id: 'tool-call-3',
            kind: 'toolCall',
            toolCallId: 'call-3',
            toolName: 'spawn',
            status: 'ok',
            args: {
              description: '探索当前项目',
            },
            output: '子 Agent 已启动',
            metadata: {
              openSessionId: 'session-child-1',
              agentRef: {
                agentId: 'agent-child-1',
                subRunId: 'subrun-child-1',
                openSessionId: 'session-child-1',
              },
            },
            timestamp: Date.now(),
          }}
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('打开子会话');
  });
});
