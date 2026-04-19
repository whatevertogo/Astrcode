import type {
  AgentLifecycle,
  ChildSessionNotificationKind,
  ChildSessionNotificationMessage,
  CompactMeta,
  ConversationControlState,
  ConversationPlanReference,
  ConversationTaskItem,
  ConversationTaskStatus,
  LastCompactMeta,
  Message,
  ParentDelivery,
  Phase,
  SubRunThreadTree,
  SubRunViewData,
  ToolStatus,
} from '../../types';
import {
  buildSubRunThreadTree,
  listRootSubRunViews,
  patchSubRunThreadTreeMessages,
} from '../subRunView';
import { request } from './client';
import type { SessionEventFilterQuery } from '../sessionView';
import { asRecord, pickOptionalString, pickStringOrUndefined as pickString } from '../shared';

type ConversationRecord = Record<string, unknown>;

export interface ConversationSnapshotState {
  cursor: string | null;
  phase: Phase;
  control: ConversationControlState;
  blocks: ConversationRecord[];
  childSummaries: ConversationRecord[];
}

export interface ConversationViewProjection {
  cursor: string | null;
  phase: Phase;
  control: ConversationControlState;
  messages: Message[];
  messageTree: SubRunThreadTree;
  messageFingerprint: string;
  childSubRuns: SubRunViewData[];
  childFingerprint: string;
}

function parsePhase(value: unknown): Phase {
  switch (value) {
    case 'idle':
    case 'thinking':
    case 'callingTool':
    case 'streaming':
    case 'interrupted':
    case 'done':
      return value;
    default:
      throw new Error(`invalid conversation phase: ${String(value)}`);
  }
}

function parseAgentLifecycle(value: unknown): AgentLifecycle {
  switch (value) {
    case 'pending':
    case 'running':
    case 'idle':
    case 'terminated':
      return value;
    default:
      return 'running';
  }
}

function parseToolStatus(value: unknown): ToolStatus {
  switch (value) {
    case 'complete':
    case 'completed':
      return 'ok';
    case 'failed':
    case 'cancelled':
      return 'fail';
    default:
      return 'running';
  }
}

function parseToolChildRef(value: unknown) {
  const record = asRecord(value);
  const agentId = pickString(record ?? {}, 'agentId');
  const sessionId = pickString(record ?? {}, 'sessionId');
  const subRunId = pickString(record ?? {}, 'subRunId');
  const openSessionId = pickString(record ?? {}, 'openSessionId');
  if (!agentId || !sessionId || !subRunId || !openSessionId) {
    return undefined;
  }

  return {
    agentId,
    sessionId,
    subRunId,
    executionId: pickOptionalString(record ?? {}, 'executionId') ?? undefined,
    parentAgentId: pickOptionalString(record ?? {}, 'parentAgentId') ?? undefined,
    parentSubRunId: pickOptionalString(record ?? {}, 'parentSubRunId') ?? undefined,
    lineageKind:
      pickString(record ?? {}, 'lineageKind') === 'fork'
        ? 'fork'
        : pickString(record ?? {}, 'lineageKind') === 'resume'
          ? 'resume'
          : 'spawn',
    status: parseAgentLifecycle(record?.status),
    openSessionId,
  } as const;
}

function buildConversationQueryString(options?: {
  cursor?: string | null;
  filter?: SessionEventFilterQuery;
}): string {
  const params = new URLSearchParams();
  params.set('focus', options?.filter?.subRunId ? `subrun:${options.filter.subRunId}` : 'root');
  if (options?.cursor) {
    params.set('cursor', options.cursor);
  }
  const query = params.toString();
  return query ? `?${query}` : '';
}

function createProgressDelivery(
  idempotencyKey: string,
  message: string,
  terminalSemantics: 'terminal' | 'non_terminal'
): ParentDelivery {
  return {
    idempotencyKey,
    origin: 'explicit',
    terminalSemantics,
    kind: 'progress',
    payload: { message },
  };
}

function childSummaryNotificationKind(lifecycle: AgentLifecycle): ChildSessionNotificationKind {
  return lifecycle === 'idle' || lifecycle === 'terminated' ? 'delivered' : 'progress_summary';
}

function parseCompactTrigger(
  value: unknown,
  fallback: LastCompactMeta['trigger'] = 'manual'
): LastCompactMeta['trigger'] {
  switch (value) {
    case 'auto':
    case 'manual':
    case 'deferred':
      return value;
    default:
      return fallback;
  }
}

