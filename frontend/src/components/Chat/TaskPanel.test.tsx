import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { ChatScreenProvider, type ChatScreenContextValue } from './ChatScreenContext';
import TaskPanel from './TaskPanel';

const contextValueWithTasks: ChatScreenContextValue = {
  projectName: 'Astrcode',
  sessionId: 'session-1',
  sessionTitle: 'Cleanup Plan Session',
  currentModeId: 'plan',
  isChildSession: false,
  workingDir: 'D:/GitObjectsOwn/Astrcode',
  phase: 'callingTool',
  conversationControl: {
    phase: 'callingTool',
    canSubmitPrompt: false,
    canRequestCompact: true,
    compactPending: false,
    compacting: false,
    currentModeId: 'plan',
    activePlan: {
      slug: 'cleanup-crates',
      path: 'D:/GitObjectsOwn/Astrcode/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md',
      status: 'draft',
      title: 'Cleanup crates',
    },
    activeTasks: [
      {
        content: '梳理受影响模块',
        status: 'in_progress',
        activeForm: '正在梳理受影响模块',
      },
      {
        content: '补齐验证矩阵',
        status: 'pending',
      },
      {
        content: '整理退出 plan mode 说明',
        status: 'completed',
      },
    ],
  },
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

describe('TaskPanel', () => {
  it('renders authoritative task progress summary and per-task statuses', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={contextValueWithTasks}>
        <TaskPanel />
      </ChatScreenProvider>
    );

    expect(html).toContain('TASKS');
    expect(html).toContain('当前执行 · 正在梳理受影响模块');
    expect(html).toContain('待处理 1');
    expect(html).toContain('已完成 1');
    expect(html).toContain('总计 3');
    expect(html).toContain('梳理受影响模块');
    expect(html).toContain('进行中');
    expect(html).toContain('补齐验证矩阵');
    expect(html).toContain('待处理');
    expect(html).toContain('整理退出 plan mode 说明');
    expect(html).toContain('已完成');
  });
});
