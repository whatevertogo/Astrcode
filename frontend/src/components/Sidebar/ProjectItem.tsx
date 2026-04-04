import React, { useRef, useEffect, useState } from 'react';
import type { Project } from '../../types';
import SessionItem from './SessionItem';
import styles from './ProjectItem.module.css';

interface ContextMenuState {
  x: number;
  y: number;
}

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
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Close context menu on outside click
  useEffect(() => {
    if (!contextMenu) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setContextMenu(null);
      }
    };
    window.addEventListener('mousedown', handler);
    return () => window.removeEventListener('mousedown', handler);
  }, [contextMenu]);

  useEffect(() => {
    if (!contextMenu || !menuRef.current) {
      return;
    }

    const margin = 8;
    const maxX = Math.max(margin, window.innerWidth - menuRef.current.offsetWidth - margin);
    const maxY = Math.max(margin, window.innerHeight - menuRef.current.offsetHeight - margin);
    const nextX = Math.min(contextMenu.x, maxX);
    const nextY = Math.min(contextMenu.y, maxY);
    if (nextX !== contextMenu.x || nextY !== contextMenu.y) {
      setContextMenu({ x: nextX, y: nextY });
    }
  }, [contextMenu]);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
  };

  return (
    <div className={styles.wrapper}>
      <div
        className={styles.projectRow}
        onContextMenu={handleContextMenu}
        onClick={() => onToggleExpand(project.id)}
      >
        <span className={styles.iconContainer} aria-hidden="true">
          <svg className={styles.folderIcon} viewBox="0 0 20 20">
            <path
              d="M2.5 5.75A1.75 1.75 0 0 1 4.25 4h4.03c.46 0 .9.18 1.23.5l1.02 1c.32.3.74.47 1.18.47h4.04A1.75 1.75 0 0 1 17.5 7.72v6.53A1.75 1.75 0 0 1 15.75 16H4.25A1.75 1.75 0 0 1 2.5 14.25V5.75Z"
              fill="none"
              stroke="currentColor"
              strokeLinejoin="round"
              strokeWidth="1.4"
            />
          </svg>
          <svg
            className={`${styles.arrowIcon} ${project.isExpanded ? styles.arrowOpen : ''}`}
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
        <span className={styles.projectName}>{project.name}</span>
      </div>

      {project.isExpanded && (
        <div className={styles.sessions}>
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
          className={styles.contextMenu}
          style={{ top: contextMenu.y, left: contextMenu.x }}
        >
          <button
            className={`${styles.menuItem} ${styles.menuItemDanger}`}
            type="button"
            onClick={() => {
              onDelete(project.id);
              setContextMenu(null);
            }}
          >
            删除项目
          </button>
        </div>
      )}
    </div>
  );
}
