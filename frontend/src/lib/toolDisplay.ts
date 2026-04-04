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

// 工具函数已迁移到 lib/shared/index.ts，此处统一引用
import {
  asRecord,
  pickStringOrUndefined as pickString,
  pickNumberOrUndefined as pickNumber,
  UnknownRecord,
} from './shared';

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

export function formatToolShellPreview(
  display: ToolShellDisplayMetadata | null,
  fallbackToolName: string,
  fallbackOutput?: string,
  fallbackError?: string,
  fallbackRunningText = '执行中...'
): string {
  if (!display) {
    return fallbackError ?? fallbackOutput ?? fallbackRunningText;
  }

  const latestChunk = display.segments[display.segments.length - 1]?.text ?? fallbackOutput ?? '';
  const lines = latestChunk
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const latestLine = lines[lines.length - 1];
  // Include the resolved shell in collapsed previews so users can tell at a glance
  // whether the command ran under pwsh, powershell, or /bin/sh without expanding.
  const commandLabel = display.command ? `$ ${display.command}` : fallbackToolName;
  const prefix = [display.shell ? `[${display.shell}]` : '', commandLabel]
    .filter(Boolean)
    .join(' ');

  return latestLine ? `${prefix}  ${latestLine}` : prefix;
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
