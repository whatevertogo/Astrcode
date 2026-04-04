import { describe, expect, it } from 'vitest';

import {
  appendToolDeltaMetadata,
  extractStructuredJsonOutput,
  extractToolMetadataSummary,
  extractToolShellDisplay,
  formatToolShellPreview,
  mergeToolMetadata,
} from './toolDisplay';

describe('toolDisplay shell metadata helpers', () => {
  it('extracts terminal display metadata and segments', () => {
    const display = extractToolShellDisplay({
      display: {
        kind: 'terminal',
        command: 'npm test',
        cwd: '/repo',
        exitCode: 1,
        segments: [
          { stream: 'stdout', text: 'ok\\n' },
          { stream: 'stderr', text: 'boom\\n' },
        ],
      },
    });

    expect(display).toEqual({
      kind: 'terminal',
      command: 'npm test',
      cwd: '/repo',
      shell: undefined,
      exitCode: 1,
      segments: [
        { stream: 'stdout', text: 'ok\\n' },
        { stream: 'stderr', text: 'boom\\n' },
      ],
    });
  });

  it('appends shell deltas into display metadata and keeps command from args', () => {
    const metadata = appendToolDeltaMetadata(
      undefined,
      'shell',
      { command: 'cargo test', cwd: '/repo' },
      'stdout',
      'running\\n'
    );

    expect(extractToolShellDisplay(metadata)).toEqual({
      kind: 'terminal',
      command: 'cargo test',
      cwd: '/repo',
      shell: undefined,
      exitCode: undefined,
      segments: [{ stream: 'stdout', text: 'running\\n' }],
    });
  });

  it('preserves streamed shell segments when final metadata arrives later', () => {
    const previous = appendToolDeltaMetadata(
      undefined,
      'shell',
      { command: 'cargo test' },
      'stdout',
      'running\\n'
    );
    const merged = mergeToolMetadata(previous, {
      command: 'cargo test',
      exitCode: 0,
      display: {
        kind: 'terminal',
        command: 'cargo test',
        exitCode: 0,
      },
    });

    expect(extractToolShellDisplay(merged)).toEqual({
      kind: 'terminal',
      command: 'cargo test',
      cwd: undefined,
      shell: undefined,
      exitCode: 0,
      segments: [{ stream: 'stdout', text: 'running\\n' }],
    });
  });

  it('formats collapsed shell previews with the resolved shell name', () => {
    const preview = formatToolShellPreview(
      {
        kind: 'terminal',
        command: 'cargo test',
        cwd: '/repo',
        shell: 'pwsh',
        exitCode: 0,
        segments: [{ stream: 'stdout', text: 'ok\n' }],
      },
      'shell'
    );

    expect(preview).toBe('[pwsh] $ cargo test  ok');
  });

  it('extracts generic tool metadata message and stats pills', () => {
    const summary = extractToolMetadataSummary({
      message: 'No matches found for the given pattern.',
      returned: 0,
      output_mode: 'content',
      truncated: true,
      has_more: true,
    });

    expect(summary).toEqual({
      message: 'No matches found for the given pattern.',
      pills: ['0 returned', 'mode content', 'has more', 'truncated'],
    });
  });

  it('returns null when metadata has no user-facing summary fields', () => {
    expect(extractToolMetadataSummary({ path: '/repo/file.ts' })).toBeNull();
  });

  it('extracts structured JSON output when top-level is object or array', () => {
    expect(extractStructuredJsonOutput('{"a":1,"b":true}')?.summary).toBe('Object (2 keys)');
    expect(extractStructuredJsonOutput('[{"a":1},{"b":2}]')?.summary).toBe('Array (2 items)');
  });

  it('ignores non-structured or invalid JSON output', () => {
    expect(extractStructuredJsonOutput('"plain string"')).toBeNull();
    expect(extractStructuredJsonOutput('not json')).toBeNull();
    expect(extractStructuredJsonOutput(undefined)).toBeNull();
  });
});
