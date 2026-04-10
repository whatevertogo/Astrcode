import type { AgentStatus, Message, SubRunFinishMessage, SubRunStartMessage } from '../types';

interface IndexedMessage {
  index: number;
  message: Message;
}

interface SpawnedAgentRef {
  subRunId: string;
  agentId?: string;
  childSessionId?: string;
}

interface SubRunRecord {
  subRunId: string;
  startMessage?: SubRunStartMessage;
  finishMessage?: SubRunFinishMessage;
  ownBodyEntries: IndexedMessage[];
  agentId?: string;
  childSessionId?: string;
  title: string;
  parentAgentId?: string;
  hasDescriptorLineage: boolean;
  parentSubRunId: string | null;
  directChildSubRunIds: string[];
  firstMessageIndex: number;
  startIndex: number;
}

interface SubRunIndex {
  records: Map<string, SubRunRecord>;
  rootEntries: IndexedMessage[];
}

export interface ThreadMessageItem {
  kind: 'message';
  message: Message;
}

export interface ThreadSubRunItem {
  kind: 'subRun';
  subRunId: string;
}

export type ThreadItem = ThreadMessageItem | ThreadSubRunItem;

export interface SubRunViewData {
  subRunId: string;
  title: string;
  startMessage?: SubRunStartMessage;
  finishMessage?: SubRunFinishMessage;
  bodyMessages: Message[];
  threadItems: ThreadItem[];
  streamFingerprint: string;
  childSessionId?: string;
  parentSubRunId: string | null;
  directChildSubRunIds: string[];
  hasDescriptorLineage: boolean;
}

export interface SubRunThreadTree {
  rootThreadItems: ThreadItem[];
  rootStreamFingerprint: string;
  subRuns: Map<string, SubRunViewData>;
}

export interface SubRunPathView {
  validPath: string[];
  views: SubRunViewData[];
  activeView: SubRunViewData | null;
}

/// 父视图摘要卡片——从 childSessionNotification 消息中提取的只读投影。
/// 不包含 child 原始事件流或 raw JSON。
function isSubRunStartMessage(message: Message): message is SubRunStartMessage {
  return message.kind === 'subRunStart';
}

function isSubRunFinishMessage(message: Message): message is SubRunFinishMessage {
  return message.kind === 'subRunFinish';
}

function deriveSubRunTitle(
  subRunId: string,
  startMessage: SubRunStartMessage | undefined,
  finishMessage: SubRunFinishMessage | undefined,
  bodyMessages: Message[]
): string {
  return (
    startMessage?.agentProfile ??
    finishMessage?.agentProfile ??
    bodyMessages.find((message) => message.agentProfile)?.agentProfile ??
    bodyMessages.find((message) => message.agentId)?.agentId ??
    subRunId
  );
}

function buildMessageFingerprint(message: Message): string {
  if (message.kind === 'assistant') {
    return `${message.id}:assistant:${message.text.length}:${message.reasoningText?.length ?? 0}:${
      message.streaming ? 1 : 0
    }`;
  }
  if (message.kind === 'toolCall') {
    return `${message.id}:tool:${message.status}:${message.output?.length ?? 0}:${
      message.error?.length ?? 0
    }`;
  }
  if (message.kind === 'promptMetrics') {
    return `${message.id}:promptMetrics:${message.stepIndex}:${message.estimatedTokens}:${
      message.cacheReadInputTokens ?? 0
    }:${message.cacheCreationInputTokens ?? 0}`;
  }
  if (message.kind === 'compact') {
    return `${message.id}:compact:${message.summary.length}`;
  }
  if (message.kind === 'user') {
    return `${message.id}:user:${message.text.length}`;
  }
  if (message.kind === 'subRunStart') {
    return `${message.id}:subRunStart:${message.subRunId ?? 'unknown'}`;
  }
  if (message.kind === 'childSessionNotification') {
    return `${message.id}:childNotification:${message.childRef.subRunId}:${message.notificationKind}:${message.status}`;
  }
  return `${message.id}:subRunFinish:${message.subRunId ?? 'unknown'}:${message.result.status}`;
}

function getOrCreateRecord(
  records: Map<string, SubRunRecord>,
  subRunId: string,
  firstMessageIndex: number
): SubRunRecord {
  const existing = records.get(subRunId);
  if (existing) {
    return existing;
  }

  const created: SubRunRecord = {
    subRunId,
    ownBodyEntries: [],
    title: subRunId,
    hasDescriptorLineage: false,
    parentSubRunId: null,
    directChildSubRunIds: [],
    firstMessageIndex,
    startIndex: firstMessageIndex,
  };
  records.set(subRunId, created);
  return created;
}

