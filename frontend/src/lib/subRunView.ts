import type {
  ChildSessionNotificationMessage,
  Message,
  SubRunFinishMessage,
  SubRunStartMessage,
  SubRunThreadTree,
  SubRunViewData,
  ThreadItem,
  ThreadSubRunItem,
} from '../types';

import { logger } from './logger';

export type { SubRunThreadTree, SubRunViewData, ThreadItem, ThreadSubRunItem } from '../types';

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
  latestNotification?: ChildSessionNotificationMessage;
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
  aliases: Map<string, string>;
}

function wouldCreateSubRunCycle(
  records: Map<string, SubRunRecord>,
  childSubRunId: string,
  parentSubRunId: string | null
): boolean {
  let current = parentSubRunId;
  const seen = new Set<string>();

  while (current) {
    if (current === childSubRunId) {
      return true;
    }
    if (seen.has(current)) {
      return true;
    }
    seen.add(current);
    current = records.get(current)?.parentSubRunId ?? null;
  }

  return false;
}

export interface SubRunPathView {
  validPath: string[];
  views: SubRunViewData[];
  activeView: SubRunViewData | null;
}

export function createEmptySubRunThreadTree(): SubRunThreadTree {
  return {
    rootThreadItems: [],
    rootStreamFingerprint: '',
    subRuns: new Map(),
  };
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
    }:${message.metadata === undefined ? '' : JSON.stringify(message.metadata)}`;
  }
  if (message.kind === 'toolStream') {
    return `${message.id}:toolStream:${message.toolCallId}:${message.stream}:${message.status}:${message.content.length}`;
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
    return `${message.id}:childNotification:${message.childRef.subRunId}:${message.notificationKind}:${message.status}:${message.delivery?.idempotencyKey ?? 'legacy'}`;
  }
  return `${message.id}:subRunFinish:${message.subRunId ?? 'unknown'}:${message.result.status}`;
}

function remapMessageReference(
  previous: Message,
  nextById: ReadonlyMap<string, Message>
): Message | null {
  const next = nextById.get(previous.id);
  if (!next) {
    return null;
  }
  // Why: 同一个消息 ID 在投影里如果 kind 被替换，说明结构语义已经变化，
  // 增量 patch 继续复用旧 tree 会把消息挂到错误分支，必须回退全量重建。
  if (next.kind !== previous.kind) {
    return null;
  }
  if (hasSubRunTopologyChange(previous, next)) {
    return null;
  }
  return next;
}

function sameSpawnedAgentRef(left: SpawnedAgentRef | null, right: SpawnedAgentRef | null): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }
  return (
    left.subRunId === right.subRunId &&
    left.agentId === right.agentId &&
    left.childSessionId === right.childSessionId
  );
}

function sameChildRefTopology(
  left: ChildSessionNotificationMessage['childRef'],
  right: ChildSessionNotificationMessage['childRef']
): boolean {
  return (
    left.agentId === right.agentId &&
    left.subRunId === right.subRunId &&
    left.parentAgentId === right.parentAgentId &&
    left.parentSubRunId === right.parentSubRunId &&
    left.openSessionId === right.openSessionId
  );
}

function hasSubRunTopologyChange(previous: Message, next: Message): boolean {
  if (
    previous.subRunId !== next.subRunId ||
    previous.agentId !== next.agentId ||
    previous.parentSubRunId !== next.parentSubRunId ||
    previous.childSessionId !== next.childSessionId
  ) {
    return true;
  }

  if (previous.kind === 'toolCall' && next.kind === 'toolCall') {
    return !sameSpawnedAgentRef(pickSpawnedAgentRef(previous), pickSpawnedAgentRef(next));
  }

  if (previous.kind === 'childSessionNotification' && next.kind === 'childSessionNotification') {
    return !sameChildRefTopology(previous.childRef, next.childRef);
  }

  return false;
}

function patchThreadItems(
  items: ThreadItem[],
  nextById: ReadonlyMap<string, Message>,
  usedMessageIds: Set<string>
): ThreadItem[] | null {
  const patched: ThreadItem[] = [];
  for (const item of items) {
    if (item.kind === 'subRun') {
      patched.push(item);
      continue;
    }
    const nextMessage = remapMessageReference(item.message, nextById);
    if (!nextMessage) {
      return null;
    }
    usedMessageIds.add(nextMessage.id);
    patched.push({ kind: 'message', message: nextMessage });
  }
  return patched;
}

function buildThreadItemsFingerprint(
  items: ThreadItem[],
  subRunFingerprints: ReadonlyMap<string, string>
): string {
  return items
    .map((item) =>
      item.kind === 'message'
        ? buildMessageFingerprint(item.message)
        : `subRun:${item.subRunId}:${subRunFingerprints.get(item.subRunId) ?? item.subRunId}`
    )
    .join('|');
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
  if (message.kind !== 'toolCall' || message.toolName !== 'spawn' || message.status !== 'ok') {
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

function buildStableSubRunIdentity(agentId?: string, childSessionId?: string): string | undefined {
  if (agentId) {
    return `agent:${agentId}`;
  }
  if (childSessionId) {
    return `child-session:${childSessionId}`;
  }
  return undefined;
}

function registerSubRunAlias(
  aliases: Map<string, string>,
  stableOwners: Map<string, string>,
  ref: SpawnedAgentRef
): string {
  const stableIdentity = buildStableSubRunIdentity(ref.agentId, ref.childSessionId);
  const canonicalSubRunId =
    (stableIdentity ? stableOwners.get(stableIdentity) : undefined) ??
    aliases.get(ref.subRunId) ??
    ref.subRunId;

  aliases.set(ref.subRunId, canonicalSubRunId);
  if (stableIdentity && !stableOwners.has(stableIdentity)) {
    stableOwners.set(stableIdentity, canonicalSubRunId);
  }

  return canonicalSubRunId;
}

function pickStableSubRunRef(message: Message): SpawnedAgentRef | null {
  if (!message.subRunId) {
    return null;
  }

  return {
    subRunId: message.subRunId,
    agentId:
      message.agentId ??
      (message.kind === 'childSessionNotification' ? message.childRef.agentId : undefined),
    childSessionId: pickOpenableChildSessionId(message),
  };
}

function buildSubRunIndex(messages: Message[]): SubRunIndex {
  const records = new Map<string, SubRunRecord>();
  const rootEntries: IndexedMessage[] = [];
  const aliases = new Map<string, string>();
  const stableOwners = new Map<string, string>();

  messages.forEach((message) => {
    const stableRef = pickStableSubRunRef(message);
    if (stableRef) {
      registerSubRunAlias(aliases, stableOwners, stableRef);
    }
    const spawnedAgentRef = pickSpawnedAgentRef(message);
    if (spawnedAgentRef) {
      registerSubRunAlias(aliases, stableOwners, spawnedAgentRef);
    }
  });

  // 推导 subRun 嵌套关系需要两步：
  // 第一遍：从 body 消息（非 start/finish）收集 turnId → subRunId 映射
  const turnToSubRun = new Map<string, string>();
  for (const message of messages) {
    if (!message.subRunId) continue;
    if (message.kind === 'subRunStart' || message.kind === 'subRunFinish') continue;
    if (message.turnId) {
      turnToSubRun.set(message.turnId, aliases.get(message.subRunId) ?? message.subRunId);
    }
  }

  // 第二遍：处理消息，用映射 + 栈确定父关系
  const subRunStack: string[] = [];

  messages.forEach((message, index) => {
    if (!message.subRunId) {
      rootEntries.push({ index, message });
      return;
    }

    const subRunId = aliases.get(message.subRunId) ?? message.subRunId;
    const record = getOrCreateRecord(records, subRunId, index);
    if (message.agentId && !record.agentId) {
      record.agentId = message.agentId;
    }
    const childSessionId = pickOpenableChildSessionId(message);
    if (childSessionId && !record.childSessionId) {
      record.childSessionId = childSessionId;
    }

    if (isSubRunStartMessage(message)) {
      record.startMessage = message;
      record.finishMessage = undefined;
      record.startIndex = Math.min(record.startIndex, index);
      if (message.parentTurnId) {
        record.hasDescriptorLineage = true;
      }

      // 父关系推导：先从 turnToSubRun 精确匹配，再退化到栈
      const turnParent = turnToSubRun.get(message.turnId ?? '') ?? null;
      if (!record.parentSubRunId && turnParent && turnParent !== subRunId) {
        record.parentSubRunId = turnParent;
      }
      // 栈退化：turnToSubRun 未命中时，检查栈顶 start 的 turnId 判断兄弟还是父子
      if (!record.parentSubRunId && subRunStack.length > 0) {
        const stackTopId = subRunStack[subRunStack.length - 1];
        const stackTopRecord = records.get(stackTopId);
        if (stackTopId !== subRunId) {
          if (stackTopRecord?.startMessage?.turnId === message.turnId) {
            // 相同 turnId → 兄弟关系，继承栈顶的父
            record.parentSubRunId = stackTopRecord?.parentSubRunId ?? null;
          } else {
            // 不同 turnId → 栈顶是父
            record.parentSubRunId = stackTopId;
          }
        }
      }

      subRunStack.push(subRunId);
      return;
    }

    if (isSubRunFinishMessage(message)) {
      record.finishMessage = message;
      if (message.parentTurnId) {
        record.hasDescriptorLineage = true;
      }
      // 从栈中移除已完成的 subRun（可能在栈的任意深度）
      const stackIndex = subRunStack.lastIndexOf(subRunId);
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
      if (!record.parentSubRunId) {
        record.parentSubRunId =
          message.childRef?.parentSubRunId ?? message.parentSubRunId ?? record.parentSubRunId;
      }
      record.latestNotification = message;
    }

    record.ownBodyEntries.push({ index, message });
  });

  messages.forEach((message, index) => {
    const spawnedAgentRef = pickSpawnedAgentRef(message);
    if (!spawnedAgentRef) {
      return;
    }

    // Why: 历史回放偶发缺少 subRun lifecycle 时，spawn 的 agentRef 仍然能稳定标识子执行；
    // 用它补建占位记录，避免父会话把已启动的子 Agent 直接“吃掉”。
    const canonicalSubRunId = registerSubRunAlias(aliases, stableOwners, spawnedAgentRef);
    const record = getOrCreateRecord(records, canonicalSubRunId, index);
    record.startIndex = Math.min(record.startIndex, index);
    if (!record.agentId && spawnedAgentRef.agentId) {
      record.agentId = spawnedAgentRef.agentId;
    }
    if (!record.childSessionId && spawnedAgentRef.childSessionId) {
      record.childSessionId = spawnedAgentRef.childSessionId;
    }
    const parentSubRunId = message.subRunId
      ? (aliases.get(message.subRunId) ?? message.subRunId)
      : null;
    if (!record.parentSubRunId && parentSubRunId && parentSubRunId !== canonicalSubRunId) {
      record.parentSubRunId = parentSubRunId;
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
    if (
      record.parentSubRunId === record.subRunId ||
      !records.has(record.parentSubRunId) ||
      wouldCreateSubRunCycle(records, record.subRunId, record.parentSubRunId)
    ) {
      // Why: lineage 数据可能来自不同事件源（start、notification、spawn fallback），
      // 一旦形成自环或环引用，递归渲染会直接栈溢出；这里降级为根节点，保证 UI 可继续工作。
      record.parentSubRunId = null;
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
    if (!parentRecord.directChildSubRunIds.includes(record.subRunId)) {
      parentRecord.directChildSubRunIds.push(record.subRunId);
    }
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

  return { records, rootEntries, aliases };
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
  const materializing = new Set<string>();

  const materializeSubRun = (subRunId: string): SubRunViewData | null => {
    const cached = subRuns.get(subRunId);
    if (cached) {
      return cached;
    }

    const record = index.records.get(subRunId);
    if (!record) {
      return null;
    }
    if (materializing.has(subRunId)) {
      // Why: 即便上游 lineage 清洗漏网，视图层也必须防止递归爆栈。
      logger.warn(
        'subRunView',
        'sub-run tree detected recursive lineage, skipping recursive materialization',
        {
          subRunId,
        }
      );
      return null;
    }

    materializing.add(subRunId);

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
      latestNotification: record.latestNotification,
      bodyMessages: record.ownBodyEntries.map((entry) => entry.message),
      threadItems,
      streamFingerprint,
      childSessionId: record.childSessionId,
      parentSubRunId: record.parentSubRunId,
      directChildSubRunIds: record.directChildSubRunIds,
      hasDescriptorLineage: record.hasDescriptorLineage,
    };
    subRuns.set(subRunId, view);
    materializing.delete(subRunId);
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

  index.aliases.forEach((canonicalSubRunId, aliasSubRunId) => {
    if (aliasSubRunId === canonicalSubRunId) {
      return;
    }
    const canonicalView = subRuns.get(canonicalSubRunId);
    if (canonicalView) {
      subRuns.set(aliasSubRunId, canonicalView);
    }
  });

  return { rootThreadItems, rootStreamFingerprint, subRuns };
}

/// 在不改变 sub-run 结构的前提下，增量刷新 thread tree 中的消息引用与 fingerprint。
///
/// 适用场景：assistant/tool/promptMetrics 文本或状态更新（消息 ID 不变）。
/// 若检测到结构不兼容（消息缺失、kind 变化），返回 `null` 让调用方回退全量重建。
export function patchSubRunThreadTreeMessages(
  previousTree: SubRunThreadTree,
  nextMessages: Message[]
): SubRunThreadTree | null {
  const nextById = new Map(nextMessages.map((message) => [message.id, message] as const));
  const usedMessageIds = new Set<string>();

  const patchedRootThreadItems = patchThreadItems(
    previousTree.rootThreadItems,
    nextById,
    usedMessageIds
  );
  if (!patchedRootThreadItems) {
    return null;
  }

  const patchedSubRuns = new Map<string, SubRunViewData>();
  for (const [subRunId, view] of previousTree.subRuns.entries()) {
    const patchedBodyMessages: Message[] = [];
    for (const message of view.bodyMessages) {
      const nextMessage = remapMessageReference(message, nextById);
      if (!nextMessage) {
        return null;
      }
      usedMessageIds.add(nextMessage.id);
      patchedBodyMessages.push(nextMessage);
    }

    const patchedStartMessage = view.startMessage
      ? remapMessageReference(view.startMessage, nextById)
      : undefined;
    if (view.startMessage && !patchedStartMessage) {
      return null;
    }
    if (patchedStartMessage) {
      usedMessageIds.add(patchedStartMessage.id);
    }

    const patchedFinishMessage = view.finishMessage
      ? remapMessageReference(view.finishMessage, nextById)
      : undefined;
    if (view.finishMessage && !patchedFinishMessage) {
      return null;
    }
    if (patchedFinishMessage) {
      usedMessageIds.add(patchedFinishMessage.id);
    }

    const patchedThreadItems = patchThreadItems(view.threadItems, nextById, usedMessageIds);
    if (!patchedThreadItems) {
      return null;
    }
    const patchedLatestNotification = view.latestNotification
      ? remapMessageReference(view.latestNotification, nextById)
      : null;
    if (view.latestNotification && !patchedLatestNotification) {
      return null;
    }
    const latestNotification =
      patchedLatestNotification?.kind === 'childSessionNotification'
        ? patchedLatestNotification
        : undefined;

    patchedSubRuns.set(subRunId, {
      ...view,
      startMessage: patchedStartMessage as SubRunStartMessage | undefined,
      finishMessage: patchedFinishMessage as SubRunFinishMessage | undefined,
      latestNotification,
      bodyMessages: patchedBodyMessages,
      threadItems: patchedThreadItems,
    });
  }

  // Why: 旧 tree 无法覆盖的新增消息意味着结构或拓扑发生变化，
  // 继续增量 patch 会丢消息；此时必须回退全量重建。
  if (usedMessageIds.size !== nextById.size) {
    return null;
  }

  const subRunFingerprints = new Map<string, string>();
  const visiting = new Set<string>();
  const resolveSubRunFingerprint = (subRunId: string): string => {
    const cached = subRunFingerprints.get(subRunId);
    if (cached) {
      return cached;
    }

    const view = patchedSubRuns.get(subRunId);
    if (!view) {
      return subRunId;
    }

    if (visiting.has(subRunId)) {
      return view.streamFingerprint;
    }
    visiting.add(subRunId);

    const fingerprint = view.threadItems
      .map((item) =>
        item.kind === 'message'
          ? buildMessageFingerprint(item.message)
          : `subRun:${item.subRunId}:${resolveSubRunFingerprint(item.subRunId)}`
      )
      .join('|');

    visiting.delete(subRunId);
    subRunFingerprints.set(subRunId, fingerprint);
    return fingerprint;
  };

  for (const subRunId of patchedSubRuns.keys()) {
    resolveSubRunFingerprint(subRunId);
  }

  for (const [subRunId, view] of patchedSubRuns.entries()) {
    patchedSubRuns.set(subRunId, {
      ...view,
      streamFingerprint: subRunFingerprints.get(subRunId) ?? view.streamFingerprint,
    });
  }

  const rootStreamFingerprint = buildThreadItemsFingerprint(
    patchedRootThreadItems,
    subRunFingerprints
  );

  return {
    rootThreadItems: patchedRootThreadItems,
    rootStreamFingerprint,
    subRuns: patchedSubRuns,
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

/// 从消息列表构建父视图 delivery 投影。
/// 直接消费 childSessionNotification，避免再从 mixed-thread 生命周期反推父摘要。
/// 父视图只消费 typed delivery，不消费子会话原始事件流，不暴露 raw JSON。
