import type { Project } from '../../types';
import SessionItem from './SessionItem';
import { useContextMenu } from '../../hooks/useContextMenu';
import { contextMenu as contextMenuClass, menuItem } from '../../lib/styles';
import { cn } from '../../lib/utils';

interface ProjectItemProps {
  project: Project;
  activeSessionId: string | null;
  onSetActive: (projectId: string, sessionId: string) => void;
  onToggleExpand: (projectId: string) => void;
  onDelete: (projectId: string) => void;
  onDeleteSession: (projectId: string, sessionId: string) => void;
}

export default function ProjectItem({
  project,
  activeSessionId,
  onSetActive,
  onToggleExpand,
  onDelete,
  onDeleteSession,
}: ProjectItemProps) {
  const { contextMenu, menuRef, openMenu, closeMenu } = useContextMenu();

  return (
    <div className="relative">
      <div
        className="group flex min-h-[34px] cursor-pointer select-none items-center gap-2 rounded-lg px-2 py-1.5 transition-[color,background-color] duration-150 ease-out hover:bg-black/5 hover:text-text-primary"
        onContextMenu={openMenu}
        onClick={() => onToggleExpand(project.id)}
      >
        <span
          className="w-4 h-4 text-text-secondary shrink-0 inline-flex items-center justify-center relative"
          aria-hidden="true"
        >
          <svg
            className="w-4 h-4 absolute transition-[opacity] duration-150 ease-out group-hover:opacity-0"
            viewBox="0 0 20 20"
          >
            <path
              d="M2.5 5.75A1.75 1.75 0 0 1 4.25 4h4.03c.46 0 .9.18 1.23.5l1.02 1c.32.3.74.47 1.18.47h4.04A1.75 1.75 0 0 1 17.5 7.72v6.53A1.75 1.75 0 0 1 15.75 16H4.25A1.75 1.75 0 0 1 2.5 14.25V5.75Z"
              fill="none"
              stroke="currentColor"
              strokeLinejoin="round"
              strokeWidth="1.4"
            />
          </svg>
          <svg
            className={cn(
              'w-4 h-4 absolute opacity-0 transition-[opacity,transform] duration-150 ease-out group-hover:opacity-100',
              project.isExpanded && 'rotate-90'
            )}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <polyline points="9 18 15 12 9 6"></polyline>
          </svg>
        </span>
        <span className="text-[13px] text-text-primary font-medium flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
          {project.name}
        </span>
      </div>

      {project.isExpanded && (
        <div className="pt-1">
          {project.sessions.map((session) => (
            <SessionItem
              key={session.id}
              session={session}
              isActive={session.id === activeSessionId}
              onSelect={() => onSetActive(project.id, session.id)}
              onDelete={() => onDeleteSession(project.id, session.id)}
            />
          ))}
        </div>
      )}

      {contextMenu && (
        <div
          ref={menuRef}
          className={contextMenuClass}
          style={{ top: contextMenu.y, left: contextMenu.x }}
        >
          <button
            className={cn(menuItem, 'text-danger')}
            type="button"
            onClick={() => {
              onDelete(project.id);
              closeMenu();
            }}
          >
            删除项目
          </button>
        </div>
      )}
    </div>
  );
}
