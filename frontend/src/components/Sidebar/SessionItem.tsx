import React, { useEffect, useRef } from 'react';
import type { Session } from '../../types';
import { useContextMenu } from '../../hooks/useContextMenu';
import { contextMenu as contextMenuClass, menuItem } from '../../lib/styles';
import { cn } from '../../lib/utils';

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
    <div className="relative">
      <div
        ref={rowRef}
        className={cn(
          'flex items-center min-h-9 ml-6 py-2 px-3 cursor-pointer transition-[background-color,border-color,box-shadow,transform] duration-150 ease-out select-none border border-transparent rounded-[10px] hover:bg-[rgba(255,255,255,0.6)] focus-visible:outline-2 focus-visible:outline-[rgba(76,138,255,0.18)] focus-visible:outline-offset-2',
          isActive &&
            'bg-surface border-border shadow-[0_8px_20px_rgba(112,86,50,0.08)] translate-x-px'
        )}
        onClick={(e) => {
          onSelect();
          e.currentTarget.focus();
        }}
        onContextMenu={openMenu}
        tabIndex={0}
      >
        <span
          className={cn(
            'text-[13px] text-text-secondary overflow-hidden text-ellipsis whitespace-nowrap flex-1',
            isActive && 'text-text-primary font-medium'
          )}
          title={session.title}
        >
          {session.title}
        </span>
      </div>

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
