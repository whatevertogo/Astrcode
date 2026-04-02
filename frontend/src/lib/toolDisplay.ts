import type { ToolOutputStream } from '../types';

export interface ToolShellDisplaySegment {
  stream: ToolOutputStream;
  text: string;
}

export interface ToolShellDisplayMetadata {
  kind: 'terminal';
  command?: string;
  cwd?: string;
  shell?: string;
  exitCode?: number;
  segments: ToolShellDisplaySegment[];
}

type UnknownRecord = Record<string, unknown>;

function asRecord(value: unknown): UnknownRecord | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as UnknownRecord;
}

function pickString(record: UnknownRecord, key: string): string | undefined {
  const value = record[key];
  return typeof value === 'string' ? value : undefined;
}

function pickNumber(record: UnknownRecord, key: string): number | undefined {
  const value = record[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function extractShellSegments(value: unknown): ToolShellDisplaySegment[] {
  if (!Array.isArray(value)) {
    return [];
  }

  return value.flatMap((segment) => {
    const record = asRecord(segment);
    const stream = record?.stream;
    const text = record?.text;
    if ((stream === 'stdout' || stream === 'stderr') && typeof text === 'string') {
      return [{ stream, text }];
    }
    return [];
  });
}

function serializeShellDisplay(display: ToolShellDisplayMetadata): UnknownRecord {
  return {
    kind: 'terminal',
    ...(display.command ? { command: display.command } : {}),
    ...(display.cwd ? { cwd: display.cwd } : {}),
    ...(display.shell ? { shell: display.shell } : {}),
    ...(display.exitCode !== undefined ? { exitCode: display.exitCode } : {}),
    ...(display.segments.length > 0
      ? {
          segments: display.segments.map((segment) => ({
            stream: segment.stream,
            text: segment.text,
          })),
        }
      : {}),
  };
}

function commandFromArgs(args: unknown): string | undefined {
  return pickString(asRecord(args) ?? {}, 'command');
}

function cwdFromArgs(args: unknown): string | undefined {
  return pickString(asRecord(args) ?? {}, 'cwd');
}

export function extractToolShellDisplay(metadata: unknown): ToolShellDisplayMetadata | null {
  const container = asRecord(metadata);
  const display = asRecord(container?.display);
  if (!display || display.kind !== 'terminal') {
    return null;
  }

  return {
    kind: 'terminal',
    command: pickString(display, 'command'),
    cwd: pickString(display, 'cwd'),
    shell: pickString(display, 'shell'),
    exitCode: pickNumber(display, 'exitCode'),
    segments: extractShellSegments(display.segments),
  };
}

export function appendToolDeltaMetadata(
  metadata: unknown,
  toolName: string,
  args: unknown,
  stream: ToolOutputStream,
  delta: string
): unknown {
  if (toolName !== 'shell') {
    return metadata;
  }

  const container = { ...(asRecord(metadata) ?? {}) };
  const current = extractToolShellDisplay(metadata);
  const segments = [...(current?.segments ?? [])];
  const last = segments[segments.length - 1];
  if (last && last.stream === stream) {
    last.text += delta;
  } else {
    segments.push({ stream, text: delta });
  }

  container.display = serializeShellDisplay({
    kind: 'terminal',
    command: current?.command ?? commandFromArgs(args),
    cwd: current?.cwd ?? cwdFromArgs(args),
    shell: current?.shell,
    exitCode: current?.exitCode,
    segments,
  });
  return container;
}

export function mergeToolMetadata(previousMetadata: unknown, nextMetadata: unknown): unknown {
  const previousDisplay = extractToolShellDisplay(previousMetadata);
  const nextDisplay = extractToolShellDisplay(nextMetadata);
  if (!previousDisplay && nextMetadata !== undefined) {
    return nextMetadata;
  }
  if (!previousDisplay) {
    return previousMetadata;
  }

  const merged = { ...(asRecord(nextMetadata) ?? asRecord(previousMetadata) ?? {}) };
  merged.display = serializeShellDisplay({
    kind: 'terminal',
    command: nextDisplay?.command ?? previousDisplay.command,
    cwd: nextDisplay?.cwd ?? previousDisplay.cwd,
    shell: nextDisplay?.shell ?? previousDisplay.shell,
    exitCode: nextDisplay?.exitCode ?? previousDisplay.exitCode,
    segments: previousDisplay.segments,
  });
  return merged;
}
