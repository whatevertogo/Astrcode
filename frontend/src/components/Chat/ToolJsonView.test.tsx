import { describe, expect, it } from 'vitest';

import { summarizeLongString } from './ToolJsonView';

describe('summarizeLongString', () => {
  it('keeps short strings unchanged after whitespace normalization', () => {
    expect(summarizeLongString('alpha   beta')).toBe('alpha beta');
  });

  it('truncates long strings into a compact preview', () => {
    const value = `${'x'.repeat(360)} tail`;
    const preview = summarizeLongString(value);

    expect(preview.length).toBeLessThan(value.length);
    expect(preview.endsWith('...')).toBe(true);
  });

  it('renders empty strings with a stable placeholder', () => {
    expect(summarizeLongString('   \n\t   ')).toBe('(empty string)');
  });
});
