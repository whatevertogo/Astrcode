/**
 * 共享 Tailwind 类组合常量
 *
 * 提供跨组件复用的样式字符串，避免在多个 TSX 中重复长串类名。
 * 条件类名组合应在 TSX 中使用 cn() 完成。
 */

/* ====== 弹窗 / 面板 ====== */

/** 全屏遮罩层 */
export const overlay =
  'fixed inset-0 z-[10000] flex items-center justify-center bg-overlay-backdrop p-5 backdrop-blur-[8px]';

/** 桌面端对话框面板 */
export const dialogSurface = 'rounded-[20px] border border-border bg-surface p-6 shadow-surface-lg';

/** 通用输入框 */
export const fieldInput =
  'w-full rounded-xl border border-border bg-surface px-3 py-[11px] text-[13px] text-text-primary outline-none transition-[border-color,box-shadow,background-color] duration-150 ease-out placeholder:text-text-muted focus:border-border-strong focus:shadow-focus-warm';

/** 通用选择/浏览按钮 */
export const fieldButton =
  'flex w-full items-center justify-between gap-3 rounded-xl border border-border bg-surface px-3 py-[11px] text-[13px] text-text-primary transition-[border-color,background-color,box-shadow] duration-150 ease-out hover:bg-white focus-visible:border-border-strong focus-visible:shadow-focus-warm focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-55';

/* ====== 按钮 ====== */

/** 次要按钮（取消、关闭等） */
export const btnSecondary =
  'rounded-xl border border-border bg-surface-soft px-4 py-2.5 text-[13px] font-semibold text-text-secondary transition-[background-color,border-color,color] duration-150 ease-out hover:border-border-strong hover:bg-white hover:text-text-primary';

/** 主要按钮（确认、提交等） */
export const btnPrimary =
  'rounded-xl border-none bg-accent-strong px-4 py-2.5 text-[13px] font-semibold text-white transition-[filter,opacity] duration-150 ease-out hover:brightness-95 disabled:cursor-not-allowed disabled:opacity-40';

/** 危险按钮 */
export const btnDanger =
  'rounded-xl border-none bg-danger px-4 py-2.5 text-[13px] font-semibold text-white transition-[filter,opacity] duration-150 ease-out hover:brightness-95 disabled:cursor-not-allowed disabled:opacity-40';

/** 轻量图标按钮 */
export const ghostIconButton =
  'inline-flex items-center justify-center rounded-lg text-text-secondary transition-[background-color,color,opacity,transform] duration-150 ease-out hover:bg-black/5 hover:text-text-primary';

/** 禁用占位图标按钮 */
export const disabledIconButton =
  'inline-flex items-center justify-center rounded-full bg-surface-soft text-text-muted opacity-80 cursor-not-allowed';

/** 信息强调按钮 */
export const infoButton =
  'rounded-full border border-info-border bg-info-soft px-3 text-xs font-semibold text-info transition-[background-color,border-color,opacity] duration-150 ease-out hover:border-info hover:bg-white';

/** 次级圆角操作按钮 */
export const subtleActionButton =
  'rounded-full border border-border bg-white px-3 text-xs font-semibold text-text-secondary transition-[background-color,border-color,opacity] duration-150 ease-out hover:border-border-strong hover:bg-surface-soft';

/* ====== 徽章 / Pill ====== */

/** 状态 pill 基础类 */
export const pillBase =
  'inline-flex min-h-[22px] shrink-0 items-center rounded-full px-2.5 text-[11px] font-bold tracking-[0.02em]';

export const pillNeutral = `${pillBase} bg-surface-muted text-text-secondary`;
export const pillSuccess = `${pillBase} bg-success-soft text-success`;
export const pillWarning = `${pillBase} bg-warning-soft text-warning`;
export const pillDanger = `${pillBase} bg-danger-soft text-danger`;
export const pillInfo = `${pillBase} bg-info-soft text-info`;

/* ====== 消息 / 卡片 ====== */

/** 渲染失败或错误提示块 */
export const errorSurface =
  'self-stretch rounded-2xl border border-danger/20 bg-danger-soft px-4 py-3.5 text-danger';

/** 空状态提示块 */
export const emptyStateSurface =
  'rounded-[18px] border border-dashed border-border bg-surface/60 px-7 py-6 text-center text-sm text-text-secondary';

/** 助手头像底座 */
export const assistantAvatar =
  'inline-flex h-7 w-7 shrink-0 items-center justify-center rounded bg-linear-to-b from-avatar-surface to-avatar-surface-strong text-avatar-text';

/** Thinking / expandable body 容器 */
export const expandableBody = 'mb-3 ml-2 mt-2 border-l-2 border-border pl-4';

