import React, { useEffect, useRef, useState } from 'react';
import type { Session } from '../../types';
import styles from './SessionItem.module.css';

interface SessionItemProps {
  session: Session;
  isActive: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
}

interface ContextMenuState {
  x: number;
  y: number;
}

export default function SessionItem({
  session,
  isActive,
  onSelect,
  onRename,
  onDelete,
}: SessionItemProps) {
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState(session.title);
  const menuRef = useRef<HTMLDivElement>(null);
  const renameRef = useRef<HTMLInputElement>(null);
  const rowRef = useRef<HTMLDivElement>(null);

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

  useEffect(() => {
    if (isActive && !renaming) {
      rowRef.current?.focus();
    }
  }, [isActive, renaming]);

  // Keep rename value in sync with session title
  useEffect(() => {
    if (!renaming) setRenameValue(session.title);
  }, [session.title, renaming]);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
  };

  const commitRename = () => {
    const trimmed = renameValue.trim();
    if (trimmed && trimmed !== session.title) {
      onRename(trimmed);
    } else {
      setRenameValue(session.title);
    }
    setRenaming(false);
  };

  return (
    <div className={styles.wrapper} style={{ position: 'relative' }}>
      <div
        ref={rowRef}
        className={`${styles.sessionRow} ${isActive ? styles.active : ''}`}
        onClick={(e) => {
          onSelect();
          e.currentTarget.focus();
        }}
        onContextMenu={handleContextMenu}
        onKeyDown={(e) => {
          if (e.key === 'F2') {
            e.preventDefault();
            setRenaming(true);
            setContextMenu(null);
          }
        }}
        tabIndex={0}
      >
        <span className={styles.bullet}>›</span>
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
                setRenameValue(session.title);
                setRenaming(false);
              }
            }}
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <span className={styles.title}>{session.title}</span>
        )}
      </div>

      {contextMenu && (
        <div
          ref={menuRef}
          className={styles.contextMenu}
          style={{ top: contextMenu.y, left: contextMenu.x, position: 'fixed' }}
        >
          <button
            className={styles.menuItem}
            onClick={() => {
              setRenaming(true);
              setContextMenu(null);
            }}
          >
            重命名 (F2)
          </button>
          <button
            className={`${styles.menuItem} ${styles.menuItemDanger}`}
            onClick={() => {
              onDelete();
              setContextMenu(null);
            }}
          >
            删除会话
          </button>
        </div>
      )}
    </div>
  );
}
