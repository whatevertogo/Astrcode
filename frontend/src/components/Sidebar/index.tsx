import React, { useState } from 'react';
import type { Phase, Project } from '../../types';
import ProjectItem from './ProjectItem';
import NewProjectModal from '../NewProjectModal';
import styles from './Sidebar.module.css';

const PHASE_COLOR: Record<Phase, string> = {
  idle:        '#4ec9b0',
  thinking:    '#dcdcaa',
  callingTool: '#9cdcfe',
  streaming:   '#c586c0',
  interrupted: '#f44747',
  done:        '#4ec9b0',
};

interface SidebarProps {
  projects: Project[];
  activeSessionId: string | null;
  phase: Phase;
  onSetActive: (projectId: string, sessionId: string) => void;
  onToggleExpand: (projectId: string) => void;
  onNewProject: (name: string, workingDir: string) => void;
  onRenameProject: (projectId: string, name: string) => void;
  onDeleteProject: (projectId: string) => void;
  onRenameSession: (projectId: string, sessionId: string, title: string) => void;
  onDeleteSession: (projectId: string, sessionId: string) => void;
}

export default function Sidebar({
  projects,
  activeSessionId,
  phase,
  onSetActive,
  onToggleExpand,
  onNewProject,
  onRenameProject,
  onDeleteProject,
  onRenameSession,
  onDeleteSession,
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
            onRename={onRenameProject}
            onDelete={onDeleteProject}
            onRenameSession={onRenameSession}
            onDeleteSession={onDeleteSession}
          />
        ))}
      </div>

      {/* Footer */}
      <div className={styles.footer}>
        <button
          className={styles.newProjectBtn}
          onClick={() => setShowModal(true)}
        >
          + 新项目
        </button>
      </div>

      {showModal && (
        <NewProjectModal
          onConfirm={(name, workingDir) => {
            onNewProject(name, workingDir);
            setShowModal(false);
          }}
          onCancel={() => setShowModal(false)}
        />
      )}
    </div>
  );
}
