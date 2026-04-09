/// 兼容历史上可能存在的 `session-` 前缀，统一按 canonical 形式做前端匹配。
/// 这样 child notification / URL / session list 三边 ID 形态不一致时，仍能稳定跳转。
export function normalizeSessionIdForCompare(sessionId: string): string {
  const trimmed = sessionId.trim();
  return trimmed.startsWith('session-') ? trimmed.slice('session-'.length) : trimmed;
}

export function findMatchingSessionId(
  candidates: string[],
  targetSessionId?: string | null
): string | null {
  if (!targetSessionId) {
    return null;
  }

  const canonicalTarget = normalizeSessionIdForCompare(targetSessionId);
  return (
    candidates.find((candidate) => normalizeSessionIdForCompare(candidate) === canonicalTarget) ??
    null
  );
}
