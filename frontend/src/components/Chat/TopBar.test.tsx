import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { ChatScreenProvider, type ChatScreenContextValue } from './ChatScreenContext';
import TopBar from './TopBar';

const baseContextValue: ChatScreenContextValue = {
  projectName: 'Astrcode',
  sessionId: 'session-1',
  sessionTitle: 'Cleanup Plan Session',
  currentModeId: 'plan',
  isChildSession: false,
  workingDir: 'D:/GitObjectsOwn/Astrcode',
  phase: 'idle',
  conversationControl: {
    phase: 'idle',
    canSubmitPrompt: true,
    canRequestCompact: true,
    compactPending: false,
    compacting: false,
    currentModeId: 'plan',
    activePlan: {
      slug: 'cleanup-crates',
      path:
        'D:/GitObjectsOwn/Astrcode/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md',
      status: 'awaiting_approval',
      title: 'Cleanup crates',
    },
    activeTasks: undefined,
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

describe('TopBar', () => {
  it('renders plan mode badge and active plan summary from authoritative control state', () => {
    const html = renderToStaticMarkup(
      <ChatScreenProvider value={baseContextValue}>
        <TopBar />
      </ChatScreenProvider>
    );

    expect(html).toContain('Astrcode');
    expect(html).toContain('Cleanup Plan Session');
    expect(html).toContain('plan');
    expect(html).toContain('当前计划 · Cleanup crates');
    expect(html).toContain('当前计划: Cleanup crates (awaiting_approval)');
  });
});
