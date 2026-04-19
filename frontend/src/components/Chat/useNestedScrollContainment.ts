import { useEffect } from 'react';
import type { RefObject } from 'react';

export type NestedScrollContainmentMode = 'contain' | 'bubble';

export function resolveNestedScrollContainmentMode(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
  deltaY: number
): NestedScrollContainmentMode {
  const canScroll = scrollHeight > clientHeight + 1;
  if (!canScroll || deltaY === 0) {
    return 'bubble';
  }

  const atTop = scrollTop <= 0 && deltaY < 0;
  const atBottom = scrollTop + clientHeight >= scrollHeight - 1 && deltaY > 0;
  return atTop || atBottom ? 'bubble' : 'contain';
}

export function useNestedScrollContainment<T extends HTMLElement>(ref: RefObject<T | null>) {
  useEffect(() => {
    const container = ref.current;
    if (!container) {
      return;
    }

    const onWheel = (event: WheelEvent) => {
      if (
        resolveNestedScrollContainmentMode(
          container.scrollTop,
          container.clientHeight,
          container.scrollHeight,
          event.deltaY
        ) !== 'contain'
      ) {
        return;
      }

      event.stopPropagation();
    };

    container.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      container.removeEventListener('wheel', onWheel);
    };
  }, [ref]);
}
