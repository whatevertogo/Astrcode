import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import PlanMessage from './PlanMessage';

describe('PlanMessage', () => {
  it('renders a presented plan card', () => {
    const html = renderToStaticMarkup(
      <PlanMessage
        message={{
          id: 'plan-1',
          kind: 'plan',
          toolCallId: 'call-plan-presented',
          eventKind: 'presented',
          title: 'Cleanup crates',
          planPath:
            'D:/GitObjectsOwn/Astrcode/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md',
          status: 'awaiting_approval',
          content: '# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent',
          blockers: {
            missingHeadings: [],
            invalidSections: [],
          },
          timestamp: Date.now(),
        }}
      />
    );

    expect(html).toContain('计划已呈递');
    expect(html).toContain('待确认');
    expect(html).toContain('Cleanup crates');
    expect(html).toContain('cleanup-crates.md');
  });

  it('renders a plan update card for saved plan artifacts', () => {
    const html = renderToStaticMarkup(
      <PlanMessage
        message={{
          id: 'plan-2',
          kind: 'plan',
          toolCallId: 'call-plan-save',
          eventKind: 'saved',
          title: 'Cleanup crates',
          planPath:
            'D:/GitObjectsOwn/Astrcode/.astrcode/projects/demo/sessions/session-1/plan/cleanup-crates.md',
          status: 'draft',
          summary: 'updated session plan',
          blockers: {
            missingHeadings: [],
            invalidSections: [],
          },
          timestamp: Date.now(),
        }}
      />
    );

    expect(html).toContain('计划已更新');
    expect(html).toContain('草稿');
    expect(html).toContain('updated session plan');
  });
});
