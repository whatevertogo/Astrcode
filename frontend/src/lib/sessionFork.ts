import type { ConversationControlState, Message } from '../types';

export function resolveForkTurnIdFromMessage(
  message: Message,
  control?: ConversationControlState | null
): string | null {
  switch (message.kind) {
    case 'user':
    case 'assistant':
    case 'toolCall':
    case 'compact':
      break;
    default:
      return null;
  }

  const turnId = message.turnId ?? null;
  if (!turnId) {
    return null;
  }
  if (control?.activeTurnId && control.activeTurnId === turnId) {
    return null;
  }
  if (message.kind === 'assistant' && message.streaming) {
    return null;
  }
  return turnId;
}
