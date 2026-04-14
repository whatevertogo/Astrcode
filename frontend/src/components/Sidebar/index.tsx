import { useState } from 'react';
import type { Phase, Project } from '../../types';
import ProjectItem from './ProjectItem';
import NewProjectModal from '../NewProjectModal';
import { cn } from '../../lib/utils';

// Phase 指示灯颜色映射到 Tailwind 令牌
const PHASE_BG_CLASS: Record<Phase, string> = {
  idle: 'bg-phase-idle',
  thinking: 'bg-phase-thinking',
  callingTool: 'bg-phase-calling-tool',
  streaming: 'bg-phase-streaming',
  interrupted: 'bg-phase-interrupted',
  done: 'bg-phase-done',
};

interface SidebarProps {
  projects: Project[];
  activeSessionId: string | null;
  phase: Phase;
  onSelectProject: (projectId: string) => void;
  onSetActive: (projectId: string, sessionId: string) => void;
  onToggleExpand: (projectId: string) => void;
  canSelectDirectory: boolean;
  defaultWorkingDir?: string;
  onSelectDirectory: () => Promise<string | null>;
  onNewProject: (workingDir: string) => void;
  onDeleteProject: (projectId: string) => void;
  onDeleteSession: (projectId: string, sessionId: string) => void;
  onOpenSettings: () => void;
  onNewSession: () => void;
  showDebugWorkbenchEntry?: boolean;
  onOpenDebugWorkbench?: () => void;
}

export default function Sidebar({
  projects,
  activeSessionId,
  phase,
  onSelectProject,
  onSetActive,
  onToggleExpand,
  canSelectDirectory,
  defaultWorkingDir,
  onSelectDirectory,
  onNewProject,
  onDeleteProject,
  onDeleteSession,
  onOpenSettings,
  onNewSession,
  showDebugWorkbenchEntry = false,
  onOpenDebugWorkbench,
}: SidebarProps) {
  const [showModal, setShowModal] = useState(false);

  return (
    <div className="w-full min-w-0 bg-sidebar-bg flex flex-col h-full min-h-0 overflow-hidden px-3 pt-[18px] pb-4">
      <div className="flex items-center gap-2.5 px-2 shrink-0">
        <span
          className={cn(
            'h-[9px] w-[9px] shrink-0 rounded-full shadow-[0_0_0_6px_theme(colors.accent-soft/12%)] transition-[background-color] duration-300 ease-out',
            PHASE_BG_CLASS[phase]
          )}
          title={phase}
        />
        <span className="font-semibold text-[13px] tracking-[0.02em] text-text-primary flex-1">
          AstrCode
        </span>
      </div>

      <div className="mt-4 px-1 flex-shrink-0">
        <button
          type="button"
          onClick={onNewSession}
          className="flex min-h-[34px] w-full items-center gap-2 rounded-lg border-none bg-transparent px-2 text-text-primary outline-none transition-[background-color,color] duration-150 ease-out hover:bg-black/5"
        >
          <div className="w-4 h-4 flex items-center justify-center shrink-0 text-text-secondary">
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              className="w-4 h-4"
            >
              <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path>
              <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
            </svg>
          </div>
          <span className="truncate text-[13px] font-medium">新会话</span>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto px-1 pt-5 pb-4">
        <div className="px-2 mb-2 text-[11px] font-semibold text-text-muted tracking-[0.05em]">
          文件夹
        </div>
        {projects.map((project) => (
          <ProjectItem
            key={project.id}
            project={project}
            activeSessionId={activeSessionId}
            onSelectProject={onSelectProject}
            onSetActive={onSetActive}
            onToggleExpand={onToggleExpand}
            onDelete={onDeleteProject}
            onDeleteSession={onDeleteSession}
          />
        ))}
      </div>

      <div className="px-1 pt-4 border-t border-border shrink-0">
        {showDebugWorkbenchEntry && onOpenDebugWorkbench ? (
          <button
            type="button"
            className="mb-2 flex h-[38px] w-full items-center justify-center rounded-xl border border-border bg-surface text-sm font-semibold text-text-primary shadow-soft transition-[background-color,border-color,transform] duration-150 ease-out hover:border-border-strong hover:bg-white hover:-translate-y-px"
            onClick={onOpenDebugWorkbench}
          >
            打开 Debug Workbench
          </button>
        ) : null}
        <div className="flex items-center gap-2">
          <button
            className="h-[38px] flex-1 rounded-xl border border-border bg-surface text-center text-sm font-semibold text-text-primary shadow-soft transition-[background-color,border-color,transform] duration-150 ease-out hover:border-border-strong hover:bg-white hover:-translate-y-px"
            onClick={() => setShowModal(true)}
          >
            + 新项目
          </button>
          <button
            type="button"
            className="inline-flex h-[38px] w-[38px] items-center justify-center rounded-xl border border-border bg-surface text-text-secondary shadow-soft transition-[background-color,color,border-color,transform] duration-150 ease-out hover:border-border-strong hover:bg-white hover:text-text-primary hover:-translate-y-px"
            onClick={onOpenSettings}
            aria-label="打开设置"
            title="设置"
          >
            <svg viewBox="0 0 24 24" className="w-4 h-4" aria-hidden="true">
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
