import { memo } from 'react';
import type { ReactNode } from 'react';
import type { UnknownRecord } from '../../lib/shared';
import styles from './ToolJsonView.module.css';

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
    return <span className={styles.valueNull}>null</span>;
  }

  if (typeof value === 'string') {
    const truncated =
      value.length > MAX_STRING_PREVIEW
        ? `${value.slice(0, MAX_STRING_PREVIEW)}... (${value.length} chars)`
        : value;
    return <span className={styles.valueString}>&quot;{truncated}&quot;</span>;
  }

  if (typeof value === 'number') {
    return <span className={styles.valueNumber}>{String(value)}</span>;
  }

  if (typeof value === 'boolean') {
    return <span className={styles.valueBoolean}>{String(value)}</span>;
  }

  // JSON.parse 产物通常不会到这里，这里兜底展示，避免出现空白节点。
  return <span className={styles.valueUnknown}>{String(value)}</span>;
}

function JsonNode({ value, label, path, defaultOpen = false }: JsonNodeProps) {
  if (!Array.isArray(value) && !isObjectContainer(value)) {
    return (
      <div className={styles.row}>
        <span className={styles.key}>{label}</span>
        <span className={styles.separator}>:</span>
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
    <details className={styles.node} open={defaultOpen}>
      <summary className={styles.summary}>
        <span className={styles.key}>{label}</span>
        <span className={styles.separator}>:</span>
        <span className={styles.kind}>{summarizeContainer(value)}</span>
      </summary>
      <div className={styles.children}>
        {visibleEntries.map(([childLabel, childValue]) => (
          <JsonNode
            key={`${path}.${childLabel}`}
            value={childValue}
            label={Array.isArray(value) ? `[${childLabel}]` : childLabel}
            path={`${path}.${childLabel}`}
          />
        ))}
        {hiddenCount > 0 && (
          <div className={styles.truncated}>... {hiddenCount} more entries hidden</div>
        )}
      </div>
    </details>
  );
}

function ToolJsonView({ value, summary }: ToolJsonViewProps) {
  return (
    <div className={styles.container}>
      <JsonNode value={value} label="JSON" path="root" defaultOpen />
      <div className={styles.footer}>{summary}</div>
    </div>
  );
}

export default memo(ToolJsonView);
