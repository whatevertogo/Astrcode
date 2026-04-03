import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';

const DEFAULT_SIDEBAR_WIDTH = 260;
const MIN_SIDEBAR_WIDTH = 220;
const MAX_SIDEBAR_WIDTH = 420;
const SIDEBAR_WIDTH_STORAGE_KEY = 'astrcode.sidebarWidth';
const SIDEBAR_OPEN_STORAGE_KEY = 'astrcode.sidebarOpen';
const SIDEBAR_KEYBOARD_STEP = 16;

type SidebarDragState = {
  startX: number;
  startWidth: number;
};

function getViewportWidth(): number {
  return typeof window === 'undefined' ? DEFAULT_SIDEBAR_WIDTH + 360 : window.innerWidth;
}

export function getMaxSidebarWidth(viewportWidth = getViewportWidth()): number {
  return Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, viewportWidth - 360));
}

export function clampSidebarWidth(width: number, viewportWidth = getViewportWidth()): number {
  return Math.min(getMaxSidebarWidth(viewportWidth), Math.max(MIN_SIDEBAR_WIDTH, width));
}

export function useSidebarResize() {
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    if (typeof window === 'undefined') {
      return DEFAULT_SIDEBAR_WIDTH;
    }

    const savedWidth = Number(window.localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY));
    return Number.isFinite(savedWidth) ? clampSidebarWidth(savedWidth) : DEFAULT_SIDEBAR_WIDTH;
  });
  const [isResizingSidebar, setIsResizingSidebar] = useState(false);
  const [isSidebarOpen, setIsSidebarOpen] = useState(() => {
    if (typeof window === 'undefined') {
      return true;
    }
    const savedState = window.localStorage.getItem(SIDEBAR_OPEN_STORAGE_KEY);
    return savedState !== 'false';
  });
  const sidebarDragRef = useRef<SidebarDragState | null>(null);

  const toggleSidebar = useCallback(() => {
    setIsSidebarOpen((prev) => !prev);
  }, []);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }

    // Persisting width avoids a jarring layout jump every time the desktop app reopens.
    window.localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(sidebarWidth));
  }, [sidebarWidth]);

  useEffect(() => {
    if (typeof window !== 'undefined') {
      window.localStorage.setItem(SIDEBAR_OPEN_STORAGE_KEY, String(isSidebarOpen));
    }
  }, [isSidebarOpen]);

  useEffect(() => {
    const handleResize = () => {
      setSidebarWidth((width) => clampSidebarWidth(width));
    };

    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  useEffect(() => {
    return () => {
      document.body.style.removeProperty('cursor');
      document.body.style.removeProperty('user-select');
    };
  }, []);

  const finishSidebarResize = useCallback(() => {
    sidebarDragRef.current = null;
    setIsResizingSidebar(false);
    document.body.style.removeProperty('cursor');
    document.body.style.removeProperty('user-select');
  }, []);

  useEffect(() => {
    if (!isResizingSidebar) {
      return;
    }

    const handlePointerMove = (event: globalThis.PointerEvent) => {
      const dragState = sidebarDragRef.current;
      if (!dragState) {
        return;
      }

      setSidebarWidth(clampSidebarWidth(dragState.startWidth + event.clientX - dragState.startX));
    };

    const handlePointerUp = () => {
      finishSidebarResize();
    };

    window.addEventListener('pointermove', handlePointerMove);
    window.addEventListener('pointerup', handlePointerUp);
    window.addEventListener('pointercancel', handlePointerUp);

    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', handlePointerUp);
      window.removeEventListener('pointercancel', handlePointerUp);
    };
  }, [finishSidebarResize, isResizingSidebar]);

  const handleSidebarResizeStart = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      sidebarDragRef.current = {
        startX: event.clientX,
        startWidth: sidebarWidth,
      };
      setIsResizingSidebar(true);
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
    },
    [sidebarWidth]
  );

  const handleSidebarResizeKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      setSidebarWidth((width) => clampSidebarWidth(width - SIDEBAR_KEYBOARD_STEP));
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      setSidebarWidth((width) => clampSidebarWidth(width + SIDEBAR_KEYBOARD_STEP));
    }
  }, []);

  return {
    sidebarWidth,
    isResizingSidebar,
    isSidebarOpen,
    toggleSidebar,
    minSidebarWidth: MIN_SIDEBAR_WIDTH,
    maxSidebarWidth: getMaxSidebarWidth(),
    handleSidebarResizeStart,
    handleSidebarResizeKeyDown,
  };
}
