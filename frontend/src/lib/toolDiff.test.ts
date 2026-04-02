import { describe, expect, it } from 'vitest';

import { classifyToolDiffLine, extractToolDiffMetadata } from './toolDiff';

describe('extractToolDiffMetadata', () => {
  it('returns null when metadata does not contain a diff patch', () => {
    expect(extractToolDiffMetadata({ path: '/tmp/demo.ts' })).toBeNull();
    expect(extractToolDiffMetadata({ diff: { patch: '' } })).toBeNull();
  });

  it('normalizes diff metadata from tool results', () => {
    expect(
      extractToolDiffMetadata({
        path: '/tmp/demo.ts',
        diff: {
          patch: '--- a/demo.ts\n+++ b/demo.ts\n@@ -1,1 +1,1 @@\n-old\n+new',
          addedLines: 1,
          removedLines: 1,
          truncated: true,
          hasChanges: true,
        },
      })
    ).toEqual({
      path: '/tmp/demo.ts',
      patch: '--- a/demo.ts\n+++ b/demo.ts\n@@ -1,1 +1,1 @@\n-old\n+new',
      addedLines: 1,
      removedLines: 1,
      truncated: true,
      hasChanges: true,
    });
  });
});

describe('classifyToolDiffLine', () => {
  it('classifies unified diff lines by semantic role', () => {
    expect(classifyToolDiffLine('--- a/demo.ts')).toBe('meta');
    expect(classifyToolDiffLine('+++ b/demo.ts')).toBe('meta');
    expect(classifyToolDiffLine('@@ -1,1 +1,1 @@')).toBe('header');
    expect(classifyToolDiffLine('+new')).toBe('add');
    expect(classifyToolDiffLine('-old')).toBe('remove');
    expect(classifyToolDiffLine('... diff truncated ...')).toBe('note');
    expect(classifyToolDiffLine(' unchanged')).toBe('context');
  });
});