function pickStringField(value: unknown, ...keys: string[]): string | undefined {
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }

  for (const key of keys) {
    const candidate = (value as Record<string, unknown>)[key];
    if (typeof candidate === 'string' && candidate.length > 0) {
      return candidate;
    }
  }

  return undefined;
}

function pickSpawnedAgentRef(message: Message): SpawnedAgentRef | null {
  if (message.kind !== 'toolCall' || message.toolName !== 'spawnAgent' || message.status !== 'ok') {
    return null;
  }

  const metadata =
    typeof message.metadata === 'object' && message.metadata !== null ? message.metadata : null;
  const agentRef = metadata
    ? ((metadata as Record<string, unknown>).agentRef ??
      (metadata as Record<string, unknown>).agent_ref)
    : null;
  const subRunId = pickStringField(agentRef, 'subRunId', 'sub_run_id');
  if (!subRunId) {
    return null;
  }

  return {
    subRunId,
    agentId: pickStringField(agentRef, 'agentId', 'agent_id'),
    childSessionId:
      pickStringField(agentRef, 'openSessionId', 'open_session_id') ??
      pickStringField(metadata, 'openSessionId', 'open_session_id'),
  };
}

function pickOpenableChildSessionId(message: Message): string | undefined {
  if (message.kind === 'childSessionNotification') {
    return message.childRef.openSessionId || message.childSessionId;
  }

  return message.childSessionId;
}

function pickChildSessionIdFromEntries(entries: IndexedMessage[]): string | undefined {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const childSessionId = pickOpenableChildSessionId(entries[index].message);
    if (childSessionId) {
      return childSessionId;
    }
  }

  return undefined;
}

