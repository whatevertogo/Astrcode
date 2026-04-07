import type { SessionEventScope } from '../types';

export interface SessionEventFilterQuery {
  subRunId?: string;
  scope?: SessionEventScope;
}

export interface SessionViewLocationState {
  sessionId: string | null;
  subRunPath: string[];
}

const SESSION_ID_PARAM = 'sessionId';
const SUBRUN_PATH_PARAM = 'subRunPath';

export function buildFocusedSubRunFilter(
  subRunPath: string[]
): SessionEventFilterQuery | undefined {
  const focusedSubRunId = subRunPath[subRunPath.length - 1];
  if (!focusedSubRunId) {
    return undefined;
  }
  return buildSubRunSelfFilter(focusedSubRunId);
}

export function buildSubRunSelfFilter(subRunId: string): SessionEventFilterQuery {
  return {
    subRunId,
    scope: 'self',
  };
}

export function buildSubRunChildrenFilter(subRunId: string): SessionEventFilterQuery {
  return {
    subRunId,
    scope: 'directChildren',
  };
}

export function buildSessionEventQueryString(options?: {
  afterEventId?: string | null;
  filter?: SessionEventFilterQuery;
}): string {
  const params = new URLSearchParams();
  if (options?.afterEventId) {
    params.set('afterEventId', options.afterEventId);
  }
  if (options?.filter?.subRunId) {
    params.set('subRunId', options.filter.subRunId);
    params.set('scope', options.filter.scope ?? 'subtree');
  }
  const queryString = params.toString();
  return queryString ? `?${queryString}` : '';
}

export function readSessionViewLocation(currentHref: string): SessionViewLocationState {
  const url = new URL(currentHref);
  const sessionId = url.searchParams.get(SESSION_ID_PARAM);
  const subRunPathValue = url.searchParams.get(SUBRUN_PATH_PARAM) ?? '';
  const subRunPath = subRunPathValue
    .split(',')
    .map((value) => value.trim())
    .filter((value) => value.length > 0);
  return {
    sessionId,
    subRunPath,
  };
}

export function buildSessionViewLocationHref(
  currentHref: string,
  state: SessionViewLocationState
): string {
  const url = new URL(currentHref);
  if (state.sessionId) {
    url.searchParams.set(SESSION_ID_PARAM, state.sessionId);
  } else {
    url.searchParams.delete(SESSION_ID_PARAM);
  }
  if (state.subRunPath.length > 0 && state.sessionId) {
    url.searchParams.set(SUBRUN_PATH_PARAM, state.subRunPath.join(','));
  } else {
    url.searchParams.delete(SUBRUN_PATH_PARAM);
  }
  return `${url.pathname}${url.search}${url.hash}`;
}
