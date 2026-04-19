import type { Action, AppState } from '../types';
import { mapSession } from './reducerHelpers';
import { buildSubRunThreadTree } from '../lib/subRunView';
import { deriveSessionTitleFromMessages } from './utils';

export function handleProjectedMessageAction(state: AppState, action: Action): AppState | null {
  switch (action.type) {
    case 'ADD_MESSAGE':
      return mapSession(state, action.sessionId, (session) => {
        const nextMessages = [...session.messages, action.message];
        const title = deriveSessionTitleFromMessages(nextMessages, session.title);
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