function buildSubRunIndex(messages: Message[]): SubRunIndex {
  const records = new Map<string, SubRunRecord>();
  const rootEntries: IndexedMessage[] = [];

  // 推导 subRun 嵌套关系需要两步：
  // 第一遍：从 body 消息（非 start/finish）收集 turnId → subRunId 映射
  const turnToSubRun = new Map<string, string>();
  for (const message of messages) {
    if (!message.subRunId) continue;
    if (message.kind === 'subRunStart' || message.kind === 'subRunFinish') continue;
    if (message.turnId) {
      turnToSubRun.set(message.turnId, message.subRunId);
    }
  }

  // 第二遍：处理消息，用映射 + 栈确定父关系
  const subRunStack: string[] = [];

  messages.forEach((message, index) => {
    if (!message.subRunId) {
      rootEntries.push({ index, message });
      return;
    }

    const record = getOrCreateRecord(records, message.subRunId, index);
    if (message.agentId && !record.agentId) {
      record.agentId = message.agentId;
    }

    if (isSubRunStartMessage(message)) {
      record.startMessage ??= message;
      record.startIndex = Math.min(record.startIndex, index);
      if (message.parentTurnId) {
        record.hasDescriptorLineage = true;
      }

      // 父关系推导：先从 turnToSubRun 精确匹配，再退化到栈
      const turnParent = turnToSubRun.get(message.turnId ?? '') ?? null;
      if (!record.parentSubRunId && turnParent && turnParent !== message.subRunId) {
        record.parentSubRunId = turnParent;
      }
      // 栈退化：turnToSubRun 未命中时，检查栈顶 start 的 turnId 判断兄弟还是父子
      if (!record.parentSubRunId && subRunStack.length > 0) {
        const stackTopId = subRunStack[subRunStack.length - 1];
        const stackTopRecord = records.get(stackTopId);
        if (stackTopId !== message.subRunId) {
          if (stackTopRecord?.startMessage?.turnId === message.turnId) {
            // 相同 turnId → 兄弟关系，继承栈顶的父
            record.parentSubRunId = stackTopRecord?.parentSubRunId ?? null;
          } else {
            // 不同 turnId → 栈顶是父
            record.parentSubRunId = stackTopId;
          }
        }
      }

      subRunStack.push(message.subRunId);
      return;
    }

    if (isSubRunFinishMessage(message)) {
      record.finishMessage ??= message;
      if (message.parentTurnId) {
        record.hasDescriptorLineage = true;
      }
      // 从栈中移除已完成的 subRun（可能在栈的任意深度）
      const stackIndex = subRunStack.lastIndexOf(message.subRunId);
      if (stackIndex >= 0) {
        subRunStack.splice(stackIndex, 1);
      }
      return;
    }

    // 从 childSessionNotification 提取 parentAgentId
    if (message.kind === 'childSessionNotification') {
      const parentAgent = message.childRef?.parentAgentId;
      if (parentAgent && !record.parentAgentId) {
        record.parentAgentId = parentAgent;
      }
    }

    record.ownBodyEntries.push({ index, message });
  });

  rootEntries.forEach(({ index, message }) => {
    const spawnedAgentRef = pickSpawnedAgentRef(message);
    if (!spawnedAgentRef) {
      return;
    }

    // Why: 历史回放偶发缺少 subRun lifecycle 时，spawnAgent 的 agentRef 仍然能稳定标识子执行；
    // 用它补建占位记录，避免父会话把已启动的子 Agent 直接“吃掉”。
    const record = getOrCreateRecord(records, spawnedAgentRef.subRunId, index);
    record.startIndex = Math.min(record.startIndex, index);
    if (!record.agentId && spawnedAgentRef.agentId) {
      record.agentId = spawnedAgentRef.agentId;
    }
    if (!record.childSessionId && spawnedAgentRef.childSessionId) {
      record.childSessionId = spawnedAgentRef.childSessionId;
    }
  });

  const orderedRecords = [...records.values()].sort(
    (left, right) => left.firstMessageIndex - right.firstMessageIndex
  );
  const agentOwnerMap = new Map<string, string>();

  orderedRecords.forEach((record) => {
    record.childSessionId =
      record.startMessage?.childSessionId ??
      record.finishMessage?.childSessionId ??
      pickChildSessionIdFromEntries(record.ownBodyEntries) ??
      record.childSessionId;
    record.title = deriveSubRunTitle(
      record.subRunId,
      record.startMessage,
      record.finishMessage,
      record.ownBodyEntries.map((entry) => entry.message)
    );
    if (record.title === record.subRunId && record.agentId) {
      record.title = record.agentId;
    }

    if (record.agentId && !agentOwnerMap.has(record.agentId)) {
      agentOwnerMap.set(record.agentId, record.subRunId);
    }
  });

  orderedRecords.forEach((record) => {
    // parentSubRunId 已在消息遍历时通过栈推导设置
    // 如果有 childSessionNotification 提供的 parentAgentId，优先用它
    if (record.parentAgentId && agentOwnerMap.has(record.parentAgentId)) {
      record.parentSubRunId = agentOwnerMap.get(record.parentAgentId) ?? record.parentSubRunId;
    }
  });

  orderedRecords.forEach((record) => {
    if (!record.parentSubRunId) {
      return;
    }
    const parentRecord = records.get(record.parentSubRunId);
    if (!parentRecord) {
      return;
    }
    parentRecord.directChildSubRunIds.push(record.subRunId);
  });

  orderedRecords.forEach((record) => {
    record.directChildSubRunIds.sort((leftSubRunId, rightSubRunId) => {
      const left = records.get(leftSubRunId);
      const right = records.get(rightSubRunId);
      return (
        (left?.startIndex ?? Number.MAX_SAFE_INTEGER) -
        (right?.startIndex ?? Number.MAX_SAFE_INTEGER)
      );
    });
  });

  return { records, rootEntries };
}

function buildThreadItems(
  ownEntries: IndexedMessage[],
  directChildSubRunIds: string[],
  index: SubRunIndex
): ThreadItem[] {
  const sortableEntries: Array<
    | { kind: 'message'; sortIndex: number; message: Message }
    | { kind: 'subRun'; sortIndex: number; subRunId: string }
  > = [
    ...ownEntries.map((entry) => ({
      kind: 'message' as const,
      sortIndex: entry.index,
      message: entry.message,
    })),
    ...directChildSubRunIds.map((subRunId) => ({
      kind: 'subRun' as const,
      sortIndex: index.records.get(subRunId)?.startIndex ?? Number.MAX_SAFE_INTEGER,
      subRunId,
    })),
  ];

  sortableEntries.sort((left, right) => left.sortIndex - right.sortIndex);

  return sortableEntries.map((entry) =>
    entry.kind === 'message'
      ? { kind: 'message', message: entry.message }
      : { kind: 'subRun', subRunId: entry.subRunId }
  );
}

