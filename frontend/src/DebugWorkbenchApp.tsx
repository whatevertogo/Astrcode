import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  getDebugRuntimeOverview,
  getDebugRuntimeTimeline,
  getDebugSessionAgents,
  getDebugSessionTrace,
} from './lib/api/runtime';
import { listSessionsWithMeta } from './lib/api/sessions';
import {
  buildGovernanceSparklinePoints,
  formatRatioBps,
  isDebugWorkbenchEnabled,
} from './lib/debugWorkbench';
import { getHostBridge } from './lib/hostBridge';
import { logger } from './lib/logger';
import { cn } from './lib/utils';
import type {
  RuntimeDebugOverview,
  RuntimeDebugTimeline,
  SessionDebugAgentNode,
  SessionDebugAgents,
  SessionDebugTrace,
  SessionMeta,
} from './types';

const OVERVIEW_POLL_INTERVAL_MS = 2_000;
const SESSION_POLL_INTERVAL_MS = 2_000;
const SESSION_LIST_POLL_INTERVAL_MS = 5_000;
const TREND_CHART_WIDTH = 720;
const TREND_CHART_HEIGHT = 168;

function ratioTone(value?: number | null): string {
  if (value == null) {
    return 'bg-surface text-text-secondary';
  }
  if (value >= 7_000) {
    return 'bg-success-soft text-success';
  }
  if (value >= 4_000) {
    return 'bg-info-soft text-info';
  }
  return 'bg-danger-soft text-danger';
}

function lifecycleTone(lifecycle: string): string {
  switch (lifecycle) {
    case 'running':
      return 'bg-info-soft text-info';
    case 'idle':
      return 'bg-success-soft text-success';
    case 'terminated':
      return 'bg-surface text-text-secondary';
    default:
      return 'bg-warning-soft text-warning';
  }
}

