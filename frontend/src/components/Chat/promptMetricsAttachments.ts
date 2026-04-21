import type { AssistantMessage, PromptMetricsMessage, ThreadItem } from '../../types';

export function resolvePromptMetricsAttachments(
  items: ThreadItem[]
): Map<string, PromptMetricsMessage> {
  const attachments = new Map<string, PromptMetricsMessage>();
  const attachedMetricIds = new Set<string>();

  for (const item of items) {
    if (
      item.kind !== 'message' ||
      item.message.kind !== 'promptMetrics' ||
      item.message.stepIndex === undefined
    ) {
      continue;
    }
    const assistant = findAssistantByStep(items, item.message);
    if (!assistant) {
      continue;
    }
    attachments.set(assistant.id, item.message);
    attachedMetricIds.add(item.message.id);
  }

  for (let index = 0; index < items.length; index += 1) {
    const item = items[index];
    if (item.kind !== 'message' || item.message.kind !== 'assistant') {
      continue;
    }
    if (attachments.has(item.message.id)) {
      continue;
    }

    let hasMoreAssistantInTurn = false;
    const currentTurnId = item.message.turnId;

    for (let nextIndex = index + 1; nextIndex < items.length; nextIndex += 1) {
      const nextThreadItem = items[nextIndex];
      if (nextThreadItem.kind !== 'message') {
        continue;
      }
      if (
        nextThreadItem.message.kind === 'assistant' &&
        nextThreadItem.message.turnId === currentTurnId
      ) {
        hasMoreAssistantInTurn = true;
        break;
      }
      if (
        nextThreadItem.message.kind === 'user' ||
        (nextThreadItem.message.kind === 'assistant' &&
          nextThreadItem.message.turnId !== currentTurnId)
      ) {
        break;
      }
    }

    if (hasMoreAssistantInTurn) {
      continue;
    }

    for (let nextIndex = index + 1; nextIndex < items.length; nextIndex += 1) {
      const nextThreadItem = items[nextIndex];
      if (nextThreadItem.kind !== 'message') {
        continue;
      }
      if (
        nextThreadItem.message.kind === 'promptMetrics' &&
        !attachedMetricIds.has(nextThreadItem.message.id)
      ) {
        attachments.set(item.message.id, nextThreadItem.message);
        attachedMetricIds.add(nextThreadItem.message.id);
        break;
      }
      if (nextThreadItem.message.kind === 'assistant' || nextThreadItem.message.kind === 'user') {
        break;
      }
    }
  }

  return attachments;
}

function findAssistantByStep(
  items: ThreadItem[],
  metrics: PromptMetricsMessage
): AssistantMessage | undefined {
  for (const item of items) {
    if (item.kind !== 'message' || item.message.kind !== 'assistant') {
      continue;
    }
    if (item.message.turnId !== metrics.turnId) {
      continue;
    }
    if (item.message.stepIndex === metrics.stepIndex) {
      return item.message;
    }
  }
  return undefined;
}