/// 构建子线程详情浏览所需的混合线程树。
/// 默认父视图已经改为 child summary projection，但子线程详情页和测试夹具
/// 仍需要一棵可导航的局部线程树来浏览 sub-run 内部消息。
export function buildSubRunThreadTree(messages: Message[]): SubRunThreadTree {
  const index = buildSubRunIndex(messages);
  const subRuns = new Map<string, SubRunViewData>();

  const materializeSubRun = (subRunId: string): SubRunViewData | null => {
    const cached = subRuns.get(subRunId);
    if (cached) {
      return cached;
    }

    const record = index.records.get(subRunId);
    if (!record) {
      return null;
    }

    record.directChildSubRunIds.forEach((childSubRunId) => {
      materializeSubRun(childSubRunId);
    });

    const threadItems = buildThreadItems(record.ownBodyEntries, record.directChildSubRunIds, index);
    const streamFingerprint = threadItems
      .map((item) =>
        item.kind === 'message'
          ? buildMessageFingerprint(item.message)
          : `subRun:${item.subRunId}:${subRuns.get(item.subRunId)?.streamFingerprint ?? item.subRunId}`
      )
      .join('|');

    const view: SubRunViewData = {
      subRunId,
      title: record.title,
      startMessage: record.startMessage,
      finishMessage: record.finishMessage,
      bodyMessages: record.ownBodyEntries.map((entry) => entry.message),
      threadItems,
      streamFingerprint,
      childSessionId: record.childSessionId,
      parentSubRunId: record.parentSubRunId,
      directChildSubRunIds: record.directChildSubRunIds,
      hasDescriptorLineage: record.hasDescriptorLineage,
    };
    subRuns.set(subRunId, view);
    return view;
  };

  [...index.records.keys()].forEach((subRunId) => {
    materializeSubRun(subRunId);
  });

  const rootChildSubRunIds = [...subRuns.values()]
    .filter((view) => view.parentSubRunId === null)
    .sort(
      (left, right) =>
        (index.records.get(left.subRunId)?.startIndex ?? Number.MAX_SAFE_INTEGER) -
        (index.records.get(right.subRunId)?.startIndex ?? Number.MAX_SAFE_INTEGER)
    )
    .map((view) => view.subRunId);

  const rootThreadItems = buildThreadItems(index.rootEntries, rootChildSubRunIds, index);
  const rootStreamFingerprint = rootThreadItems
    .map((item) =>
      item.kind === 'message'
        ? buildMessageFingerprint(item.message)
        : `subRun:${item.subRunId}:${subRuns.get(item.subRunId)?.streamFingerprint ?? item.subRunId}`
    )
    .join('|');

  return {
    rootThreadItems,
    rootStreamFingerprint,
    subRuns,
  };
}

export function buildSubRunView(
  messagesOrTree: Message[] | SubRunThreadTree,
  subRunId: string
): SubRunViewData | null {
  const tree = Array.isArray(messagesOrTree)
    ? buildSubRunThreadTree(messagesOrTree)
    : messagesOrTree;
  return tree.subRuns.get(subRunId) ?? null;
}

export function buildSubRunPathView(
  messagesOrTree: Message[] | SubRunThreadTree,
  subRunPath: string[]
): SubRunPathView {
  const tree = Array.isArray(messagesOrTree)
    ? buildSubRunThreadTree(messagesOrTree)
    : messagesOrTree;
  const views: SubRunViewData[] = [];

  for (const subRunId of subRunPath) {
    const nextView = tree.subRuns.get(subRunId);
    if (!nextView) {
      break;
    }

    const parentView = views[views.length - 1];
    if (parentView && nextView.parentSubRunId !== parentView.subRunId) {
      break;
    }

    views.push(nextView);
  }

  return {
    validPath: views.map((view) => view.subRunId),
    views,
    activeView: views[views.length - 1] ?? null,
  };
}

export function listRootSubRunViews(
  messagesOrTree: Message[] | SubRunThreadTree
): SubRunViewData[] {
  const tree = Array.isArray(messagesOrTree)
    ? buildSubRunThreadTree(messagesOrTree)
    : messagesOrTree;

  return tree.rootThreadItems
    .filter((item): item is ThreadSubRunItem => item.kind === 'subRun')
    .map((item) => tree.subRuns.get(item.subRunId))
    .filter((view): view is SubRunViewData => view !== undefined);
}

/// 从消息列表构建父视图摘要投影。
/// 直接消费 childSessionNotification，避免再从 mixed-thread 生命周期反推父摘要。
/// 父视图只消费摘要，不消费子会话原始事件流，不暴露 raw JSON。
