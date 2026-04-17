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
  it('renders real tool args in the collapsed summary and embedded stdout in the body', () => {
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
            streams: {
              stdout: '[workspace]\nmembers = [\n  "crates/core"\n]\n',
              stderr: '',
            },
            timestamp: Date.now(),
          }}
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

  it('renders fallback result surface when no embedded stream output exists', () => {
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
            streams: {
              stdout: '',
              stderr: '',
            },
            timestamp: Date.now(),
          }}
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('找到 12 个文件');
    expect(html).toContain('结果');
  });

  it('renders child session navigation action from explicit child ref', () => {
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
          }}
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('打开子会话');
  });

  it('renders embedded stdout/stderr sections and failure pills for failed tools', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <ToolCallBlock
          message={{
            id: 'tool-call-4',
            kind: 'toolCall',
            toolCallId: 'call-4',
            toolName: 'shell',
            status: 'fail',
            args: {
              command: 'rg missing-symbol',
            },
            error: 'rg exited with code 2',
            durationMs: 88,
            truncated: true,
            streams: {
              stdout: 'searching workspace\n',
              stderr: 'rg: missing-symbol: The system cannot find the file specified.\n',
            },
            timestamp: Date.now(),
          }}
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('已运行 shell');
    expect(html).toContain('工具结果');
    expect(html).toContain('错误输出');
    expect(html).toContain('searching workspace');
    expect(html).toContain('missing-symbol');
    expect(html).toContain('88 ms');
    expect(html).toContain('truncated');
    expect(html).toContain('失败');
  });

  it('renders explicit tool error even when stdout is present', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={chatContextValue}>
        <ToolCallBlock
          message={{
            id: 'tool-call-5',
            kind: 'toolCall',
            toolCallId: 'call-5',
            toolName: 'shell',
            status: 'fail',
            args: {
              command: 'cargo test',
            },
            error: 'command exited with code 101',
            streams: {
              stdout: 'Compiling crate...\n',
              stderr: '',
            },
            timestamp: Date.now(),
          }}
        />
      </ChatScreenProvider>
    );

    expect(html).toContain('Compiling crate...');
    expect(html).toContain('command exited with code 101');
    expect(html).toContain('错误');
  });
});
