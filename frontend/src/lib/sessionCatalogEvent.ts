import type { SessionCatalogEventPayload } from '../types';
import { asRecord, pickString, safeStringify } from './shared';

const SUPPORTED_PROTOCOL_VERSION = 1;

export function normalizeSessionCatalogEvent(raw: unknown): SessionCatalogEventPayload {
  const payload = asRecord(raw);
  if (!payload) {
    throw new Error(`session catalog event payload is not an object: ${safeStringify(raw)}`);
  }

  const protocolVersion = payload.protocolVersion;
  if (protocolVersion !== SUPPORTED_PROTOCOL_VERSION) {
    throw new Error(`unsupported session catalog protocolVersion ${String(protocolVersion)}`);
  }

  const event = pickString(payload, 'event');
  const data = asRecord(payload.data);
  if (!event || !data) {
    throw new Error(`session catalog event is missing event/data: ${safeStringify(raw)}`);
  }

  if (event === 'sessionCreated') {
    const sessionId = pickString(data, 'sessionId', 'session_id');
    if (!sessionId) {
      throw new Error(`sessionCreated requires sessionId: ${safeStringify(raw)}`);
    }
    return { event, data: { sessionId } };
  }

  if (event === 'sessionDeleted') {
    const sessionId = pickString(data, 'sessionId', 'session_id');
    if (!sessionId) {
      throw new Error(`sessionDeleted requires sessionId: ${safeStringify(raw)}`);
    }
    return { event, data: { sessionId } };
  }

  if (event === 'projectDeleted') {
    const workingDir = pickString(data, 'workingDir', 'working_dir');
    if (!workingDir) {
      throw new Error(`projectDeleted requires workingDir: ${safeStringify(raw)}`);
    }
    return { event, data: { workingDir } };
  }

  if (event === 'sessionBranched') {
    const sessionId = pickString(data, 'sessionId', 'session_id');
    const sourceSessionId = pickString(data, 'sourceSessionId', 'source_session_id');
    if (!sessionId || !sourceSessionId) {
      throw new Error(
        `sessionBranched requires sessionId and sourceSessionId: ${safeStringify(raw)}`
      );
    }
    return { event, data: { sessionId, sourceSessionId } };
  }

  throw new Error(`unsupported session catalog event: ${safeStringify(raw)}`);
}
