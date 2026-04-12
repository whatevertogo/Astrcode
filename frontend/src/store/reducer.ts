//! # Reducer + Action Handlers
//!
//! Central state management for the app.
//! The top-level reducer now only routes actions to focused handler groups.

import type { Action, AppState, AtomicAction } from '../types';
import {
  findAssistantMessageIndex,
  findPromptMetricsMessageIndex,
  findToolCallMessageIndex,
  findUserMessageIndex,
  mapProject,
  mapSession,
  moveUpdatedMessageToTail,
  upsertAssistantTurnMessage,
} from './reducerHelpers';
import { handleProjectedMessageAction } from './reducerMessageProjection';
import { buildSubRunThreadTree } from '../lib/subRunView';

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
    default:
      return null;
  }
}

function handleCatalogAction(state: AppState, action: Action): AppState | null {
  switch (action.type) {
    case 'ADD_PROJECT':
      return {
        ...state,
        projects: [action.project, ...state.projects],
        activeProjectId: action.project.id,
        activeSessionId: action.project.sessions[0]?.id ?? null,
        activeSubRunPath: [],
      };
    case 'ADD_SESSION':
      return {
        ...mapProject(state, action.projectId, (project) => ({
          ...project,
          sessions: [action.session, ...project.sessions],
        })),
        activeProjectId: action.projectId,
        activeSessionId: action.session.id,
        activeSubRunPath: [],
      };
    case 'TOGGLE_EXPAND':
      return mapProject(state, action.projectId, (project) => ({
        ...project,
        isExpanded: !project.isExpanded,
      }));
    case 'DELETE_PROJECT': {
      const projects = state.projects.filter((project) => project.id !== action.projectId);
      let activeProjectId = state.activeProjectId;
      let activeSessionId = state.activeSessionId;
      if (activeProjectId === action.projectId) {
        activeProjectId = projects[0]?.id ?? null;
        activeSessionId = projects[0]?.sessions[0]?.id ?? null;
      }
      return { ...state, projects, activeProjectId, activeSessionId, activeSubRunPath: [] };
    }
    case 'DELETE_SESSION': {
      const nextState = mapProject(state, action.projectId, (project) => ({
        ...project,
        sessions: project.sessions.filter((session) => session.id !== action.sessionId),
      }));
      let activeSessionId = nextState.activeSessionId;
      let activeProjectId = nextState.activeProjectId;
      if (state.activeSessionId === action.sessionId) {
        const project = nextState.projects.find((item) => item.id === action.projectId);
        activeSessionId = project?.sessions[0]?.id ?? null;
        activeProjectId = project?.id ?? nextState.projects[0]?.id ?? null;
      }
      return { ...nextState, activeProjectId, activeSessionId, activeSubRunPath: [] };
    }
    case 'REPLACE_SESSION_MESSAGES':
      return mapSession(state, action.sessionId, (session) => ({
        ...session,
        messages: action.messages,
        subRunThreadTree: buildSubRunThreadTree(action.messages),
      }));
    default:
      return null;
  }
}

function reduceAtomicAction(state: AppState, action: AtomicAction): AppState {
  return (
    handleUiStateAction(state, action) ??
    handleNavigationAction(state, action) ??
    handleCatalogAction(state, action) ??
    handleProjectedMessageAction(state, action) ??
    state
  );
}

export function reducer(state: AppState, action: Action): AppState {
  if (action.type === 'APPLY_AGENT_EVENTS_BATCH') {
    return action.actions.reduce(reduceAtomicAction, state);
  }
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
