import type { Message, SubRunFinishMessage, SubRunStartMessage } from '../types';

interface IndexedMessage {
  index: number;
  message: Message;
}

interface SubRunRecord {
  subRunId: string;
  startMessage?: SubRunStartMessage;
  finishMessage?: SubRunFinishMessage;
  ownBodyEntries: IndexedMessage[];
  childSessionId?: string;
  title: string;
  parentTurnId?: string;
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

function isSubRunStartMessage(message: Message): message is SubRunStartMessage {
  return message.kind === 'subRunStart';
}

function isSubRunFinishMessage(message: Message): message is SubRunFinishMessage {
  return message.kind === 'subRunFinish';
}

function isSubRunLifecycleMessage(message: Message): boolean {
  return isSubRunStartMessage(message) || isSubRunFinishMessage(message);
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
  if (message.kind === 'compact') {
    return `${message.id}:compact:${message.summary.length}`;
  }
  if (message.kind === 'user') {
    return `${message.id}:user:${message.text.length}`;
  }
  if (message.kind === 'subRunStart') {
    return `${message.id}:subRunStart:${message.subRunId ?? 'unknown'}`;
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
    parentSubRunId: null,
    directChildSubRunIds: [],
    firstMessageIndex,
    startIndex: firstMessageIndex,
  };
  records.set(subRunId, created);
  return created;
}

function buildSubRunIndex(messages: Message[]): SubRunIndex {
  const records = new Map<string, SubRunRecord>();
  const rootEntries: IndexedMessage[] = [];
  const turnOwnerMap = new Map<string, string | null>();

  messages.forEach((message, index) => {
    if (message.turnId && !isSubRunLifecycleMessage(message) && !turnOwnerMap.has(message.turnId)) {
      turnOwnerMap.set(message.turnId, message.subRunId ?? null);
    }

    if (!message.subRunId) {
      rootEntries.push({ index, message });
      return;
    }

    const record = getOrCreateRecord(records, message.subRunId, index);
    if (message.parentTurnId && !record.parentTurnId) {
      record.parentTurnId = message.parentTurnId;
    }

    if (isSubRunStartMessage(message)) {
      record.startMessage ??= message;
      record.startIndex = Math.min(record.startIndex, index);
      return;
    }

    if (isSubRunFinishMessage(message)) {
      record.finishMessage ??= message;
      return;
    }

    record.ownBodyEntries.push({ index, message });
  });

  const orderedRecords = [...records.values()].sort(
    (left, right) => left.firstMessageIndex - right.firstMessageIndex
  );

  orderedRecords.forEach((record) => {
    record.parentSubRunId =
      record.parentTurnId !== undefined ? (turnOwnerMap.get(record.parentTurnId) ?? null) : null;
    record.childSessionId =
      record.startMessage?.childSessionId ?? record.finishMessage?.childSessionId;
    record.title = deriveSubRunTitle(
      record.subRunId,
      record.startMessage,
      record.finishMessage,
      record.ownBodyEntries.map((entry) => entry.message)
    );
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
