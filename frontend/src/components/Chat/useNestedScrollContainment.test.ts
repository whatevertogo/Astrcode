import { describe, expect, it } from 'vitest';

import { resolveNestedScrollContainmentMode } from './useNestedScrollContainment';

describe('resolveNestedScrollContainmentMode', () => {
  it('contains wheel events while the nested container can continue scrolling', () => {
    expect(resolveNestedScrollContainmentMode(120, 200, 800, 48)).toBe('contain');
    expect(resolveNestedScrollContainmentMode(120, 200, 800, -48)).toBe('contain');
  });

  it('lets wheel events bubble at the top boundary', () => {
    expect(resolveNestedScrollContainmentMode(0, 200, 800, -48)).toBe('bubble');
  });

  it('lets wheel events bubble at the bottom boundary', () => {
    expect(resolveNestedScrollContainmentMode(600, 200, 800, 48)).toBe('bubble');
  });

  it('lets wheel events bubble when the nested container cannot scroll', () => {
    expect(resolveNestedScrollContainmentMode(0, 200, 200, 48)).toBe('bubble');
  });
});
