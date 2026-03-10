import React from 'react';
import type { CurrentModelInfo, ModelOption, Phase, Project, Session } from '../../types';
import TopBar from './TopBar';
import MessageList from './MessageList';
import InputBar from './InputBar';
import styles from './Chat.module.css';

interface ChatProps {
  project: Project | null;
  session: Session | null;
  phase: Phase;
  onNewSession: () => void;
  onSubmitPrompt: (text: string) => void | Promise<void>;
  onInterrupt: () => void | Promise<void>;
  modelRefreshKey: number;
  getCurrentModel: () => Promise<CurrentModelInfo>;
  listAvailableModels: () => Promise<ModelOption[]>;
  setModel: (profileName: string, model: string) => Promise<void>;
}

export default function Chat({
  project,
  session,
  phase,
  onNewSession,
  onSubmitPrompt,
  onInterrupt,
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
        onNewSession={onNewSession}
        modelRefreshKey={modelRefreshKey}
        getCurrentModel={getCurrentModel}
        listAvailableModels={listAvailableModels}
        setModel={setModel}
      />
      <MessageList messages={session?.messages ?? []} />
      <InputBar
        workingDir={project?.workingDir ?? ''}
        phase={phase}
        onSubmit={onSubmitPrompt}
        onInterrupt={onInterrupt}
      />
    </div>
  );
}
