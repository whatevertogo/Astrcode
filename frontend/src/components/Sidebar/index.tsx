import { useState } from 'react';
import type { Phase, Project } from '../../types';
import ProjectItem from './ProjectItem';
import NewProjectModal from '../NewProjectModal';
import styles from './Sidebar.module.css';

const PHASE_COLOR: Record<Phase, string> = {
  idle: '#4ec9b0',
  thinking: '#dcdcaa',
  callingTool: '#9cdcfe',
  streaming: '#c586c0',
  interrupted: '#f44747',
  done: '#4ec9b0',
};

interface SidebarProps {
  projects: Project[];
  activeSessionId: string | null;
  phase: Phase;
  onSetActive: (projectId: string, sessionId: string) => void;
  onToggleExpand: (projectId: string) => void;
  canSelectDirectory: boolean;
  defaultWorkingDir?: string;
  onSelectDirectory: () => Promise<string | null>;
  onNewProject: (workingDir: string) => void;
  onDeleteProject: (projectId: string) => void;
  onDeleteSession: (projectId: string, sessionId: string) => void;
  onOpenSettings: () => void;
}

export default function Sidebar({
  projects,
  activeSessionId,
  phase,
  onSetActive,
  onToggleExpand,
  canSelectDirectory,
  defaultWorkingDir,
  onSelectDirectory,
  onNewProject,
  onDeleteProject,
  onDeleteSession,
  onOpenSettings,
}: SidebarProps) {
  const [showModal, setShowModal] = useState(false);

  return (
    <div className={styles.sidebar}>
      {/* Header */}
      <div className={styles.header}>
        <span className={styles.title}>AstrCode</span>
        <span
          className={styles.phaseIndicator}
          style={{ backgroundColor: PHASE_COLOR[phase] }}
          title={phase}
        />
      </div>

      {/* Project tree */}
      <div className={styles.projectList}>
        {projects.map((project) => (
          <ProjectItem
            key={project.id}
            project={project}
            activeSessionId={activeSessionId}
            onSetActive={onSetActive}
            onToggleExpand={onToggleExpand}
            onDelete={onDeleteProject}
            onDeleteSession={onDeleteSession}
          />
        ))}
      </div>

      {/* Footer */}
      <div className={styles.footer}>
        <div className={styles.footerActions}>
          <button className={styles.newProjectBtn} onClick={() => setShowModal(true)}>
            + 新项目
          </button>
          <button
            type="button"
            className={styles.settingsBtn}
            onClick={onOpenSettings}
            aria-label="打开设置"
            title="设置"
          >
            <svg viewBox="0 0 24 24" className={styles.settingsIcon} aria-hidden="true">
              <path
                d="M10.4 2h3.2l.5 2.6c.6.2 1.1.5 1.6.9l2.5-.9 1.6 2.8-2 1.7c.1.3.1.6.1.9s0 .6-.1.9l2 1.7-1.6 2.8-2.5-.9c-.5.4-1 .7-1.6.9l-.5 2.6h-3.2l-.5-2.6c-.6-.2-1.1-.5-1.6-.9l-2.5.9-1.6-2.8 2-1.7c-.1-.3-.1-.6-.1-.9s0-.6.1-.9l-2-1.7 1.6-2.8 2.5.9c.5-.4 1-.7 1.6-.9L10.4 2Zm1.6 6.5A3.5 3.5 0 1 0 12 15.5 3.5 3.5 0 0 0 12 8.5Z"
                fill="currentColor"
              />
            </svg>
          </button>
        </div>
      </div>

      {showModal && (
        <NewProjectModal
          canSelectDirectory={canSelectDirectory}
          defaultWorkingDir={defaultWorkingDir}
          onSelectDirectory={onSelectDirectory}
          onConfirm={(workingDir) => {
            onNewProject(workingDir);
            setShowModal(false);
          }}
          onCancel={() => setShowModal(false)}
        />
      )}
    </div>
  );
}
