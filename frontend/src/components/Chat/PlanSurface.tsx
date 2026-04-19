import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { pillNeutral, pillSuccess } from '../../lib/styles';
import type { PlanReviewState } from '../../types';

interface PlanSurfaceModeTransition {
  fromModeId: string;
  toModeId: string;
  modeChanged: boolean;
}

interface PlanSurfaceBlockers {
  missingHeadings: string[];
  invalidSections: string[];
}

export function planStatusLabel(status: string): string {
  switch (status) {
    case 'awaiting_approval':
      return '待确认';
    case 'approved':
      return '已批准';
    case 'completed':
      return '已完成';
    case 'draft':
      return '草稿';
    case 'superseded':
      return '已替换';
    default:
      return status;
  }
}

export function PresentedPlanSurface({
  title,
  status,
  planPath,
  content,
  mode,
}: {
  title: string;
  status?: string;
  planPath: string;
  content: string;
  mode?: PlanSurfaceModeTransition;
}) {
  return (
    <section className="flex min-w-0 flex-col gap-3 rounded-2xl border border-success/25 bg-success-soft px-4 py-3.5 shadow-[0_12px_28px_rgba(15,23,42,0.05)]">
      <div className="flex flex-wrap items-center gap-2 text-xs text-text-secondary">
        <span className={pillSuccess}>计划已呈递</span>
        {status ? <span className={pillNeutral}>{planStatusLabel(status)}</span> : null}
        {mode?.modeChanged ? (
          <span className={pillNeutral}>
            {mode.fromModeId} -&gt; {mode.toModeId}
          </span>
        ) : null}
      </div>
      <div className="space-y-1">
        <div className="text-sm font-semibold text-text-primary">{title}</div>
        <div className="break-all font-mono text-[12px] text-text-muted">{planPath}</div>
        <div className="text-[13px] leading-relaxed text-text-secondary">
          计划已经提交给你审核。你可以直接批准，或者要求继续修订。
        </div>
      </div>
      <div className="min-w-0 max-w-full break-words rounded-[18px] border border-border bg-white/80 px-4 py-3 text-sm leading-[1.7] text-text-primary prose-chat [&_ol]:my-[0.4rem] [&_ol]:pl-[1.25rem] [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 [&_ul]:my-[0.4rem] [&_ul]:pl-[1.25rem]">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
      </div>
    </section>
  );
}

export function ReviewPendingPlanSurface({
  title,
  planPath,
  review,
  blockers,
}: {
  title: string;
  planPath: string;
  review?: PlanReviewState;
  blockers: PlanSurfaceBlockers;
}) {
  return (
    <section className="flex min-w-0 flex-col gap-3 rounded-2xl border border-border bg-white/80 px-4 py-3.5 shadow-[0_12px_28px_rgba(15,23,42,0.05)]">
      <div className="flex flex-wrap items-center gap-2 text-xs text-text-secondary">
        <span className={pillNeutral}>继续完善中</span>
        <span className={pillNeutral}>
          {review?.kind === 'revise_plan' ? '正在修计划' : '正在做退出前自审'}
        </span>
      </div>
      <div className="space-y-1">
        <div className="text-sm font-semibold text-text-primary">{title}</div>
        <div className="break-all font-mono text-[12px] text-text-muted">{planPath}</div>
        <div className="text-[13px] leading-relaxed text-text-secondary">
          {review?.kind === 'revise_plan'
            ? '当前计划还没达到可执行程度。模型会先补强计划，再重新尝试退出 plan mode。'
            : '模型正在做退出前的内部最终自审。这是正常流程，不会把 review 段落写进计划正文。'}
        </div>
      </div>
      <div className="space-y-2 text-[13px] leading-relaxed text-text-primary">
        {review?.checklist && review.checklist.length > 0 ? (
          <div>
            <div className="text-xs text-text-secondary">自审检查项</div>
            <div>{review.checklist.join('；')}</div>
          </div>
        ) : null}
        {blockers.missingHeadings.length > 0 ? (
          <div>
            <div className="text-xs text-text-secondary">缺失章节</div>
            <div>{blockers.missingHeadings.join('，')}</div>
          </div>
        ) : null}
        {blockers.invalidSections.length > 0 ? (
          <div>
            <div className="text-xs text-text-secondary">需要加强</div>
            <div>{blockers.invalidSections.join('；')}</div>
          </div>
        ) : null}
      </div>
    </section>
  );
}