function formatTimestamp(value?: string | null): string {
  if (!value) {
    return '—';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString('zh-CN', {
    hour12: false,
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function readInitialSessionId(): string | null {
  const query = new URLSearchParams(window.location.search);
  const sessionId = query.get('sessionId')?.trim();
  return sessionId || null;
}

function updateWorkbenchQuery(sessionId: string | null): void {
  const url = new URL(window.location.href);
  if (sessionId) {
    url.searchParams.set('sessionId', sessionId);
  } else {
    url.searchParams.delete('sessionId');
  }
  window.history.replaceState({}, document.title, `${url.pathname}${url.search}${url.hash}`);
}

function buildSessionListSignature(sessions: SessionMeta[]): string {
  return sessions
    .map(
      (session) =>
        `${session.sessionId}|${session.updatedAt}|${session.phase}|${session.title}|${session.parentSessionId ?? ''}`
    )
    .join('\n');
}

function buildSessionTraceSignature(trace: SessionDebugTrace | null): string {
  if (!trace) {
    return '';
  }
  return [
    trace.sessionId,
    trace.phase,
    trace.parentSessionId ?? '',
    ...trace.items.map(
      (item) =>
        [
          item.id,
          item.storageSeq,
          item.turnId ?? '',
          item.recordedAt ?? '',
          item.kind,
          item.title,
          item.summary,
          item.agentId ?? '',
          item.subRunId ?? '',
          item.childAgentId ?? '',
          item.deliveryId ?? '',
          item.toolCallId ?? '',
          item.toolName ?? '',
          item.lifecycle ?? '',
          item.lastTurnOutcome ?? '',
        ].join('|')
    ),
  ].join('\n');
}

function buildSessionAgentsSignature(agents: SessionDebugAgents | null): string {
  if (!agents) {
    return '';
  }
  return [
    agents.sessionId,
    agents.title,
    ...agents.nodes.map(
      (node) =>
        [
          node.nodeId,
          node.kind,
          node.title,
          node.agentId,
          node.sessionId,
          node.childSessionId ?? '',
          node.subRunId ?? '',
          node.parentAgentId ?? '',
          node.parentSessionId ?? '',
          node.depth,
          node.lifecycle,
          node.lastTurnOutcome ?? '',
          node.statusSource ?? '',
          node.lineageKind ?? '',
        ].join('|')
    ),
  ].join('\n');
}

export default function DebugWorkbenchApp() {
  const [overview, setOverview] = useState<RuntimeDebugOverview | null>(null);
  const [timeline, setTimeline] = useState<RuntimeDebugTimeline | null>(null);
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(() =>
    readInitialSessionId()
  );
  const [trace, setTrace] = useState<SessionDebugTrace | null>(null);
  const [agents, setAgents] = useState<SessionDebugAgents | null>(null);
  const [overviewError, setOverviewError] = useState<string | null>(null);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const [sessionListError, setSessionListError] = useState<string | null>(null);
  const [overviewLoading, setOverviewLoading] = useState(false);
  const [sessionLoading, setSessionLoading] = useState(false);
  const sessionListSignatureRef = useRef('');
  const traceSignatureRef = useRef('');
  const agentsSignatureRef = useRef('');

  useEffect(() => {
    document.title = 'AstrCode Debug Workbench';
  }, []);

  useEffect(() => {
    updateWorkbenchQuery(selectedSessionId);
  }, [selectedSessionId]);

  useEffect(() => {
    if (!isDebugWorkbenchEnabled()) {
      logger.warn('DebugWorkbench', 'debug workbench opened without debug flag');
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    const bridge = getHostBridge();
    if (!bridge.isDesktopHost) {
      return;
    }

    void (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        unlisten = await listen<string | null>('debug-workbench:set-session', (event) => {
          if (cancelled) {
            return;
          }
          const nextSessionId = typeof event.payload === 'string' ? event.payload.trim() : '';
          if (nextSessionId) {
            setSelectedSessionId(nextSessionId);
          }
        });
      } catch (error) {
        logger.warn('DebugWorkbench', 'failed to subscribe to debug workbench session events', {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: number | null = null;

    const loadSessions = async () => {
      try {
        const nextSessions = await listSessionsWithMeta();
        if (cancelled) {
          return;
        }
        const nextSignature = buildSessionListSignature(nextSessions);
        if (nextSignature !== sessionListSignatureRef.current) {
          sessionListSignatureRef.current = nextSignature;
          setSessions(nextSessions);
        }
        setSessionListError(null);
        setSelectedSessionId((current) => {
          if (current && nextSessions.some((session) => session.sessionId === current)) {
            return current;
          }
          return nextSessions[0]?.sessionId ?? null;
        });
      } catch (error) {
        if (!cancelled) {
          setSessionListError(error instanceof Error ? error.message : String(error));
        }
      } finally {
        if (!cancelled) {
          timer = window.setTimeout(() => {
            void loadSessions();
          }, SESSION_LIST_POLL_INTERVAL_MS);
        }
      }
    };

    void loadSessions();

    return () => {
      cancelled = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: number | null = null;

    const loadOverview = async () => {
      setOverviewLoading(true);
      try {
        const [nextOverview, nextTimeline] = await Promise.all([
          getDebugRuntimeOverview(),
          getDebugRuntimeTimeline(),
        ]);
        if (cancelled) {
          return;
        }
        setOverview(nextOverview);
        setTimeline(nextTimeline);
        setOverviewError(null);
      } catch (error) {
        if (!cancelled) {
          setOverviewError(error instanceof Error ? error.message : String(error));
        }
      } finally {
        if (!cancelled) {
          setOverviewLoading(false);
          timer = window.setTimeout(() => {
            void loadOverview();
          }, OVERVIEW_POLL_INTERVAL_MS);
        }
      }
    };

    void loadOverview();

    return () => {
      cancelled = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    };
  }, []);

  useEffect(() => {
    if (!selectedSessionId) {
      traceSignatureRef.current = '';
      agentsSignatureRef.current = '';
      setTrace(null);
      setAgents(null);
      setSessionError(null);
      setSessionLoading(false);
      return;
    }

    traceSignatureRef.current = '';
    agentsSignatureRef.current = '';
    setTrace(null);
    setAgents(null);
    setSessionError(null);
    setSessionLoading(true);
    let cancelled = false;
    let timer: number | null = null;
    let isInitialLoad = true;

    const loadSessionDebug = async () => {
      try {
        const [nextTrace, nextAgents] = await Promise.all([
          getDebugSessionTrace(selectedSessionId),
          getDebugSessionAgents(selectedSessionId),
        ]);
        if (cancelled) {
          return;
        }
        const nextTraceSignature = buildSessionTraceSignature(nextTrace);
        if (nextTraceSignature !== traceSignatureRef.current) {
          traceSignatureRef.current = nextTraceSignature;
          setTrace(nextTrace);
        }
        const nextAgentsSignature = buildSessionAgentsSignature(nextAgents);
        if (nextAgentsSignature !== agentsSignatureRef.current) {
          agentsSignatureRef.current = nextAgentsSignature;
          setAgents(nextAgents);
        }
        setSessionError(null);
      } catch (error) {
        if (!cancelled) {
          setSessionError(error instanceof Error ? error.message : String(error));
        }
      } finally {
        if (!cancelled) {
          if (isInitialLoad) {
            setSessionLoading(false);
            isInitialLoad = false;
          }
          timer = window.setTimeout(() => {
            void loadSessionDebug();
          }, SESSION_POLL_INTERVAL_MS);
        }
      }
    };

    void loadSessionDebug();

    return () => {
      cancelled = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    };
  }, [selectedSessionId]);

  const selectedSessionMeta = useMemo(
    () => sessions.find((session) => session.sessionId === selectedSessionId) ?? null,
    [selectedSessionId, sessions]
  );
  const collaboration = overview?.metrics.agentCollaboration;
  const timelineSamples = useMemo(
    () =>
      (timeline?.samples ?? []).map((sample) => ({
        timestamp: new Date(sample.collectedAt).getTime(),
        spawnRejectionRatioBps: sample.spawnRejectionRatioBps,
        observeToActionRatioBps: sample.observeToActionRatioBps,
        childReuseRatioBps: sample.childReuseRatioBps,
      })),
    [timeline]
  );
  const spawnTrendPoints = useMemo(
    () =>
      buildGovernanceSparklinePoints(
        timelineSamples,
        (sample) => sample.spawnRejectionRatioBps,
        TREND_CHART_WIDTH,
        TREND_CHART_HEIGHT
      ),
    [timelineSamples]
  );
  const observeTrendPoints = useMemo(
    () =>
      buildGovernanceSparklinePoints(
        timelineSamples,
        (sample) => sample.observeToActionRatioBps,
        TREND_CHART_WIDTH,
        TREND_CHART_HEIGHT
      ),
    [timelineSamples]
  );
  const reuseTrendPoints = useMemo(
    () =>
      buildGovernanceSparklinePoints(
        timelineSamples,
        (sample) => sample.childReuseRatioBps,
        TREND_CHART_WIDTH,
        TREND_CHART_HEIGHT
      ),
    [timelineSamples]
  );
  const agentNodes = useMemo(
    () =>
      [...(agents?.nodes ?? [])].sort((left, right) => {
        if (left.depth !== right.depth) {
          return left.depth - right.depth;
        }
        return left.title.localeCompare(right.title, 'zh-CN');
      }),
    [agents]
  );

  return (
    <div className="flex min-h-screen min-w-0 flex-col bg-app-bg text-text-primary">
      <header className="border-b border-border bg-surface/92 px-6 py-4 backdrop-blur">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-text-muted">
              Debug Only
            </div>
            <h1 className="mt-1 text-2xl font-semibold text-text-primary">Debug Workbench</h1>
            <p className="mt-1 text-sm text-text-secondary">
              独立调试窗口直接读取 `/api/debug/*`，用于观察 agent-tool 治理指标、会话 trace
              和 agent tree。
            </p>
          </div>
          <div className="rounded-2xl border border-border bg-surface-soft px-4 py-3 text-right text-xs text-text-secondary">
            <div>{overviewLoading ? '正在同步全局指标…' : '全局指标已同步'}</div>
            <div className="mt-1">
              {overview ? `最新样本 ${formatTimestamp(overview.collectedAt)}` : '尚未读取样本'}
            </div>
          </div>
        </div>
      </header>

      <main className="grid min-h-0 flex-1 grid-cols-[320px_minmax(0,1fr)] gap-4 overflow-hidden px-4 py-4">
        <aside className="flex min-h-0 flex-col gap-4">
          <Panel title="Sessions" subtitle="选择当前要下钻的 session">
            {sessionListError ? <ErrorBanner message={sessionListError} /> : null}
            <div className="flex max-h-[280px] flex-col gap-2 overflow-y-auto">
              {sessions.map((session) => {
                const active = session.sessionId === selectedSessionId;
                return (
                  <button
                    key={session.sessionId}
                    type="button"
                    className={cn(
                      'rounded-2xl border px-3 py-3 text-left transition-colors',
                      active
                        ? 'border-accent bg-accent-soft/20'
                        : 'border-border bg-white/75 hover:bg-surface'
                    )}
                    onClick={() => setSelectedSessionId(session.sessionId)}
                  >
                    <div className="truncate text-sm font-medium text-text-primary">
                      {session.title}
                    </div>
                    <div className="mt-1 text-[11px] text-text-secondary">
                      {session.sessionId.slice(0, 12)} · {session.phase}
                    </div>
                    <div className="mt-1 truncate text-[11px] text-text-muted">
                      {session.workingDir}
                    </div>
                  </button>
                );
              })}
            </div>
          </Panel>

          <Panel
            title="Agent Tree"
            subtitle={selectedSessionMeta ? `当前会话：${selectedSessionMeta.title}` : '未选择会话'}
          >
            {sessionError ? <ErrorBanner message={sessionError} /> : null}
            <div className="flex min-h-0 flex-col gap-2 overflow-y-auto">
              {agentNodes.length === 0 ? (
                <EmptyState label="当前会话还没有 agent tree 样本。" />
              ) : (
                agentNodes.map((node) => <AgentTreeNode key={node.nodeId} node={node} />)
              )}
            </div>
          </Panel>
        </aside>

        <section className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] gap-4 overflow-hidden">
          <Panel title="Runtime Overview" subtitle="全局治理值与最近 5 分钟趋势">
            {overviewError ? <ErrorBanner message={overviewError} /> : null}
            <div className="grid grid-cols-1 gap-3 xl:grid-cols-3">
              <MetricCard
                label="spawn rejection"
                value={formatRatioBps(overview?.spawnRejectionRatioBps)}
                tone={ratioTone(overview?.spawnRejectionRatioBps)}
                detail={`${collaboration?.spawnRejected ?? 0} rejected / ${collaboration?.spawnAccepted ?? 0} accepted`}
              />
              <MetricCard
                label="observe to action"
                value={formatRatioBps(collaboration?.observeToActionRatioBps)}
                tone={ratioTone(collaboration?.observeToActionRatioBps)}
                detail={`${collaboration?.observeFollowedByAction ?? 0} follow-ups / ${collaboration?.observeCalls ?? 0} observes`}
              />
              <MetricCard
                label="child reuse"
                value={formatRatioBps(collaboration?.childReuseRatioBps)}
                tone={ratioTone(collaboration?.childReuseRatioBps)}
                detail={`${(collaboration?.sendReused ?? 0) + (collaboration?.sendQueued ?? 0)} reuse signals / ${collaboration?.spawnAccepted ?? 0} spawns`}
              />
            </div>

            <div className="mt-4 rounded-[20px] border border-border bg-white/80 px-4 py-4">
              <div className="flex items-center justify-between gap-3">
                <div className="text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
                  Recent 5m Trend
                </div>
                <div className="text-[11px] text-text-secondary">
                  {timeline
                    ? `${formatTimestamp(timeline.windowStartedAt)} - ${formatTimestamp(timeline.windowEndedAt)}`
                    : '等待服务端样本'}
                </div>
              </div>
              <div className="mt-3 overflow-hidden rounded-[16px] border border-border bg-surface">
                <svg
                  viewBox={`0 0 ${TREND_CHART_WIDTH} ${TREND_CHART_HEIGHT}`}
                  className="block h-[168px] w-full"
                  preserveAspectRatio="none"
                >
                  <line x1="0" y1="8" x2={TREND_CHART_WIDTH} y2="8" stroke="rgba(79,87,102,0.12)" />
                  <line
                    x1="0"
                    y1={TREND_CHART_HEIGHT / 2}
                    x2={TREND_CHART_WIDTH}
                    y2={TREND_CHART_HEIGHT / 2}
                    stroke="rgba(79,87,102,0.12)"
                  />
                  <line
                    x1="0"
                    y1={TREND_CHART_HEIGHT - 8}
                    x2={TREND_CHART_WIDTH}
                    y2={TREND_CHART_HEIGHT - 8}
                    stroke="rgba(79,87,102,0.12)"
                  />
                  <TrendLine points={spawnTrendPoints} color="#d96c2f" />
                  <TrendLine points={observeTrendPoints} color="#4472d9" />
                  <TrendLine points={reuseTrendPoints} color="#2f9a68" />
                </svg>
              </div>
              <div className="mt-3 flex flex-wrap gap-2 text-[11px] text-text-secondary">
                <LegendPill
                  label="spawn rejection"
                  color="#d96c2f"
                  value={formatRatioBps(overview?.spawnRejectionRatioBps)}
                />
                <LegendPill
                  label="observe to action"
                  color="#4472d9"
                  value={formatRatioBps(collaboration?.observeToActionRatioBps)}
                />
                <LegendPill
                  label="child reuse"
                  color="#2f9a68"
                  value={formatRatioBps(collaboration?.childReuseRatioBps)}
                />
              </div>
            </div>
          </Panel>

          <Panel
            title="Session Trace"
            subtitle={
              selectedSessionMeta
                ? `${selectedSessionMeta.title} · ${selectedSessionMeta.sessionId.slice(0, 12)}`
                : '请在左侧选择会话'
            }
          >
            {sessionLoading && trace == null ? (
              <div className="mb-3 text-xs text-text-secondary">正在同步当前会话 trace…</div>
            ) : null}
            {trace?.parentSessionId ? (
              <div className="mb-3 text-xs text-text-secondary">
                parent session: {trace.parentSessionId.slice(0, 12)}
              </div>
            ) : null}
            <div className="flex min-h-0 flex-col gap-2 overflow-y-auto">
              {trace?.items.length ? (
                trace.items.map((item) => <TraceItemCard key={item.id} item={item} />)
              ) : (
                <EmptyState label="当前会话还没有 trace 样本。" />
              )}
            </div>
          </Panel>
        </section>
      </main>
    </div>
  );
}

function Panel({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
}) {
  return (
    <section className="flex min-h-0 flex-col rounded-[24px] border border-border bg-surface/94 p-4 shadow-soft">
      <div className="mb-4">
        <div className="text-xs font-semibold uppercase tracking-[0.1em] text-text-muted">
          {title}
        </div>
        <div className="mt-1 text-sm text-text-secondary">{subtitle}</div>
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </section>
  );
}

function MetricCard({
  label,
  value,
  detail,
  tone,
}: {
  label: string;
  value: string;
  detail: string;
  tone: string;
}) {
  return (
    <div className="rounded-[20px] border border-border bg-white/80 px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          {label}
        </div>
        <span className={cn('rounded-full px-2.5 py-1 text-xs font-semibold', tone)}>{value}</span>
      </div>
      <div className="mt-2 text-xs text-text-secondary">{detail}</div>
    </div>
  );
}

function LegendPill({ label, color, value }: { label: string; color: string; value: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-border bg-surface px-2.5 py-1">
      <span
        className="inline-block h-2 w-2 rounded-full"
        style={{ backgroundColor: color }}
        aria-hidden="true"
      />
      <span>{label}</span>
      <span className="font-semibold text-text-primary">{value}</span>
    </span>
  );
}

function TrendLine({ points, color }: { points: Array<{ x: number; y: number }>; color: string }) {
  if (points.length === 0) {
    return null;
  }
  if (points.length === 1) {
    const point = points[0];
    return <circle cx={point.x} cy={point.y} r="2.5" fill={color} />;
  }
  return (
    <polyline
      fill="none"
      stroke={color}
      strokeWidth="2"
      strokeLinejoin="round"
      strokeLinecap="round"
      points={points.map((point) => `${point.x},${point.y}`).join(' ')}
    />
  );
}

function AgentTreeNode({ node }: { node: SessionDebugAgentNode }) {
  return (
    <div
      className="rounded-[18px] border border-border bg-white/75 px-3 py-3"
      style={{ marginLeft: `${node.depth * 12}px` }}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium text-text-primary">{node.title}</div>
          <div className="mt-1 text-[11px] text-text-secondary">
            {node.agentId}
            {node.subRunId ? ` · ${node.subRunId}` : ''}
          </div>
        </div>
        <span
          className={cn(
            'rounded-full px-2.5 py-1 text-[11px] font-semibold',
            lifecycleTone(node.lifecycle)
          )}
        >
          {node.lifecycle}
        </span>
      </div>
      <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-text-muted">
        <span>{node.kind === 'sessionRoot' ? 'session root' : 'child agent'}</span>
        {node.lineageKind ? <span>{node.lineageKind}</span> : null}
        {node.statusSource ? <span>{node.statusSource}</span> : null}
        {node.lastTurnOutcome ? <span>{node.lastTurnOutcome}</span> : null}
      </div>
    </div>
  );
}

function TraceItemCard({ item }: { item: SessionDebugTrace['items'][number] }) {
  return (
    <div className="rounded-[20px] border border-border bg-white/80 px-4 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium text-text-primary">{item.title}</div>
          <div className="mt-1 text-[11px] text-text-secondary">
            {formatTimestamp(item.recordedAt)}
            {item.storageSeq ? ` · seq ${item.storageSeq}` : ''}
            {item.turnId ? ` · turn ${item.turnId}` : ''}
          </div>
        </div>
        <span className="rounded-full bg-surface px-2.5 py-1 text-[11px] font-semibold text-text-secondary">
          {item.kind}
        </span>
      </div>
      <div className="mt-2 text-sm leading-6 text-text-secondary">{item.summary}</div>
      <div className="mt-3 flex flex-wrap gap-2 text-[11px] text-text-muted">
        {item.agentId ? <span>agent {item.agentId}</span> : null}
        {item.subRunId ? <span>subRun {item.subRunId}</span> : null}
        {item.childAgentId ? <span>child {item.childAgentId}</span> : null}
        {item.deliveryId ? <span>delivery {item.deliveryId}</span> : null}
        {item.toolName ? <span>tool {item.toolName}</span> : null}
        {item.lifecycle ? <span>{item.lifecycle}</span> : null}
        {item.lastTurnOutcome ? <span>{item.lastTurnOutcome}</span> : null}
      </div>
    </div>
  );
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <div className="mb-3 rounded-2xl border border-danger/20 bg-danger-soft px-3 py-2 text-xs text-danger">
      {message}
    </div>
  );
}

function EmptyState({ label }: { label: string }) {
  return (
    <div className="rounded-[18px] border border-dashed border-border px-4 py-6 text-sm text-text-secondary">
      {label}
    </div>
  );
}
