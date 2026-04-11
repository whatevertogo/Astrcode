import { useEffect, useRef } from 'react';
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
          'ml-6 flex min-h-9 cursor-pointer select-none items-center rounded-[10px] border border-transparent px-3 py-2 transition-[background-color,border-color,box-shadow,transform] duration-150 ease-out hover:bg-white/60 focus-visible:outline-2 focus-visible:outline-accent-soft/30 focus-visible:outline-offset-2',
          isActive && 'translate-x-px border-border bg-surface shadow-soft'
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
