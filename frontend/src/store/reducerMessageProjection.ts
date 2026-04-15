import type { Action, AppState } from '../types';
import { mapSession } from './reducerHelpers';
import { buildSubRunThreadTree } from '../lib/subRunView';

export function handleProjectedMessageAction(state: AppState, action: Action): AppState | null {
  switch (action.type) {
    case 'ADD_MESSAGE':
      return mapSession(state, action.sessionId, (session) => {
        let title = session.title;
        if (
          action.message.kind === 'user' &&
          session.messages.filter((message) => message.kind === 'user').length === 0
        ) {
          title = action.message.text.slice(0, 20) || '新会话';
        }

        const nextMessages = [...session.messages, action.message];
        const nextSession = {
          ...session,
          messages: nextMessages,
          subRunThreadTree: buildSubRunThreadTree(nextMessages),
        };

        return title === session.title
          ? nextSession
          : {
              ...nextSession,
              title,
            };
      });

    default:
      return null;
  }
}