function parseCompactMode(value: unknown): CompactMeta['mode'] {
  switch (value) {
    case 'full':
    case 'incremental':
    case 'retry_salvage':
      return value;
    default:
      return 'full';
  }
}

function parseCompactMeta(value: unknown): CompactMeta | undefined {
  const record = asRecord(value);
  if (!record) {
    return undefined;
  }
  return {
    mode: parseCompactMode(record.mode),
    instructionsPresent: record.instructionsPresent === true,
    fallbackUsed: record.fallbackUsed === true,
    retryCount: typeof record.retryCount === 'number' ? record.retryCount : 0,
    inputUnits: typeof record.inputUnits === 'number' ? record.inputUnits : 0,
    outputSummaryChars:
      typeof record.outputSummaryChars === 'number' ? record.outputSummaryChars : 0,
  };
}

function parseLastCompactMeta(value: unknown): LastCompactMeta | undefined {
  const record = asRecord(value);
  const meta = parseCompactMeta(record?.meta ?? record);
  if (!record || !meta) {
    return undefined;
  }
  return {
    trigger: parseCompactTrigger(record.trigger),
    meta,
  };
}

function parseTaskStatus(value: unknown): ConversationTaskStatus | undefined {
  switch (value) {
    case 'pending':
    case 'in_progress':
    case 'completed':
      return value;
    default:
      return undefined;
  }
}

function parsePlanReference(value: unknown): ConversationPlanReference | undefined {
  const plan = asRecord(value);
  const slug = pickString(plan ?? {}, 'slug');
  const path = pickString(plan ?? {}, 'path');
  const status = pickString(plan ?? {}, 'status');
  const title = pickString(plan ?? {}, 'title');
  if (!slug || !path || !status || !title) {
    return undefined;
  }
  return { slug, path, status, title };
}

function parseActiveTasks(value: unknown): ConversationTaskItem[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const items = value
    .map((entry): ConversationTaskItem | null => {
      const task = asRecord(entry);
      const content = pickString(task ?? {}, 'content');
      const status = parseTaskStatus(task?.status);
      if (!content || !status) {
        return null;
      }
      const activeForm = pickOptionalString(task ?? {}, 'activeForm') ?? undefined;
      return {
        content,
        status,
        ...(activeForm ? { activeForm } : {}),
      };
    })
    .filter((item): item is ConversationTaskItem => item !== null);
  return items.length > 0 ? items : undefined;
}

function parseConversationControlState(record: ConversationRecord): ConversationControlState {
  const controlRecord = asRecord(record.control);
  const phase = parsePhase(controlRecord?.phase ?? record.phase);
  const lastCompactMeta = parseLastCompactMeta(controlRecord?.lastCompactMeta);
  return {
    phase,
    canSubmitPrompt: controlRecord?.canSubmitPrompt !== false,
    canRequestCompact: controlRecord?.canRequestCompact !== false,
    compactPending: controlRecord?.compactPending === true,
    compacting: controlRecord?.compacting === true,
    currentModeId: pickString(controlRecord ?? {}, 'currentModeId') ?? 'code',
    activeTurnId: pickOptionalString(controlRecord ?? {}, 'activeTurnId') ?? undefined,
    lastCompactMeta,
    activePlan: parsePlanReference(controlRecord?.activePlan),
    activeTasks: parseActiveTasks(controlRecord?.activeTasks),
  };
}

