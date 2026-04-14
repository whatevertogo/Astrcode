//! # Debug Runtime API Endpoints
//!
//! Debug-only runtime observability accessors.

import type {
  RuntimeDebugOverview,
  RuntimeDebugTimeline,
  SessionDebugAgents,
  SessionDebugTrace,
} from '../../types';
import { requestJson } from './client';

export async function getDebugRuntimeOverview(): Promise<RuntimeDebugOverview> {
  return requestJson<RuntimeDebugOverview>('/api/debug/runtime/overview');
}

export async function getDebugRuntimeTimeline(): Promise<RuntimeDebugTimeline> {
  return requestJson<RuntimeDebugTimeline>('/api/debug/runtime/timeline');
}

export async function getDebugSessionTrace(sessionId: string): Promise<SessionDebugTrace> {
  return requestJson<SessionDebugTrace>(
    `/api/debug/sessions/${encodeURIComponent(sessionId)}/trace`
  );
}

export async function getDebugSessionAgents(sessionId: string): Promise<SessionDebugAgents> {
  return requestJson<SessionDebugAgents>(
    `/api/debug/sessions/${encodeURIComponent(sessionId)}/agents`
  );
}
