import type React from 'react';
import type {
  ComposerOption,
  CurrentModelInfo,
  ModelOption,
  Phase,
  Project,
  Session,
} from '../../types';
import type { SubRunViewData, ThreadItem } from '../../lib/subRunView';
import TopBar from './TopBar';
import MessageList from './MessageList';
import InputBar from './InputBar';

interface ChatProps {
  project: Project | null;
  session: Session | null;
  threadItems: ThreadItem[];
  childSubRuns: SubRunViewData[];
  subRunViews: Map<string, SubRunViewData>;
  contentFingerprint: string;
  phase: Phase;
  activeSubRunPath: string[];
  activeSubRunTitle: string | null;
  activeSubRunBreadcrumbs: Array<{ subRunId: string; title: string }>;
  isSidebarOpen: boolean;
  toggleSidebar: () => void;
  onOpenSubRun: (subRunId: string) => void;
  onCloseSubRun: () => void;
  onNavigateSubRunPath: (subRunPath: string[]) => void;
  onOpenChildSession: (childSessionId: string) => void | Promise<void>;
  onSubmitPrompt: (text: string) => void | Promise<void>;
  onInterrupt: () => void | Promise<void>;
  onCancelSubRun: (sessionId: string, subRunId: string) => void | Promise<void>;
  listComposerOptions: (
    sessionId: string,
    query: string,
    signal?: AbortSignal
  ) => Promise<ComposerOption[]>;
  modelRefreshKey: number;
  getCurrentModel: () => Promise<CurrentModelInfo>;
  listAvailableModels: () => Promise<ModelOption[]>;
  setModel: (profileName: string, model: string) => Promise<void>;
}

export default function Chat({
  project,
  session,
  threadItems,
  childSubRuns,
  subRunViews,
  contentFingerprint,
  phase,
  activeSubRunPath,
  activeSubRunTitle,
  activeSubRunBreadcrumbs,
  isSidebarOpen,
  toggleSidebar,
  onOpenSubRun,
  onCloseSubRun,
  onNavigateSubRunPath,
  onOpenChildSession,
  onSubmitPrompt,
  onInterrupt,
  onCancelSubRun,
  listComposerOptions,
  modelRefreshKey,
  getCurrentModel,
  listAvailableModels,
  setModel,
}: ChatProps) {
  return (
    <div
      className="flex flex-col h-full min-h-0 min-w-0 overflow-hidden bg-panel-bg"
      style={
        {
          '--chat-content-max-width': '860px',
          '--chat-composer-max-width': 'calc(860px + 56px)',
          '--chat-content-horizontal-padding': '32px',
          '--chat-content-horizontal-padding-mobile': '16px',
          '--chat-assistant-content-offset': '44px',
          '--chat-assistant-center-shift': 'calc(44px / 2)',
          '--chat-composer-shell-padding-x': '16px',
        } as React.CSSProperties
      }
    >
      <TopBar
        projectName={project?.name ?? null}
        sessionTitle={session?.title ?? null}
        activeSubRunPath={activeSubRunPath}
        activeSubRunBreadcrumbs={activeSubRunBreadcrumbs}
        isSidebarOpen={isSidebarOpen}
        toggleSidebar={toggleSidebar}
        onCloseSubRun={onCloseSubRun}
        onNavigateSubRunPath={onNavigateSubRunPath}
      />
      <MessageList
        sessionId={session?.id ?? null}
        threadItems={threadItems}
        childSubRuns={childSubRuns}
        subRunViews={subRunViews}
        contentFingerprint={contentFingerprint}
        emptyStateText={activeSubRunPath.length > 0 ? '等待该子执行输出...' : undefined}
        onCancelSubRun={onCancelSubRun}
        onFocusSubRun={onOpenSubRun}
        onOpenChildSession={onOpenChildSession}
      />
      {activeSubRunPath.length > 0 ? (
        <div className="flex-shrink-0 px-6 pt-3.5 pb-4 border-t border-border text-text-secondary text-xs leading-relaxed bg-[linear-gradient(180deg,rgba(255,252,247,0.94)_0%,rgba(252,250,247,0.88)_100%)]">
          当前正在查看子执行
          {activeSubRunTitle ? `「${activeSubRunTitle}」` : ''}{' '}
          的内容视图。下方会单独列出下一层子执行；返回主会话后可继续输入。
        </div>
      ) : (
        <InputBar
          sessionId={session?.id ?? null}
          workingDir={project?.workingDir ?? ''}
          phase={phase}
          onSubmit={onSubmitPrompt}
          onInterrupt={onInterrupt}
          listComposerOptions={listComposerOptions}
          modelRefreshKey={modelRefreshKey}
          getCurrentModel={getCurrentModel}
          listAvailableModels={listAvailableModels}
          setModel={setModel}
        />
      )}
    </div>
  );
}
