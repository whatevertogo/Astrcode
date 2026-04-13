interface KnownProjectRecord {
  key: string;
  workingDir: string;
  lastSeenAt: number;
}

const KNOWN_PROJECTS_STORAGE_KEY = 'astrcode.knownProjects.v1';

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function normalizeProjectIdentity(workingDir: string): string {
  const trimmed = workingDir.trim();
  if (!trimmed) {
    return '';
  }

  let normalized = trimmed.replace(/\\/g, '/');
  if (/^\/+$/.test(normalized)) {
    return '/';
  }

  const windowsRoot = normalized.match(/^([A-Za-z]):\/+$/);
  if (windowsRoot) {
    return `${windowsRoot[1].toLowerCase()}:/`;
  }

  normalized = normalized.replace(/\/+$/, '');
  if (!normalized) {
    return '/';
  }

  // Windows/UNC 路径在 UI 侧统一按不区分大小写处理，避免同一项目被拆成多个条目。
  if (/^[A-Za-z]:($|\/)/.test(normalized) || normalized.startsWith('//')) {
    return normalized.toLowerCase();
  }

  return normalized;
}

function normalizeRecord(raw: unknown): KnownProjectRecord | null {
  if (!raw || typeof raw !== 'object') {
    return null;
  }

  const candidate = raw as Partial<KnownProjectRecord>;
  const workingDir = typeof candidate.workingDir === 'string' ? candidate.workingDir.trim() : '';
  const key = normalizeProjectIdentity(
    typeof candidate.key === 'string' && candidate.key.trim() ? candidate.key : workingDir
  );
  if (!workingDir || !key) {
    return null;
  }

  const lastSeenAt =
    typeof candidate.lastSeenAt === 'number' && Number.isFinite(candidate.lastSeenAt)
      ? candidate.lastSeenAt
      : 0;

  return {
    key,
    workingDir,
    lastSeenAt,
  };
}

function sortRecords(records: KnownProjectRecord[]): KnownProjectRecord[] {
  return [...records].sort(
    (left, right) =>
      right.lastSeenAt - left.lastSeenAt || left.workingDir.localeCompare(right.workingDir)
  );
}

function loadRecords(): KnownProjectRecord[] {
  const target = storage();
  if (!target) {
    return [];
  }

  try {
    const raw = target.getItem(KNOWN_PROJECTS_STORAGE_KEY);
    if (!raw) {
      return [];
    }

    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }

    const deduped = new Map<string, KnownProjectRecord>();
    for (const item of parsed) {
      const record = normalizeRecord(item);
      if (!record) {
        continue;
      }

      const existing = deduped.get(record.key);
      if (!existing || record.lastSeenAt >= existing.lastSeenAt) {
        deduped.set(record.key, record);
      }
    }

    return sortRecords(Array.from(deduped.values()));
  } catch {
    return [];
  }
}

function saveRecords(records: KnownProjectRecord[]): void {
  const target = storage();
  if (!target) {
    return;
  }

  target.setItem(KNOWN_PROJECTS_STORAGE_KEY, JSON.stringify(sortRecords(records)));
}

export function listKnownProjects(): string[] {
  return loadRecords().map((record) => record.workingDir);
}

export function ensureKnownProjects(workingDirs: string[]): string[] {
  const records = loadRecords();
  const merged = new Map(records.map((record) => [record.key, record]));
  let changed = false;

  for (const workingDir of workingDirs) {
    const trimmed = workingDir.trim();
    const key = normalizeProjectIdentity(trimmed);
    if (!trimmed || !key) {
      continue;
    }

    const existing = merged.get(key);
    if (!existing) {
      merged.set(key, {
        key,
        workingDir: trimmed,
        lastSeenAt: 0,
      });
      changed = true;
      continue;
    }

    if (existing.workingDir !== trimmed) {
      merged.set(key, {
        ...existing,
        workingDir: trimmed,
      });
      changed = true;
    }
  }

  const next = sortRecords(Array.from(merged.values()));
  if (changed) {
    saveRecords(next);
  }
  return next.map((record) => record.workingDir);
}

export function rememberProject(workingDir: string): string[] {
  const trimmed = workingDir.trim();
  const key = normalizeProjectIdentity(trimmed);
  if (!trimmed || !key) {
    return listKnownProjects();
  }

  const records = loadRecords();
  const merged = new Map(records.map((record) => [record.key, record]));
  merged.set(key, {
    key,
    workingDir: trimmed,
    lastSeenAt: Date.now(),
  });
  const next = sortRecords(Array.from(merged.values()));
  saveRecords(next);
  return next.map((record) => record.workingDir);
}

export function forgetProject(workingDir: string): string[] {
  const key = normalizeProjectIdentity(workingDir);
  if (!key) {
    return listKnownProjects();
  }

  const next = loadRecords().filter((record) => record.key !== key);
  saveRecords(next);
  return next.map((record) => record.workingDir);
}
