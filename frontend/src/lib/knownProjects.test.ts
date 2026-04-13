import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  ensureKnownProjects,
  forgetProject,
  listKnownProjects,
  normalizeProjectIdentity,
  rememberProject,
} from './knownProjects';

class MemoryStorage implements Storage {
  private map = new Map<string, string>();

  get length(): number {
    return this.map.size;
  }

  clear(): void {
    this.map.clear();
  }

  getItem(key: string): string | null {
    return this.map.get(key) ?? null;
  }

  key(index: number): string | null {
    return Array.from(this.map.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.map.delete(key);
  }

  setItem(key: string, value: string): void {
    this.map.set(key, value);
  }
}

const originalLocalStorage = globalThis.localStorage;

beforeEach(() => {
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: new MemoryStorage(),
  });
});

afterEach(() => {
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: originalLocalStorage,
  });
});

describe('knownProjects', () => {
  it('normalizes windows paths into a stable identity', () => {
    expect(normalizeProjectIdentity('D:\\Repo\\')).toBe('d:/repo');
    expect(normalizeProjectIdentity('d:/repo')).toBe('d:/repo');
  });

  it('deduplicates remembered projects by normalized identity', () => {
    rememberProject('D:\\Repo');
    rememberProject('d:/repo/');

    expect(listKnownProjects()).toEqual(['d:/repo/']);
  });

  it('keeps session discovered projects and forgets explicit deletions', () => {
    ensureKnownProjects(['D:\\Alpha', 'D:\\Beta']);
    forgetProject('d:/alpha');

    expect(listKnownProjects()).toEqual(['D:\\Beta']);
  });
});
