//! # Slash Commands
//!
//! 这里专门收敛前端自执行的 slash command 解析规则，避免把 UI 文本判断散落在组件里。

export type RuntimeSlashCommand =
  | { kind: 'compact'; instructions?: string }
  | { kind: 'compactInvalidArgs' };

export function parseRuntimeSlashCommand(input: string): RuntimeSlashCommand | null {
  const trimmed = input.trim();
  if (trimmed === '/compact') {
    return { kind: 'compact' };
  }

  const compactWithArgs = trimmed.match(/^\/compact(?:\s+(.*))$/u);
  if (compactWithArgs) {
    const instructions = compactWithArgs[1]?.trim();
    if (!instructions) {
      return { kind: 'compact' };
    }
    return { kind: 'compact', instructions };
  }

  return null;
}
