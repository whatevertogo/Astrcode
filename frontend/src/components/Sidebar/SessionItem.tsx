import React, { useEffect, useRef, useState } from 'react';
import type { Session } from '../../types';
import styles from './SessionItem.module.css';

interface SessionItemProps {
  session: Session;
  isActive: boolean;
  onSelect: () => void;
  onDelete: () => void;
}

interface ContextMenuState {
  x: number;
  y: number;
}

export default function SessionItem({ session, isActive, onSelect, onDelete }: SessionItemProps) {
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
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

  useEffect(() => {
    if (isActive) {
      rowRef.current?.focus();
    }
  }, [isActive]);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
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
        tabIndex={0}
      >
        <span className={styles.bullet}>›</span>
        <span className={styles.title}>{session.title}</span>
      </div>

      {contextMenu && (
        <div
          ref={menuRef}
          className={styles.contextMenu}
          style={{ top: contextMenu.y, left: contextMenu.x, position: 'fixed' }}
        >
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
