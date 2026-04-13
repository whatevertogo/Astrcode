import type { Phase } from '../types';

const EXECUTION_PHASES: ReadonlySet<Phase> = new Set(['thinking', 'callingTool', 'streaming']);

// Why: `interrupted` / `done` 是终态展示，不应继续锁输入或显示中断按钮。
// 前端交互层只把真正仍在执行中的 phase 视为 busy。
export function isExecutionPhase(phase: Phase): boolean {
  return EXECUTION_PHASES.has(phase);
}
