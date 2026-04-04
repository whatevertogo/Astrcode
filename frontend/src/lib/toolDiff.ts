export interface ToolDiffMetadata {
  path?: string;
  changeType?: string;
  bytes?: number;
  patch: string;
  addedLines?: number;
  removedLines?: number;
  truncated: boolean;
  hasChanges: boolean;
}

export type ToolDiffLineKind = 'meta' | 'header' | 'add' | 'remove' | 'note' | 'context';

// 工具函数已迁移到 lib/shared/index.ts，此处统一引用
import { asRecord, pickNumberOrUndefined as pickNumber } from './shared';

export function extractToolDiffMetadata(metadata: unknown): ToolDiffMetadata | null {
  const container = asRecord(metadata);
  const diff = asRecord(container?.diff);
  if (!container || !diff || typeof diff.patch !== 'string' || diff.patch.length === 0) {
    return null;
  }

  return {
    path: typeof container.path === 'string' ? container.path : undefined,
    changeType: typeof container.changeType === 'string' ? container.changeType : undefined,
    bytes: pickNumber(container, 'bytes'),
    patch: diff.patch,
    addedLines: pickNumber(diff, 'addedLines'),
    removedLines: pickNumber(diff, 'removedLines'),
    truncated: diff.truncated === true,
    hasChanges: diff.hasChanges === true,
  };
}

export function classifyToolDiffLine(line: string): ToolDiffLineKind {
  if (line.startsWith('+++') || line.startsWith('---')) {
    return 'meta';
  }
  if (line.startsWith('@@')) {
    return 'header';
  }
  if (line.startsWith('+')) {
    return 'add';
  }
  if (line.startsWith('-')) {
    return 'remove';
  }
  if (line.startsWith('...')) {
    return 'note';
  }
  return 'context';
}
