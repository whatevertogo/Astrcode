import React from 'react';
import type { Phase, Project, Session } from '../../types';
import TopBar from './TopBar';
import MessageList from './MessageList';
import InputBar from './InputBar';
import styles from './Chat.module.css';

interface ChatProps {
  project: Project | null;
  session: Session | null;
  phase: Phase;
  onNewSession: () => void;
  onSubmitPrompt: (text: string) => void;
  onInterrupt: () => void;
}

export default function Chat({
  project,
  session,
  phase,
  onNewSession,
  onSubmitPrompt,
  onInterrupt,
}: ChatProps) {
  return (
    <div className={styles.chat}>
      <TopBar
        projectName={project?.name ?? null}
        sessionTitle={session?.title ?? null}
        onNewSession={onNewSession}
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
