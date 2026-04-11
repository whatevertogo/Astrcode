import { memo } from 'react';
import type { ReactNode } from 'react';
import type { UnknownRecord } from '../../lib/shared';

const MAX_CHILDREN_PER_NODE = 200;
const MAX_STRING_PREVIEW = 240;

interface ToolJsonViewProps {
  value: UnknownRecord | unknown[];
  summary: string;
}

interface JsonNodeProps {
  value: unknown;
  label: string;
  path: string;
  defaultOpen?: boolean;
}

function isObjectContainer(value: unknown): value is UnknownRecord {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function summarizeContainer(value: UnknownRecord | unknown[]): string {
  if (Array.isArray(value)) {
    return `Array (${value.length})`;
  }
  return `Object (${Object.keys(value).length})`;
}

function renderPrimitiveValue(value: unknown): ReactNode {
  if (value === null) {
    return <span className="text-text-secondary">null</span>;
  }

  if (typeof value === 'string') {
    const truncated =
      value.length > MAX_STRING_PREVIEW
        ? `${value.slice(0, MAX_STRING_PREVIEW)}... (${value.length} chars)`
        : value;
    return (
      <span className="whitespace-pre-wrap overflow-wrap-anywhere text-json-string">
        &quot;{truncated}&quot;
      </span>
    );
  }

  if (typeof value === 'number') {
    return <span className="text-json-number">{String(value)}</span>;
  }

  if (typeof value === 'boolean') {
    return <span className="text-json-boolean">{String(value)}</span>;
  }

  // JSON.parse 产物通常不会到这里，兜底展示
  return <span className="text-text-secondary">{String(value)}</span>;
}

function JsonNode({ value, label, path, defaultOpen = false }: JsonNodeProps) {
  if (!Array.isArray(value) && !isObjectContainer(value)) {
    return (
      <div className="flex items-baseline gap-1.5 px-3 py-1 text-text-primary">
        <span className="break-words text-json-key">{label}</span>
        <span className="text-text-secondary">:</span>
        {renderPrimitiveValue(value)}
      </div>
    );
  }

  const entries: Array<readonly [string, unknown]> = Array.isArray(value)
    ? value.map((entry, index) => [String(index), entry] as const)
    : Object.keys(value).map((key) => [key, value[key]] as const);
  const visibleEntries = entries.slice(0, MAX_CHILDREN_PER_NODE);
  const hiddenCount = entries.length - visibleEntries.length;

  return (
    <details className="m-0 group" open={defaultOpen}>
      <summary className="flex items-baseline gap-1.5 px-3 py-2 cursor-pointer text-text-primary list-none [&::-webkit-details-marker]:hidden before:content-['▸'] before:text-text-secondary before:transition-transform before:duration-120 before:ease-out group-open:before:rotate-90">
        <span className="break-words text-json-key">{label}</span>
        <span className="text-text-secondary">:</span>
        <span className="text-text-secondary">{summarizeContainer(value)}</span>
      </summary>
      <div className="ml-[18px] pl-3 border-l border-dashed border-border">
        {visibleEntries.map(([childLabel, childValue]) => (
          <JsonNode
            key={`${path}.${childLabel}`}
            value={childValue}
            label={Array.isArray(value) ? `[${childLabel}]` : childLabel}
            path={`${path}.${childLabel}`}
          />
        ))}
        {hiddenCount > 0 && (
          <div className="px-3 py-1.5 pb-2.5 text-text-secondary text-xs">
            ... {hiddenCount} more entries hidden
          </div>
        )}
      </div>
    </details>
  );
}

function ToolJsonView({ value, summary }: ToolJsonViewProps) {
  return (
    <div className="m-0 max-h-[360px] overflow-auto rounded-lg border border-code-border bg-code-surface font-mono text-[13px]">
      <JsonNode value={value} label="JSON" path="root" defaultOpen={false} />
      <div className="px-3 py-2 border-t border-border text-text-secondary text-xs">{summary}</div>
    </div>
  );
}

export default memo(ToolJsonView);
