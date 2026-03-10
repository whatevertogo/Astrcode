export function resolveSessionForTurn(
  turnSessionMap: Record<string, string>,
  pendingSubmitSessions: string[],
  turnId: string | null | undefined,
  activeSessionId: string | null
): string | null {
  if (!turnId) {
    return activeSessionId;
  }

  const mapped = turnSessionMap[turnId];
  if (mapped) {
    return mapped;
  }

  const queued = pendingSubmitSessions.shift() ?? null;
  const fallback = queued ?? activeSessionId;
  if (fallback) {
    turnSessionMap[turnId] = fallback;
  }

  return fallback;
}

export function releaseTurnMapping(
  turnSessionMap: Record<string, string>,
  turnId: string | null | undefined
): void {
  if (!turnId) {
    return;
  }
  delete turnSessionMap[turnId];
}
