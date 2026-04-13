import { describe, expect, it } from 'vitest';
import { isExecutionPhase } from './phase';

describe('isExecutionPhase', () => {
  it('treats active runtime phases as busy', () => {
    expect(isExecutionPhase('thinking')).toBe(true);
    expect(isExecutionPhase('callingTool')).toBe(true);
    expect(isExecutionPhase('streaming')).toBe(true);
  });

  it('treats terminal and stable phases as interactive', () => {
    expect(isExecutionPhase('idle')).toBe(false);
    expect(isExecutionPhase('interrupted')).toBe(false);
    expect(isExecutionPhase('done')).toBe(false);
  });
});
