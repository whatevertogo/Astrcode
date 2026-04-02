import { describe, expect, it } from 'vitest';

import { clampSidebarWidth, getMaxSidebarWidth } from './useSidebarResize';

describe('useSidebarResize helpers', () => {
  it('keeps max width safe without a browser viewport', () => {
    expect(getMaxSidebarWidth(0)).toBe(220);
  });

  it('clamps widths to the configured minimum and computed maximum', () => {
    expect(clampSidebarWidth(100, 1280)).toBe(220);
    expect(clampSidebarWidth(900, 1280)).toBe(420);
    expect(clampSidebarWidth(320, 900)).toBe(320);
  });
});
