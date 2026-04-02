import { describe, expect, it } from 'vitest';

import { appendToolDeltaMetadata, extractToolShellDisplay, mergeToolMetadata } from './toolDisplay';

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
});