function childSummaryToMessage(
  summary: ConversationRecord,
  options?: {
    idPrefix?: string;
    notificationKind?: ChildSessionNotificationKind;
    deliveryMessage?: string;
    terminalSemantics?: 'terminal' | 'non_terminal';
  }
): ChildSessionNotificationMessage | null {
  const childSessionId = pickString(summary, 'childSessionId');
  const childAgentId = pickString(summary, 'childAgentId');
  const title = pickString(summary, 'title') ?? childAgentId ?? childSessionId;
  const childRefRecord = asRecord(summary.childRef);
  const subRunId = childRefRecord ? pickString(childRefRecord, 'subRunId') : undefined;
  if (!childSessionId || !childAgentId || !subRunId) {
    return null;
  }

  const lifecycle = parseAgentLifecycle(summary.lifecycle);
  const latestOutputSummary =
    options?.deliveryMessage ?? pickString(summary, 'latestOutputSummary') ?? undefined;
  const delivery =
    latestOutputSummary && latestOutputSummary.length > 0
      ? createProgressDelivery(
          `${options?.idPrefix ?? 'conversation-child-summary'}:${childSessionId}`,
          latestOutputSummary,
          options?.terminalSemantics ??
            (lifecycle === 'idle' || lifecycle === 'terminated' ? 'terminal' : 'non_terminal')
        )
      : undefined;

  return {
    id: `${options?.idPrefix ?? 'conversation-child-summary'}:${childSessionId}`,
    kind: 'childSessionNotification',
    turnId: null,
    agentId: childAgentId,
    agentProfile: title,
    subRunId,
    childSessionId,
    childRef: {
      agentId: childAgentId,
      sessionId:
        (childRefRecord ? pickString(childRefRecord, 'sessionId') : undefined) ?? childSessionId,
      subRunId,
      executionId:
        (childRefRecord ? pickOptionalString(childRefRecord, 'executionId') : undefined) ??
        undefined,
      parentAgentId:
        (childRefRecord ? pickOptionalString(childRefRecord, 'parentAgentId') : undefined) ??
        undefined,
      parentSubRunId:
        (childRefRecord ? pickOptionalString(childRefRecord, 'parentSubRunId') : undefined) ??
        undefined,
      lineageKind:
        childRefRecord && pickString(childRefRecord, 'lineageKind') === 'fork'
          ? 'fork'
          : childRefRecord && pickString(childRefRecord, 'lineageKind') === 'resume'
            ? 'resume'
            : 'spawn',
      status: lifecycle,
      openSessionId:
        (childRefRecord ? pickString(childRefRecord, 'openSessionId') : undefined) ??
        childSessionId,
    },
    notificationKind: options?.notificationKind ?? childSummaryNotificationKind(lifecycle),
    status: lifecycle,
    delivery,
    timestamp: 0,
  };
}

function normalizeSnapshotState(payload: unknown): ConversationSnapshotState {
  const record = asRecord(payload);
  if (!record) {
    throw new Error('invalid conversation snapshot response');
  }
  const control = parseConversationControlState(record);
  return {
    cursor: pickOptionalString(record, 'cursor') ?? null,
    phase: control.phase,
    control,
    blocks: Array.isArray(record.blocks)
      ? (record.blocks.filter(asRecord) as ConversationRecord[])
      : [],
    childSummaries: Array.isArray(record.childSummaries)
      ? (record.childSummaries.filter(asRecord) as ConversationRecord[])
      : [],
  };
}

