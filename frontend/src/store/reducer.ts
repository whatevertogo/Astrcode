//! # Reducer + Action Handlers
//!
//! Central state management for the app.
//! The top-level reducer now only routes actions to focused handler groups.

import type { Action, AppState } from '../types';
import {
  findAssistantMessageIndex,
  findPromptMetricsMessageIndex,
  findToolCallMessageIndex,
  findUserMessageIndex,
  mapProject,
  moveUpdatedMessageToTail,
  upsertAssistantTurnMessage,
} from './reducerHelpers';
import { handleProjectedMessageAction } from './reducerMessageProjection';
import { replaceSessionMessages } from './utils';

export {
  findAssistantMessageIndex,
  findPromptMetricsMessageIndex,
  findToolCallMessageIndex,
  findUserMessageIndex,
  moveUpdatedMessageToTail,
  upsertAssistantTurnMessage,
};

function handleUiStateAction(state: AppState, action: Action): AppState | null {
  switch (action.type) {
    case 'SET_PHASE':
      if (state.phase === action.phase) {
        return state;
      }
      return { ...state, phase: action.phase };
    case 'INITIALIZE':
      return {
        ...state,
        projects: action.projects,
        activeProjectId: action.activeProjectId,
        activeSessionId: action.activeSessionId,
        activeSubRunPath: action.activeSubRunPath ?? [],
      };
    default:
      return null;
  }
}

function handleNavigationAction(state: AppState, action: Action): AppState | null {
  switch (action.type) {
    case 'PUSH_ACTIVE_SUBRUN':
      if (state.activeSubRunPath[state.activeSubRunPath.length - 1] === action.subRunId) {
        return state;
      }
      return {
        ...state,
        activeSubRunPath: [...state.activeSubRunPath, action.subRunId],
      };
    case 'POP_ACTIVE_SUBRUN':
      if (state.activeSubRunPath.length === 0) {
        return state;
      }
      return {
        ...state,
        activeSubRunPath: state.activeSubRunPath.slice(0, -1),
      };
    case 'SET_ACTIVE_SUBRUN_PATH':
      if (
        state.activeSubRunPath.length === action.subRunPath.length &&
        state.activeSubRunPath.every((subRunId, index) => subRunId === action.subRunPath[index])
      ) {
        return state;
      }
      return {
        ...state,
        activeSubRunPath: [...action.subRunPath],
      };
    case 'CLEAR_ACTIVE_SUBRUN_PATH':
      if (state.activeSubRunPath.length === 0) {
        return state;
      }
      return {
        ...state,
        activeSubRunPath: [],
      };
    case 'SET_ACTIVE':
      // Why: 切换会话后默认回到父摘要入口，不能把上一会话的子线程浏览路径继续沿用过来。
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: action.sessionId,
        activeSubRunPath: [],
      };
    case 'SET_ACTIVE_PROJECT':
      if (state.activeProjectId === action.projectId && state.activeSessionId === null) {
        return state;
      }
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: null,
        activeSubRunPath: [],
      };
    default:
      return null;
  }
}

function handleCatalogAction(state: AppState, action: Action): AppState | null {
  switch (action.type) {
    case 'TOGGLE_EXPAND':
      return mapProject(state, action.projectId, (project) => ({
        ...project,
        isExpanded: !project.isExpanded,
      }));
    case 'REPLACE_SESSION_MESSAGES':
      return {
        ...state,
        projects: replaceSessionMessages(
          state.projects,
          action.sessionId,
          action.messages,
          action.subRunThreadTree
        ),
      };
    default:
      return null;
  }
}

function reduceAtomicAction(state: AppState, action: Action): AppState {
  return (
    handleUiStateAction(state, action) ??
    handleNavigationAction(state, action) ??
    handleCatalogAction(state, action) ??
    handleProjectedMessageAction(state, action) ??
    state
  );
}

export function reducer(state: AppState, action: Action): AppState {
  return reduceAtomicAction(state, action);
}

export function makeInitialState(): AppState {
  return {
    projects: [],
    activeProjectId: null,
    activeSessionId: null,
    activeSubRunPath: [],
    phase: 'idle',
  };
}
