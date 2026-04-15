import { useEffect } from 'react';
import type { RefObject } from 'react';

export function useNestedScrollContainment<T extends HTMLElement>(ref: RefObject<T | null>) {
  useEffect(() => {
    const container = ref.current;
    if (!container) {
      return;
    }

    const onWheel = (event: WheelEvent) => {
      const canScroll = container.scrollHeight > container.clientHeight + 1;
      if (!canScroll) {
        return;
      }

      const atTop = container.scrollTop <= 0 && event.deltaY < 0;
      const atBottom =
        container.scrollTop + container.clientHeight >= container.scrollHeight - 1 &&
        event.deltaY > 0;

      if (!atTop && !atBottom) {
        event.stopPropagation();
        return;
      }

      event.preventDefault();
    };

    container.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      container.removeEventListener('wheel', onWheel);
    };
  }, [ref]);
}