function projectConversationMessages(
  state: ConversationSnapshotState,
  options?: { includeInlineChildSummaries?: boolean }
): Message[] {
  const messages: Message[] = [];
  const reasoningByTurn = new Map<string, string>();
  const thinkingTurnIds = new Set<string>();
  const inlineChildSessions = new Set<string>();

  state.blocks.forEach((block, index) => {
    const kind = pickString(block, 'kind');
    const id = pickString(block, 'id') ?? `conversation-block-${index}`;
    const turnId = pickOptionalString(block, 'turnId') ?? null;
    if (!kind) {
      return;
    }

    switch (kind) {
      case 'user':
        messages.push({
          id: `conversation-user:${id}`,
          kind: 'user',
          turnId,
          text: pickString(block, 'markdown') ?? '',
          timestamp: index,
        });
        return;

      case 'thinking': {
        const markdown = pickString(block, 'markdown') ?? '';
        if (turnId) {
          reasoningByTurn.set(turnId, markdown);
          thinkingTurnIds.add(turnId);
        }
        // TODO(conversation-stream): 如果后端后续把 thinking 升级为一级流事件契约，
        // 这里应直接映射成专用 message kind，而不是继续借 assistant message 承载。
        messages.push({
          id: `conversation-thinking:${id}`,
          kind: 'assistant',
          turnId,
          text: '',
          reasoningText: markdown,
          streaming: pickString(block, 'status') === 'streaming',
          timestamp: index,
        });
        return;
      }

      case 'assistant':
        messages.push({
          id: `conversation-assistant:${id}`,
          kind: 'assistant',
          turnId,
          text: pickString(block, 'markdown') ?? '',
          reasoningText:
            turnId && !thinkingTurnIds.has(turnId) ? reasoningByTurn.get(turnId) : undefined,
          streaming: pickString(block, 'status') === 'streaming',
          timestamp: index,
        });
        return;

      case 'plan': {
        const blockers = asRecord(block.blockers);
        const review = asRecord(block.review);
        messages.push({
          id: `conversation-plan:${id}`,
          kind: 'plan',
          turnId,
          toolCallId: pickString(block, 'toolCallId') ?? id,
          eventKind:
            pickString(block, 'eventKind') === 'review_pending'
              ? 'review_pending'
              : pickString(block, 'eventKind') === 'presented'
                ? 'presented'
                : 'saved',
          title: pickString(block, 'title') ?? 'Session Plan',
          planPath: pickString(block, 'planPath') ?? '',
          summary: pickOptionalString(block, 'summary') ?? undefined,
          status: pickOptionalString(block, 'status') ?? undefined,
          slug: pickOptionalString(block, 'slug') ?? undefined,
          updatedAt: pickOptionalString(block, 'updatedAt') ?? undefined,
          content: pickOptionalString(block, 'content') ?? undefined,
          review:
            review &&
            (pickString(review, 'kind') === 'revise_plan' ||
              pickString(review, 'kind') === 'final_review')
              ? {
                  kind: pickString(review, 'kind') as 'revise_plan' | 'final_review',
                  checklist: Array.isArray(review.checklist)
                    ? review.checklist.filter((value): value is string => typeof value === 'string')
                    : [],
                }
              : undefined,
          blockers: {
            missingHeadings: Array.isArray(blockers?.missingHeadings)
              ? blockers.missingHeadings.filter(
                  (value): value is string => typeof value === 'string'
                )
              : [],
            invalidSections: Array.isArray(blockers?.invalidSections)
              ? blockers.invalidSections.filter(
                  (value): value is string => typeof value === 'string'
                )
              : [],
          },
          timestamp: index,
        });
        return;
      }

      case 'tool_call': {
        const toolCallId = pickOptionalString(block, 'toolCallId') ?? id;
        const streams = asRecord(block.streams);
        const childRef = parseToolChildRef(block.childRef);
        messages.push({
          id: `conversation-tool:${toolCallId}`,
          kind: 'toolCall',
          turnId,
          toolCallId,
          toolName: pickString(block, 'toolName') ?? 'tool',
          status: parseToolStatus(block.status),
          args: block.input ?? null,
          output: pickOptionalString(block, 'summary') || undefined,
          error: pickOptionalString(block, 'error') || undefined,
          metadata: block.metadata ?? undefined,
          childRef,
          childSessionId: childRef?.openSessionId,
          streams: {
            stdout: pickString(streams ?? {}, 'stdout') ?? '',
            stderr: pickString(streams ?? {}, 'stderr') ?? '',
          },
          durationMs: (() => {
            const value = block.durationMs;
            return typeof value === 'number' ? value : undefined;
          })(),
          truncated: block.truncated === true,
          timestamp: index,
        });
        return;
      }

      case 'system_note': {
        if (pickString(block, 'noteKind') !== 'compact') {
          return;
        }
        const compactMetaRecord = asRecord(block.compactMeta);
        const trigger = parseCompactTrigger(
          compactMetaRecord?.trigger ??
            pickString(block, 'compactTrigger') ??
            pickString(block, 'trigger'),
          state.control.lastCompactMeta?.trigger ?? 'manual'
        );
        messages.push({
          id: `conversation-compact:${id}`,
          kind: 'compact',
          turnId,
          trigger,
          meta: parseCompactMeta(block.compactMeta) ?? {
            mode: 'full',
            instructionsPresent: false,
            fallbackUsed: false,
            retryCount: 0,
            inputUnits: 0,
            outputSummaryChars: (pickString(block, 'markdown') ?? '').length,
          },
          summary: pickString(block, 'markdown') ?? '',
          preservedRecentTurns: 0,
          timestamp: index,
        });
        return;
      }

      case 'child_handoff': {
        const child = asRecord(block.child);
        if (!child) {
          return;
        }
        const handoffKind = pickString(block, 'handoffKind');
        const childSessionId = pickString(child, 'childSessionId');
        const message = childSummaryToMessage(child, {
          idPrefix: `conversation-child-handoff:${id}`,
          notificationKind:
            handoffKind === 'returned'
              ? 'delivered'
              : handoffKind === 'progress'
                ? 'progress_summary'
                : 'started',
          deliveryMessage: pickString(block, 'message') ?? undefined,
          terminalSemantics: handoffKind === 'returned' ? 'terminal' : 'non_terminal',
        });
        if (!message) {
          return;
        }
        message.timestamp = index;
        messages.push(message);
        if (childSessionId) {
          inlineChildSessions.add(childSessionId);
        }
        return;
      }

      case 'error':
        messages.push({
          id: `conversation-error:${id}`,
          kind: 'assistant',
          turnId,
          text: `错误：${pickString(block, 'message') ?? 'conversation error'}`,
          reasoningText: '',
          streaming: false,
          timestamp: index,
        });
        return;

      default:
        return;
    }
  });

  if (options?.includeInlineChildSummaries !== false) {
    state.childSummaries.forEach((summary, index) => {
      const childSessionId = pickString(summary, 'childSessionId');
      if (childSessionId && inlineChildSessions.has(childSessionId)) {
        return;
      }
      const message = childSummaryToMessage(summary);
      if (!message) {
        return;
      }
      message.timestamp = state.blocks.length + index;
      messages.push(message);
    });
  }

  return messages;
}

