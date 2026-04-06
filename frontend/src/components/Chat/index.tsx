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
import styles from './Chat.module.css';

interface ChatProps {
  project: Project | null;
  session: Session | null;
  threadItems: ThreadItem[];
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
    <div className={styles.chat}>
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
        subRunViews={subRunViews}
        contentFingerprint={contentFingerprint}
        emptyStateText={activeSubRunPath.length > 0 ? '等待该子执行输出...' : undefined}
        onCancelSubRun={onCancelSubRun}
        onFocusSubRun={onOpenSubRun}
        onOpenChildSession={onOpenChildSession}
      />
      {activeSubRunPath.length > 0 ? (
        <div className={styles.subRunFocusHint}>
          当前正在查看子执行
          {activeSubRunTitle ? `「${activeSubRunTitle}」` : ''} 的过滤视图。返回主会话后可继续输入。
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
