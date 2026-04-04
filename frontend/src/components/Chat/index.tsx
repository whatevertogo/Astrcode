import type {
  ComposerOption,
  CurrentModelInfo,
  ModelOption,
  Phase,
  Project,
  Session,
} from '../../types';
import TopBar from './TopBar';
import MessageList from './MessageList';
import InputBar from './InputBar';

interface ChatProps {
  project: Project | null;
  session: Session | null;
  phase: Phase;
  isSidebarOpen: boolean;
  toggleSidebar: () => void;
  onNewSession: () => void;
  onSubmitPrompt: (text: string) => void | Promise<void>;
  onInterrupt: () => void | Promise<void>;
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
  phase,
  isSidebarOpen,
  toggleSidebar,
  onNewSession,
  onSubmitPrompt,
  onInterrupt,
  listComposerOptions,
  modelRefreshKey,
  getCurrentModel,
  listAvailableModels,
  setModel,
}: ChatProps) {
  return (
    <div className="chat">
      <TopBar
        projectName={project?.name ?? null}
        sessionTitle={session?.title ?? null}
        isSidebarOpen={isSidebarOpen}
        toggleSidebar={toggleSidebar}
        onNewSession={onNewSession}
      />
      <MessageList messages={session?.messages ?? []} />
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
    </div>
  );
}