function projectChildSubRuns(state: ConversationSnapshotState): {
  childSubRuns: SubRunViewData[];
  childFingerprint: string;
} {
  const childSummaryMessages = state.childSummaries
    .map((summary) => childSummaryToMessage(summary))
    .filter((message): message is ChildSessionNotificationMessage => message !== null);
  const childTree = buildSubRunThreadTree(childSummaryMessages);
  return {
    childSubRuns: listRootSubRunViews(childTree),
    childFingerprint: childTree.rootStreamFingerprint,
  };
}

export function projectConversationState(
  state: ConversationSnapshotState,
  focusSubRunId?: string,
  previousMessageTree?: SubRunThreadTree
): ConversationViewProjection {
  const includeInlineChildSummaries = !focusSubRunId;
  const messages = projectConversationMessages(state, { includeInlineChildSummaries });
  const messageTree =
    previousMessageTree !== undefined
      ? // TODO(stream-perf): 当前结构变化时会回退全量重建；如果后续长会话仍有卡顿，
        // 需要把 envelope -> tree patch 做成真正按 block 粒度的增量更新，而不是按 message 回退。
        (patchSubRunThreadTreeMessages(previousMessageTree, messages) ??
        buildSubRunThreadTree(messages))
      : buildSubRunThreadTree(messages);
  const { childSubRuns, childFingerprint } = includeInlineChildSummaries
    ? { childSubRuns: [] as SubRunViewData[], childFingerprint: '' }
    : projectChildSubRuns(state);

  return {
    cursor: state.cursor,
    phase: state.control.phase,
    control: state.control,
    messages,
    messageTree,
    messageFingerprint: messageTree.rootStreamFingerprint,
    childSubRuns,
    childFingerprint,
  };
}

function upsertChildSummary(childSummaries: ConversationRecord[], next: ConversationRecord): void {
  const childSessionId = pickString(next, 'childSessionId');
  if (!childSessionId) {
    return;
  }
  const index = childSummaries.findIndex(
    (candidate) => pickString(candidate, 'childSessionId') === childSessionId
  );
  if (index >= 0) {
    childSummaries[index] = next;
  } else {
    childSummaries.push(next);
  }
}

function applyBlockPatch(block: ConversationRecord, patch: ConversationRecord): void {
  const kind = pickString(patch, 'kind');
  switch (kind) {
    case 'append_markdown': {
      const markdown = pickString(patch, 'markdown') ?? '';
      if (typeof block.markdown === 'string') {
        block.markdown += markdown;
      } else {
        block.markdown = markdown;
      }
      break;
    }
    case 'replace_markdown': {
      const markdown = pickString(patch, 'markdown') ?? '';
      if (typeof block.markdown === 'string' || 'markdown' in block) {
        block.markdown = markdown;
      } else {
        block.content = markdown;
      }
      break;
    }
    case 'append_tool_stream': {
      const chunk = pickString(patch, 'chunk') ?? '';
      const stream = pickString(patch, 'stream');
      const streams = asRecord(block.streams) ?? {};
      if (stream === 'stdout' || stream === 'stderr') {
        const current = typeof streams[stream] === 'string' ? streams[stream] : '';
        block.streams = {
          ...streams,
          [stream]: `${current}${chunk}`,
        };
      }
      break;
    }
    case 'replace_summary':
      block.summary = pickOptionalString(patch, 'summary') ?? null;
      break;
    case 'replace_metadata':
      block.metadata = patch.metadata;
      break;
    case 'replace_error':
      block.error = pickOptionalString(patch, 'error') ?? null;
      break;
    case 'replace_duration': {
      const durationMs = patch.durationMs;
      if (typeof durationMs === 'number') {
        block.durationMs = durationMs;
      }
      break;
    }
    case 'replace_child_ref': {
      const childRef = parseToolChildRef(patch.childRef);
      if (childRef) {
        block.childRef = childRef;
        block.childSessionId = childRef.openSessionId;
      }
      break;
    }
    case 'set_truncated':
      block.truncated = patch.truncated === true;
      break;
    case 'set_status':
      block.status = pickString(patch, 'status') ?? block.status;
      break;
  }
}

