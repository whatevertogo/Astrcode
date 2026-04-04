import React, { useEffect, useRef } from 'react';
import type { Session } from '../../types';
import { useContextMenu } from '../../hooks/useContextMenu';
import styles from './SessionItem.module.css';

interface SessionItemProps {
  session: Session;
  isActive: boolean;
  onSelect: () => void;
  onDelete: () => void;
}

export default function SessionItem({ session, isActive, onSelect, onDelete }: SessionItemProps) {
  const { contextMenu, menuRef, openMenu, closeMenu } = useContextMenu();
  const rowRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (isActive) {
      rowRef.current?.focus();
    }
  }, [isActive]);

  return (
    <div className={styles.wrapper} style={{ position: 'relative' }}>
      <div
        ref={rowRef}
        className={`${styles.sessionRow} ${isActive ? styles.active : ''}`}
        onClick={(e) => {
          onSelect();
          e.currentTarget.focus();
        }}
        onContextMenu={openMenu}
        tabIndex={0}
      >
        <span className={styles.title} title={session.title}>
          {session.title}
        </span>
      </div>

      {contextMenu && (
        <div
          ref={menuRef}
          className={styles.contextMenu}
          style={{ top: contextMenu.y, left: contextMenu.x, position: 'fixed' }}
        >
          <button
            className={`${styles.menuItem} ${styles.menuItemDanger}`}
            type="button"
            onClick={() => {
              onDelete();
              closeMenu();
            }}
          >
            删除会话
          </button>
        </div>
      )}
    </div>
  );
}
