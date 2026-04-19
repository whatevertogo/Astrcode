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

export interface ToolMetadataSummary {
  message?: string;
  pills: string[];
}

export interface PersistedToolOutputMetadata {
  storageKind: 'toolResult';
  absolutePath: string;
  relativePath: string;
  totalBytes: number;
  previewText: string;
  previewBytes: number;
}

export interface SessionPlanExitMetadata {
  schema: 'sessionPlanExit';
  mode: {
    fromModeId: string;
    toModeId: string;
    modeChanged: boolean;
  };
  plan: {
    title: string;
    status: string;
    slug: string;
    planPath: string;
    content: string;
    updatedAt?: string;
  };
}

export interface SessionPlanExitReviewPendingMetadata {
  schema: 'sessionPlanExitReviewPending' | 'sessionPlanExitBlocked';
  plan: {
    title: string;
    planPath: string;
  };
  review?: {
    kind: 'revise_plan' | 'final_review';
    checklist: string[];
  };
  blockers: {
    missingHeadings: string[];
    invalidSections: string[];
  };
}

export interface StructuredJsonOutput {
  value: UnknownRecord | unknown[];
  summary: string;
}

// 工具函数已迁移到 lib/shared/index.ts，此处统一引用
import {
  asRecord,
  pickBoolean,
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

export function extractPersistedToolOutput(metadata: unknown): PersistedToolOutputMetadata | null {
  const container = asRecord(metadata);
  const persisted = asRecord(container?.persistedOutput);
  if (!persisted) {
    return null;
  }

  const absolutePath = pickString(persisted, 'absolutePath');
  const relativePath = pickString(persisted, 'relativePath');
  const totalBytes = pickNumber(persisted, 'totalBytes');
  const previewText = pickString(persisted, 'previewText');
  const previewBytes = pickNumber(persisted, 'previewBytes');

  if (
    persisted.storageKind !== 'toolResult' ||
    !absolutePath ||
    !relativePath ||
    totalBytes === undefined ||
    previewText === undefined ||
    previewBytes === undefined
  ) {
    return null;
  }

  return {
    storageKind: 'toolResult',
    absolutePath,
    relativePath,
    totalBytes,
    previewText,
    previewBytes,
  };
}

export function extractSessionPlanExit(metadata: unknown): SessionPlanExitMetadata | null {
  const container = asRecord(metadata);
  const plan = asRecord(container?.plan);
  const mode = asRecord(container?.mode);
  const title = pickString(plan ?? {}, 'title');
  const status = pickString(plan ?? {}, 'status');
  const slug = pickString(plan ?? {}, 'slug');
  const planPath = pickString(plan ?? {}, 'planPath');
  const content = pickString(plan ?? {}, 'content');
  const fromModeId = pickString(mode ?? {}, 'fromModeId');
  const toModeId = pickString(mode ?? {}, 'toModeId');
  const modeChanged = pickBoolean(mode ?? {}, 'modeChanged');

  if (
    container?.schema !== 'sessionPlanExit' ||
    !title ||
    !status ||
    !slug ||
    !planPath ||
    !content ||
    !fromModeId ||
    !toModeId ||
    modeChanged === undefined
  ) {
    return null;
  }

  return {
    schema: 'sessionPlanExit',
    mode: {
      fromModeId,
      toModeId,
      modeChanged,
    },
    plan: {
      title,
      status,
      slug,
      planPath,
      content,
      updatedAt: pickString(plan ?? {}, 'updatedAt'),
    },
  };
}

export function extractSessionPlanExitReviewPending(
  metadata: unknown
): SessionPlanExitReviewPendingMetadata | null {
  const container = asRecord(metadata);
  const plan = asRecord(container?.plan);
  const review = asRecord(container?.review);
  const blockers = asRecord(container?.blockers);
  const title = pickString(plan ?? {}, 'title');
  const planPath = pickString(plan ?? {}, 'planPath');
  const reviewKind = pickString(review ?? {}, 'kind');
  const reviewChecklist = Array.isArray(review?.checklist)
    ? review.checklist.filter((value): value is string => typeof value === 'string')
    : [];
  const missingHeadings = Array.isArray(blockers?.missingHeadings)
    ? blockers.missingHeadings.filter((value): value is string => typeof value === 'string')
    : [];
  const invalidSections = Array.isArray(blockers?.invalidSections)
    ? blockers.invalidSections.filter((value): value is string => typeof value === 'string')
    : [];

  if (
    (container?.schema !== 'sessionPlanExitReviewPending' &&
      container?.schema !== 'sessionPlanExitBlocked') ||
    !title ||
    !planPath
  ) {
    return null;
  }

  return {
    schema:
      container.schema === 'sessionPlanExitBlocked'
        ? 'sessionPlanExitBlocked'
        : 'sessionPlanExitReviewPending',
    plan: {
      title,
      planPath,
    },
    review:
      reviewKind === 'revise_plan' || reviewKind === 'final_review'
        ? {
            kind: reviewKind,
            checklist: reviewChecklist,
          }
        : undefined,
    blockers: {
      missingHeadings,
      invalidSections,
    },
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

function pushNumberPill(
  pills: string[],
  metadata: UnknownRecord,
  keys: string[],
  format: (value: number) => string
): void {
  const value = pickNumber(metadata, ...keys);
  if (value !== undefined) {
    pills.push(format(value));
  }
}

export function extractToolMetadataSummary(metadata: unknown): ToolMetadataSummary | null {
  const container = asRecord(metadata);
  if (!container) {
    return null;
  }

  const pills: string[] = [];
  const message = pickString(container, 'message');
  const persisted = extractPersistedToolOutput(container);
  if (persisted) {
    pills.push('persisted');
    if (pickNumber(container, 'bytes') === undefined) {
      pills.push(`${persisted.totalBytes} bytes`);
    }
  }

  pushNumberPill(pills, container, ['count'], (value) => `${value} items`);
  pushNumberPill(pills, container, ['returned'], (value) => `${value} returned`);
  pushNumberPill(pills, container, ['total_files', 'totalFiles'], (value) => `${value} files`);
  pushNumberPill(pills, container, ['filesApplied'], (value) => `${value} applied`);
  pushNumberPill(pills, container, ['filesFailed'], (value) => `${value} failed`);
  pushNumberPill(pills, container, ['bytes'], (value) => `${value} bytes`);

  const outputMode = pickString(container, 'output_mode', 'outputMode');
  if (outputMode) {
    pills.push(`mode ${outputMode}`);
  }

  if (pickBoolean(container, 'has_more', 'hasMore') === true) {
    pills.push('has more');
  }
  if (pickBoolean(container, 'truncated') === true) {
    pills.push('truncated');
  }

  if (!message && pills.length === 0) {
    return null;
  }

  return { message: message ?? undefined, pills };
}

function summarizeJsonContainer(value: UnknownRecord | unknown[]): string {
  if (Array.isArray(value)) {
    return `Array (${value.length} items)`;
  }
  return `Object (${Object.keys(value).length} keys)`;
}

export function extractStructuredJsonOutput(
  output: string | undefined
): StructuredJsonOutput | null {
  if (typeof output !== 'string') {
    return null;
  }

  const trimmed = output.trim();
  // 先做首字符判断，避免对明显非 JSON 文本执行无意义的 JSON.parse。
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) {
    return null;
  }

  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (!parsed || typeof parsed !== 'object') {
      return null;
    }
    if (!Array.isArray(parsed)) {
      const record = asRecord(parsed);
      if (!record) {
        return null;
      }
      return {
        value: record,
        summary: summarizeJsonContainer(record),
      };
    }
    return {
      value: parsed,
      summary: summarizeJsonContainer(parsed),
    };
  } catch {
    return null;
  }
}

function summarizePrimitiveArg(value: unknown): string {
  if (typeof value === 'string') {
    return JSON.stringify(value);
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  if (value === null) {
    return 'null';
  }
  if (Array.isArray(value)) {
    return `[${value.length} items]`;
  }
  if (typeof value === 'object') {
    return '{...}';
  }
  return String(value);
}

function prioritizeArgEntries(record: UnknownRecord): Array<[string, unknown]> {
  const preferredOrder = [
    'path',
    'pattern',
    'command',
    'glob',
    'query',
    'fileType',
    'oldStr',
    'newStr',
    'offset',
    'limit',
    'maxChars',
    'maxMatches',
    'replaceAll',
    'recursive',
    'cwd',
    'shell',
  ];
  const preferred = preferredOrder
    .filter((key) => key in record)
    .map((key) => [key, record[key]] as [string, unknown]);
  const seen = new Set(preferred.map(([key]) => key));
  const rest = Object.keys(record)
    .filter((key) => !seen.has(key))
    .sort((left, right) => left.localeCompare(right))
    .map((key) => [key, record[key]] as [string, unknown]);

  return [...preferred, ...rest];
}

function truncateSingleLine(text: string, maxChars: number): string {
  if (text.length <= maxChars) {
    return text;
  }
  return `${text.slice(0, Math.max(0, maxChars - 3))}...`;
}

export function formatToolCallSummary(
  toolName: string,
  args: unknown,
  status: 'running' | 'ok' | 'fail',
  metadata?: unknown
): string {
  const prefix = status === 'running' ? '运行中' : '已运行';
  const shellDisplay = extractToolShellDisplay(metadata);
  if (shellDisplay?.command) {
    return truncateSingleLine(`${prefix} ${toolName} (${shellDisplay.command})`, 180);
  }

  const record = asRecord(args);
  if (!record) {
    if (Array.isArray(args)) {
      return truncateSingleLine(`${prefix} ${toolName} (${args.length} args)`, 180);
    }
    return `${prefix} ${toolName}`;
  }

  const pairs = prioritizeArgEntries(record)
    .slice(0, 4)
    .map(([key, value]) => `${key}=${summarizePrimitiveArg(value)}`);
  const hiddenCount = Math.max(0, Object.keys(record).length - pairs.length);
  const suffix = hiddenCount > 0 ? `, +${hiddenCount}` : '';

  return truncateSingleLine(`${prefix} ${toolName} (${pairs.join(', ')}${suffix})`, 180);
}

export function extractStructuredArgs(args: unknown): StructuredJsonOutput | null {
  if (!args || typeof args !== 'object') {
    return null;
  }
  if (Array.isArray(args)) {
    return {
      value: args,
      summary: summarizeJsonContainer(args),
    };
  }

  const record = asRecord(args);
  if (!record) {
    return null;
  }

  return {
    value: record,
    summary: summarizeJsonContainer(record),
  };
}