export async function loadConversationSnapshotState(
  sessionId: string,
  filter?: SessionEventFilterQuery
): Promise<ConversationSnapshotState> {
  const response = await request(
    `/api/v1/conversation/sessions/${encodeURIComponent(sessionId)}/snapshot${buildConversationQueryString(
      {
        filter,
      }
    )}`
  );
  return normalizeSnapshotState(await response.json());
}

export function createConversationStreamRequestPath(
  sessionId: string,
  cursor?: string | null,
  filter?: SessionEventFilterQuery
): string {
  return `/api/v1/conversation/sessions/${encodeURIComponent(sessionId)}/stream${buildConversationQueryString(
    {
      cursor,
      filter,
    }
  )}`;
}

export function applyConversationEnvelope(
  state: ConversationSnapshotState,
  payload: unknown
): void {
  const envelope = asRecord(payload);
  if (!envelope) {
    throw new Error('invalid conversation stream envelope');
  }

  const kind = pickString(envelope, 'kind');
  if (!kind) {
    return;
  }
  const envelopeCursor = pickOptionalString(envelope, 'cursor');
  if (envelopeCursor) {
    state.cursor = envelopeCursor;
  }

  switch (kind) {
    case 'append_block': {
      const block = asRecord(envelope.block);
      if (block) {
        const blockId = pickString(block, 'id');
        const existingIndex = blockId
          ? state.blocks.findIndex((candidate) => pickString(candidate, 'id') === blockId)
          : -1;
        if (existingIndex >= 0) {
          state.blocks[existingIndex] = {
            ...state.blocks[existingIndex],
            ...block,
          };
        } else {
          state.blocks.push(block);
        }
      }
      return;
    }

    case 'patch_block': {
      const blockId = pickString(envelope, 'blockId');
      const patch = asRecord(envelope.patch);
      const block = blockId
        ? state.blocks.find((candidate) => pickString(candidate, 'id') === blockId)
        : undefined;
      if (block && patch) {
        applyBlockPatch(block, patch);
      }
      return;
    }

    case 'complete_block': {
      const blockId = pickString(envelope, 'blockId');
      const block = blockId
        ? state.blocks.find((candidate) => pickString(candidate, 'id') === blockId)
        : undefined;
      if (block) {
        block.status = pickString(envelope, 'status') ?? block.status;
      }
      return;
    }

    case 'update_control_state': {
      const control = asRecord(envelope.control);
      if (control) {
        state.control = {
          phase: parsePhase(control.phase),
          canSubmitPrompt: control.canSubmitPrompt !== false,
          canRequestCompact: control.canRequestCompact !== false,
          compactPending: control.compactPending === true,
          compacting: control.compacting === true,
          currentModeId: pickString(control, 'currentModeId') ?? state.control.currentModeId,
          activeTurnId: pickOptionalString(control, 'activeTurnId') ?? undefined,
          lastCompactMeta: parseLastCompactMeta(control.lastCompactMeta),
          activePlan: parsePlanReference(control.activePlan),
          activeTasks: parseActiveTasks(control.activeTasks),
        };
        state.phase = state.control.phase;
      }
      return;
    }

    case 'upsert_child_summary': {
      const child = asRecord(envelope.child);
      if (child) {
        upsertChildSummary(state.childSummaries, child);
      }
      return;
    }

    case 'remove_child_summary': {
      const childSessionId = pickString(envelope, 'childSessionId');
      state.childSummaries = state.childSummaries.filter(
        (candidate) => pickString(candidate, 'childSessionId') !== childSessionId
      );
      return;
    }

    case 'replace_slash_candidates':
    case 'set_banner':
    case 'clear_banner':
    case 'rehydrate_required':
    default:
      return;
  }
}
