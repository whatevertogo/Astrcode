import { describe, expect, it } from 'vitest';

import { snapshotToolStatus } from './sessionMessages';

describe('snapshotToolStatus', () => {
  it('keeps unfinished tool calls in running state', () => {
    expect(snapshotToolStatus(undefined)).toBe('running');
  });

  it('maps finished tool calls to terminal states', () => {
    expect(snapshotToolStatus(true)).toBe('ok');
    expect(snapshotToolStatus(false)).toBe('fail');
  });
});