/** chevron 图标：在 group-open 时旋转 90° */
export const chevronIcon =
  'inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center text-text-secondary opacity-60 transition-transform duration-150 ease-out group-open:rotate-90';

/** 顶栏背景 */
export const topBarShell =
  'relative z-30 flex shrink-0 items-center justify-between gap-4 border-b border-border bg-surface/92 px-[22px] py-3.5 backdrop-blur-[12px] max-[899px]:flex-wrap max-[899px]:px-4';

/** 子执行提示条 */
export const subRunNotice =
  'flex-shrink-0 border-t border-border bg-surface-soft px-6 pb-4 pt-3.5 text-xs leading-relaxed text-text-secondary';

/* ====== 上下文菜单 ====== */

/** 菜单容器 */
export const contextMenu =
  'fixed z-[9999] min-w-[140px] rounded-xl border border-border bg-surface py-1 shadow-soft';

/** 菜单项 */
export const menuItem =
  'block w-full px-3.5 py-2.5 text-left text-[13px] text-text-primary transition-[background-color] duration-100 ease-out hover:bg-surface-soft';

/* ====== 代码 / 终端 ====== */

/** 终端输出容器 */
export const terminalBlock =
  'm-0 max-h-[360px] overflow-auto rounded-lg border border-terminal-border bg-gradient-to-b from-terminal-bg-from to-terminal-bg-to';

/** diff/patch 行 */
export const patchLine = 'min-h-[22px] whitespace-pre px-3.5 font-mono text-xs leading-[22px]';

/** 代码块外层 */
export const codeBlockShell =
  'group relative my-4 overflow-hidden rounded-lg border border-code-border bg-code-surface';

/** 代码块标签栏 */
export const codeBlockHeader =
  'flex items-center justify-between bg-code-surface px-4 pb-1 pt-2 text-xs text-code-label';

/** 代码块内容 */
export const codeBlockContent =
  'm-0 overflow-x-auto px-4 pb-4 pt-2 font-mono text-sm leading-relaxed text-code-text';

/** 行内代码 */
export const inlineCode =
  'rounded-md border border-black/5 bg-black/4 px-[0.35rem] py-[0.1rem] text-[0.92em]';

/* ====== Chat Composer ====== */

/** 输入区外壳 */
export const composerShell =
  'rounded-[24px] border border-border bg-gradient-to-b from-white/95 to-surface-soft shadow-composer-shell transition-[border-color,box-shadow,transform] duration-[180ms] ease-out focus-within:-translate-y-px focus-within:border-accent-soft/60 focus-within:shadow-focus-accent';

/** 附件按钮当前仅作为占位态 */
export const composerAttachmentButton = `${disabledIconButton} h-8 w-8`;

/** 发送按钮 */
export const composerSubmitButton =
  'inline-flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-accent-strong text-white shadow-soft transition-[transform,filter,opacity,box-shadow] duration-150 ease-out hover:-translate-y-px hover:scale-105 hover:brightness-95 focus-visible:outline-none focus-visible:shadow-focus-accent disabled:cursor-not-allowed disabled:opacity-35 [&_svg]:h-4 [&_svg]:w-4';

/** 中断按钮 */
export const composerInterruptButton =
  'h-9 flex-shrink-0 rounded-xl border border-danger bg-danger-soft px-3.5 text-[13px] font-semibold text-danger transition-[filter,opacity] duration-150 ease-out hover:brightness-98';

/* ====== 特殊卡片 ====== */

/** 压缩摘要卡片（绿色调） */
export const compactCard =
  'ml-[var(--chat-assistant-content-offset)] border border-[rgba(122,185,153,0.28)] bg-[linear-gradient(180deg,rgba(245,252,248,0.98)_0%,rgba(237,247,241,0.96)_100%)] rounded-[18px] px-4 pt-3.5 pb-4 shadow-[0_14px_32px_rgba(63,119,88,0.08)]';

/** Prompt 指标卡片（蓝色调） */
export const metricsCard =
  'ml-[var(--chat-assistant-content-offset)] border border-info-border bg-[linear-gradient(180deg,rgba(247,249,255,0.98)_0%,rgba(240,244,255,0.96)_100%)] rounded-[18px] px-4 py-3.5 shadow-code-panel';

/** 压缩摘要徽章（绿色） */
export const compactBadge =
  'inline-flex min-h-[26px] items-center rounded-full bg-[rgba(57,201,143,0.14)] px-2.5 text-xs font-bold tracking-[0.02em] text-[#22694c]';

/** Prompt 指标徽章（蓝色） */
export const metricsBadge =
  'inline-flex min-h-[26px] items-center rounded-full bg-[rgba(89,132,255,0.14)] px-2.5 text-xs font-bold text-[#3558c4]';
