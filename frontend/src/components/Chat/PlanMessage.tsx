import { memo } from 'react';

import type { PlanMessage as PlanMessageType } from '../../types';
import { pillNeutral } from '../../lib/styles';
import { PresentedPlanSurface, ReviewPendingPlanSurface, planStatusLabel } from './PlanSurface';

interface PlanMessageProps {
  message: PlanMessageType;
}

function PlanMessage({ message }: PlanMessageProps) {
  return (
    <section className="mb-2 ml-[var(--chat-assistant-content-offset)] block min-w-0 max-w-full animate-block-enter motion-reduce:animate-none">
      {message.eventKind === 'presented' && message.content ? (
        <PresentedPlanSurface
          title={message.title}
          status={message.status}
          planPath={message.planPath}
          content={message.content}
        />
      ) : message.eventKind === 'review_pending' ? (
        <ReviewPendingPlanSurface
          title={message.title}
          planPath={message.planPath}
          review={message.review}
          blockers={message.blockers}
        />
      ) : (
        <section className="flex min-w-0 flex-col gap-3 rounded-2xl border border-border bg-white/80 px-4 py-3.5 shadow-[0_12px_28px_rgba(15,23,42,0.05)]">
          <div className="flex flex-wrap items-center gap-2 text-xs text-text-secondary">
            <span className={pillNeutral}>计划已更新</span>
            {message.status ? (
              <span className={pillNeutral}>{planStatusLabel(message.status)}</span>
            ) : null}
          </div>
          <div className="space-y-1">
            <div className="text-sm font-semibold text-text-primary">{message.title}</div>
            <div className="break-all font-mono text-[12px] text-text-muted">
              {message.planPath}
            </div>
            <div className="text-[13px] leading-relaxed text-text-secondary">
              {message.summary ?? 'canonical session plan 已同步。'}
            </div>
          </div>
        </section>
      )}
    </section>
  );
}

export default memo(PlanMessage);
