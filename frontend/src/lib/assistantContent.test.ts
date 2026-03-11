import { describe, expect, it } from 'vitest';
import { splitAssistantContent } from './assistantContent';

describe('splitAssistantContent', () => {
  it('extracts legacy think blocks from assistant text', () => {
    expect(splitAssistantContent('before\n<think>step 1</think>\nafter')).toEqual({
      text: 'before\n\nafter',
      reasoningText: 'step 1',
    });
  });

  it('prefers explicit reasoning while still hiding legacy think tags', () => {
    expect(splitAssistantContent('<think>legacy</think>\nvisible', 'persisted')).toEqual({
      text: 'visible',
      reasoningText: 'persisted\n\nlegacy',
    });
  });
});
