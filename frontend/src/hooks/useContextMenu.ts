//! # useContextMenu Hook
//!
//! 统一右键菜单逻辑，避免 ProjectItem / SessionItem 重复实现。
//! 提供菜单状态、ref、关闭逻辑和位置自动修正。

import { useEffect, useRef, useState, useCallback } from 'react';

interface ContextMenuState {
  x: number;
  y: number;
}

interface UseContextMenuReturn {
  contextMenu: ContextMenuState | null;
  menuRef: React.RefObject<HTMLDivElement>;
  openMenu: (e: React.MouseEvent) => void;
  closeMenu: () => void;
}

export function useContextMenu(): UseContextMenuReturn {
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // 点击外部自动关闭
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

  // 自动修正菜单位置，防止溢出视口
  useEffect(() => {
    if (!contextMenu || !menuRef.current) return;

    const margin = 8;
    const maxX = Math.max(margin, window.innerWidth - menuRef.current.offsetWidth - margin);
    const maxY = Math.max(margin, window.innerHeight - menuRef.current.offsetHeight - margin);
    const nextX = Math.min(contextMenu.x, maxX);
    const nextY = Math.min(contextMenu.y, maxY);
    if (nextX !== contextMenu.x || nextY !== contextMenu.y) {
      setContextMenu({ x: nextX, y: nextY });
    }
  }, [contextMenu]);

  const openMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
  }, []);

  const closeMenu = useCallback(() => {
    setContextMenu(null);
  }, []);

  return { contextMenu, menuRef, openMenu, closeMenu };
}
