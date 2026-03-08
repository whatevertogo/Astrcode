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
  onRename: (projectId: string, name: string) => void;
  onDelete: (projectId: string) => void;
  onRenameSession: (projectId: string, sessionId: string, title: string) => void;
  onDeleteSession: (projectId: string, sessionId: string) => void;
}

export default function ProjectItem({
  project,
  activeSessionId,
  onSetActive,
  onToggleExpand,
  onRename,
  onDelete,
  onRenameSession,
  onDeleteSession,
}: ProjectItemProps) {
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState(project.name);
  const menuRef = useRef<HTMLDivElement>(null);
  const renameRef = useRef<HTMLInputElement>(null);

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

  // Auto-focus rename input
  useEffect(() => {
    if (renaming) renameRef.current?.select();
  }, [renaming]);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
  };

  const commitRename = () => {
    const trimmed = renameValue.trim();
    if (trimmed && trimmed !== project.name) {
      onRename(project.id, trimmed);
    } else {
      setRenameValue(project.name);
    }
    setRenaming(false);
  };

  return (
    <div className={styles.wrapper}>
      {/* Project row */}
      <div
        className={styles.projectRow}
        onContextMenu={handleContextMenu}
        onClick={() => onToggleExpand(project.id)}
      >
        <span className={styles.arrow}>
          {project.isExpanded ? '▾' : '▸'}
        </span>
        <span className={styles.icon}>📁</span>
        {renaming ? (
          <input
            ref={renameRef}
            className={styles.renameInput}
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            onBlur={commitRename}
            onKeyDown={(e) => {
              if (e.key === 'Enter') commitRename();
              if (e.key === 'Escape') {
                setRenameValue(project.name);
                setRenaming(false);
              }
            }}
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <span className={styles.projectName}>{project.name}</span>
        )}
      </div>

      {/* Sessions */}
      {project.isExpanded && (
        <div className={styles.sessions}>
          {project.sessions.map((session) => (
            <SessionItem
              key={session.id}
              session={session}
              isActive={session.id === activeSessionId}
              onSelect={() => onSetActive(project.id, session.id)}
              onRename={(title) => onRenameSession(project.id, session.id, title)}
              onDelete={() => onDeleteSession(project.id, session.id)}
            />
          ))}
        </div>
      )}

      {/* Context menu */}
      {contextMenu && (
        <div
          ref={menuRef}
          className={styles.contextMenu}
          style={{ top: contextMenu.y, left: contextMenu.x }}
        >
          <button
            className={styles.menuItem}
            onClick={() => {
              setRenaming(true);
              setContextMenu(null);
            }}
          >
            重命名
          </button>
          <button
            className={`${styles.menuItem} ${styles.menuItemDanger}`}
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
