/**
 * 共享 Tailwind 类组合常量
 *
 * 提供跨组件复用的样式字符串，避免在多个 TSX 中重复长串类名。
 * 条件类名组合应在 TSX 中使用 cn() 完成。
 */

/* ====== 弹窗/遮罩 ====== */

/** 全屏遮罩层 */
export const overlay =
  'fixed inset-0 z-[10000] flex items-center justify-center p-5 bg-[rgba(55,42,26,0.18)] backdrop-blur-[8px]';

/* ====== 按钮 ====== */

/** 次要按钮（取消、关闭等） */
export const btnSecondary =
  'px-4 py-2.5 text-[13px] font-semibold rounded-xl border border-border bg-surface-soft text-text-secondary transition-[background-color,border-color,color] duration-150 ease-out hover:bg-white hover:border-border-strong hover:text-text-primary';

/** 主要按钮（确认、提交等） */
export const btnPrimary =
  'px-4 py-2.5 text-[13px] font-semibold rounded-xl border-none bg-accent-strong text-white transition-[background-color,opacity] duration-150 ease-out hover:bg-[#1f1b17] disabled:opacity-40 disabled:cursor-not-allowed';

/** 危险按钮 */
export const btnDanger =
  'px-4 py-2.5 text-[13px] font-semibold rounded-xl border-none bg-danger text-white transition-[background-color,opacity] duration-150 ease-out hover:bg-[#b54a4a] disabled:opacity-40 disabled:cursor-not-allowed';

/* ====== 徽章/Pill ====== */

/** 状态 pill 基础类 */
export const pillBase =
  'inline-flex items-center min-h-[22px] px-2.5 rounded-full text-[11px] font-bold tracking-[0.02em] shrink-0';

/* ====== 上下文菜单 ====== */

/** 菜单容器 */
export const contextMenu =
  'fixed z-[9999] bg-surface border border-border rounded-xl py-1 min-w-[140px] shadow-[0_18px_40px_rgba(86,65,39,0.16)]';

/** 菜单项 */
export const menuItem =
  'block w-full px-3.5 py-2.5 text-left text-[13px] text-text-primary transition-[background-color] duration-100 ease-out hover:bg-surface-soft';

/* ====== 代码/终端 ====== */

/** 终端输出容器 */
export const terminalBlock =
  'm-0 border border-border rounded-lg overflow-auto max-h-[360px] bg-surface';

/** diff/patch 行 */
export const patchLine = 'px-3.5 min-h-[22px] font-mono text-xs leading-[22px] whitespace-pre';

/* ====== 展开/折叠 ====== */

/** details 展开后的 body 容器（带左侧竖线） */
export const expandableBody = 'mt-2 mb-3 ml-2 pl-4 border-l-2 border-[rgba(0,0,0,0.1)]';

/** chevron 图标：在 group-open 时旋转 90° */
export const chevronIcon =
  'w-3.5 h-3.5 inline-flex items-center justify-center shrink-0 text-text-secondary opacity-60 transition-transform duration-150 ease-out group-open:rotate-90';
