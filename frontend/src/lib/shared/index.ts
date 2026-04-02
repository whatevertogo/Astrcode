//! # Shared Picker Utilities
//!
//! Type-safe helpers for extracting values from unknown records.
//!
//! ## Why This Module Exists
//!
//! These utilities were duplicated across `agentEvent.ts` and `useAgent.ts`.
//! Keeping them here eliminates drift and ensures consistent behavior across
//! the frontend codebase.

type UnknownRecord = Record<string, unknown>;

export function asRecord(value: unknown): UnknownRecord | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as UnknownRecord;
}

export function pickString(record: UnknownRecord, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.length > 0) {
      return value;
    }
  }
  return null;
}

export function pickStringOrUndefined(
  record: UnknownRecord,
  ...keys: string[]
): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string') {
      return value;
    }
  }
  return undefined;
}

export function pickOptionalString(
  record: UnknownRecord,
  ...keys: string[]
): string | null | undefined {
  for (const key of keys) {
    if (!(key in record)) {
      continue;
    }
    const value = record[key];
    if (value === null || value === undefined) {
      return null;
    }
    if (typeof value === 'string') {
      return value;
    }
    return undefined;
  }
  return undefined;
}

export function pickStringAllowEmpty(record: UnknownRecord, ...keys: string[]): string | undefined {
  for (const key of keys) {
    if (!(key in record)) {
      continue;
    }
    const value = record[key];
    if (typeof value === 'string') {
      return value;
    }
    return undefined;
  }
  return undefined;
}

export function pickNumber(record: UnknownRecord, ...keys: string[]): number | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value;
    }
  }
  return null;
}

export function pickNumberOrUndefined(
  record: UnknownRecord,
  ...keys: string[]
): number | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value;
    }
  }
  return undefined;
}

export function pickBoolean(record: UnknownRecord, ...keys: string[]): boolean | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'boolean') {
      return value;
    }
  }
  return undefined;
}

export function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
