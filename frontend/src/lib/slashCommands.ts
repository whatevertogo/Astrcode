//! # Slash Commands
//!
//! 这里专门收敛前端自执行的 slash command 解析规则，避免把 UI 文本判断散落在组件里。

export type RuntimeSlashCommand = { kind: 'compact' } | { kind: 'compactInvalidArgs' };

export function parseRuntimeSlashCommand(input: string): RuntimeSlashCommand | null {
  const trimmed = input.trim();
  if (trimmed === '/compact') {
    return { kind: 'compact' };
  }

  // v1 只支持独立命令。这里显式拦截附带参数的写法，避免前端悄悄把未知语义发给后端。
  if (/^\/compact\s+.+$/u.test(trimmed)) {
    return { kind: 'compactInvalidArgs' };
  }

  return null;
}
