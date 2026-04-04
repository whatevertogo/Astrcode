import { describe, expect, it } from 'vitest';

import { parseRuntimeSlashCommand } from './slashCommands';

describe('parseRuntimeSlashCommand', () => {
  it('matches compact when used as a standalone command', () => {
    expect(parseRuntimeSlashCommand('/compact')).toEqual({ kind: 'compact' });
    expect(parseRuntimeSlashCommand('  /compact  ')).toEqual({ kind: 'compact' });
  });

  it('rejects compact with unexpected arguments', () => {
    expect(parseRuntimeSlashCommand('/compact now')).toEqual({ kind: 'compactInvalidArgs' });
  });

  it('does not hijack similarly named prompt text', () => {
    expect(parseRuntimeSlashCommand('/compactLater')).toBeNull();
    expect(parseRuntimeSlashCommand('/git-commit')).toBeNull();
  });
});
