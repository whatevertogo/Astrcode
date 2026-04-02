//! # SSE Consumer
//!
//! Parses server-sent events and dispatches them to a callback.
//!
//! ## Protocol
//!
//! Each SSE event consists of `id:`, optional `event:`, and one or more `data:` lines.
//! Empty lines mark the end of an event. This helper accumulates raw payload strings
//! and calls `onMessage` once per completed event so downstream code (agentEvent.ts)
//! can normalize without worrying about stream framing.

type SseStreamCloseReason = 'ended' | 'aborted';

export async function consumeSseStream(
  response: Response,
  onMessage: (payload: string, eventId: string | null, eventType: string) => void,
  signal: AbortSignal
): Promise<SseStreamCloseReason> {
  if (!response.body) {
    throw new Error('event stream response has no body');
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let dataLines: string[] = [];
  let eventId: string | null = null;
  let eventType = 'message';

  const flushEvent = () => {
    if (dataLines.length === 0) {
      eventType = 'message';
      return;
    }
    const payload = dataLines.join('\n');
    dataLines = [];
    onMessage(payload, eventId, eventType);
    eventId = null;
    eventType = 'message';
  };

  while (!signal.aborted) {
    const { value, done } = await reader.read();
    if (done) {
      break;
    }

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split(/\r?\n/);
    buffer = lines.pop() ?? '';

    for (const line of lines) {
      if (line === '') {
        flushEvent();
        continue;
      }
      if (line.startsWith(':')) {
        continue;
      }
      if (line.startsWith('id:')) {
        eventId = line.slice(3).trimStart();
        continue;
      }
      if (line.startsWith('event:')) {
        const nextEventType = line.slice(6).trimStart();
        eventType = nextEventType || 'message';
        continue;
      }
      if (line.startsWith('data:')) {
        dataLines.push(line.slice(5).trimStart());
      }
    }
  }

  buffer += decoder.decode();
  if (buffer) {
    const lines = buffer.split(/\r?\n/);
    for (const line of lines) {
      if (line.startsWith('id:')) {
        eventId = line.slice(3).trimStart();
        continue;
      }
      if (line.startsWith('event:')) {
        const nextEventType = line.slice(6).trimStart();
        eventType = nextEventType || 'message';
        continue;
      }
      if (line.startsWith('data:')) {
        dataLines.push(line.slice(5).trimStart());
      }
    }
  }
  flushEvent();
  return signal.aborted ? 'aborted' : 'ended';
}
