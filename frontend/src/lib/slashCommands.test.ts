import { describe, expect, it } from 'vitest';

import { parseRuntimeSlashCommand } from './slashCommands';

describe('parseRuntimeSlashCommand', () => {
  it('matches compact when used as a standalone command', () => {
    expect(parseRuntimeSlashCommand('/compact')).toEqual({ kind: 'compact' });
    expect(parseRuntimeSlashCommand('  /compact  ')).toEqual({ kind: 'compact' });
  });

  it('captures compact instructions when arguments are present', () => {
    expect(parseRuntimeSlashCommand('/compact now')).toEqual({
      kind: 'compact',
      instructions: 'now',
    });
    expect(parseRuntimeSlashCommand('/compact   keep paths and errors  ')).toEqual({
      kind: 'compact',
      instructions: 'keep paths and errors',
    });
  });

  it('does not hijack similarly named prompt text', () => {
    expect(parseRuntimeSlashCommand('/compactLater')).toBeNull();
    expect(parseRuntimeSlashCommand('/git-commit')).toBeNull();
  });
});
