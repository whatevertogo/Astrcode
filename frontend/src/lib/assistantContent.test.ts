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

  // Shared test cases for behavioral alignment with Rust implementation
  // Note: blank line collapsing only happens when think tags are removed
  it('collapses extra blank lines when think tags are removed', () => {
    expect(splitAssistantContent('before\n<think>step</think>\n\n\n\nafter')).toEqual({
      text: 'before\n\nafter',
      reasoningText: 'step',
    });
  });

  it('returns original text when no think tags present', () => {
    expect(splitAssistantContent('plain text')).toEqual({
      text: 'plain text',
      reasoningText: undefined,
    });
  });

  it('handles case insensitive think tags', () => {
    expect(splitAssistantContent('<THINK>thinking</THINK>')).toEqual({
      text: '',
      reasoningText: 'thinking',
    });
  });

  it('deduplicates identical explicit and inline reasoning', () => {
    expect(splitAssistantContent('<think>thinking</think>', 'thinking')).toEqual({
      text: '',
      reasoningText: 'thinking',
    });
  });

  // Note: empty/whitespace-only think blocks do NOT trigger tag removal
  // This is consistent with Rust implementation
  it('preserves original text when think block is empty', () => {
    expect(splitAssistantContent('<think>   </think>\n\nvisible')).toEqual({
      text: '<think>   </think>\n\nvisible',
      reasoningText: undefined,
    });
  });

  it('handles empty string', () => {
    expect(splitAssistantContent('')).toEqual({
      text: '',
      reasoningText: undefined,
    });
  });

  it('extracts multiple think blocks', () => {
    expect(
      splitAssistantContent(
        'Answer before\n<think> first step</think>\n<think>second step</think>\nAnswer after'
      )
    ).toEqual({
      text: 'Answer before\n\nAnswer after',
      reasoningText: 'first step\n\nsecond step',
    });
  });
});
