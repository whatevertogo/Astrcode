import { createContext, useContext } from 'react';
import type { ComposerOption, CurrentModelInfo, ModelOption, Phase } from '../../types';

export interface ChatScreenContextValue {
  projectName: string | null;
  sessionId: string | null;
  sessionTitle: string | null;
  workingDir: string;
  phase: Phase;
  activeSubRunPath: string[];
  activeSubRunTitle: string | null;
  activeSubRunBreadcrumbs: Array<{ subRunId: string; title: string }>;
  isSidebarOpen: boolean;
  toggleSidebar: () => void;
  onOpenSubRun: (subRunId: string) => void | Promise<void>;
  onCloseSubRun: () => void | Promise<void>;
  onNavigateSubRunPath: (subRunPath: string[]) => void | Promise<void>;
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

const ChatScreenContext = createContext<ChatScreenContextValue | null>(null);

interface ChatScreenProviderProps {
  value: ChatScreenContextValue;
  children: React.ReactNode;
}

export function ChatScreenProvider({ value, children }: ChatScreenProviderProps) {
  return <ChatScreenContext.Provider value={value}>{children}</ChatScreenContext.Provider>;
}

export function useChatScreenContext(): ChatScreenContextValue {
  const context = useContext(ChatScreenContext);
  if (!context) {
    throw new Error('useChatScreenContext must be used within ChatScreenProvider');
  }
  return context;
}
