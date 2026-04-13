import { ensureServerSession, getServerAuthToken, getServerOrigin } from './serverAuth';

type LogLevel = 'debug' | 'info' | 'warn' | 'error';
type LogScope = 'frontend' | 'model' | 'backend';
interface LogScopeHint {
  __logScope?: LogScope;
}

interface LogPayload {
  level: LogLevel;
  source: string;
  scope: LogScope;
  message: string;
  details?: unknown[];
}

const LOG_ENDPOINT = '/api/logs';
const DEFAULT_SOURCE = 'frontend';

function toLogText(value: unknown): string {
  if (typeof value === 'string') {
    return value;
  }
  if (value instanceof Error) {
    return `${value.name}: ${value.message}`;
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function splitMessageAndDetails(args: unknown[]): { message: string; details: unknown[] } {
  if (args.length === 0) {
    return { message: '', details: [] };
  }

  const first = args[0];
  if (typeof first === 'string') {
    return {
      message: first,
      details: args.slice(1),
    };
  }

  return {
    message: toLogText(first),
    details: args.slice(1),
  };
}

function sendToServer(payload: LogPayload): void {
  const request = async () => {
    let token = getServerAuthToken();
    if (!token) {
      try {
        await ensureServerSession();
      } catch {
        return;
      }
      token = getServerAuthToken();
      if (!token) {
        return;
      }
    }

    const body = JSON.stringify(payload);
    await fetch(`${getServerOrigin()}${LOG_ENDPOINT}`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'x-astrcode-token': token,
      },
      body,
      keepalive: true,
    });
  };
  void request().catch(() => undefined);
}

function log(level: LogLevel, source: string, ...args: unknown[]): void {
  let scope: LogScope = 'frontend';
  if (
    args.length > 0 &&
    typeof args[0] === 'object' &&
    args[0] !== null &&
    '__logScope' in args[0]
  ) {
    const options = args[0] as LogScopeHint;
    scope = options.__logScope ?? scope;
    args = args.slice(1);
  }

  const { message, details } = splitMessageAndDetails(args);
  const payload: LogPayload = {
    level,
    source,
    scope,
    message,
    details: details.length > 0 ? details : undefined,
  };

  sendToServer(payload);

  switch (level) {
    case 'debug':
      console.debug(message, ...details);
      return;
    case 'info':
      console.info(message, ...details);
      return;
    case 'warn':
      console.warn(message, ...details);
      return;
    case 'error':
      console.error(message, ...details);
      return;
  }
}

export const logger = {
  debug(source: string, ...args: unknown[]): void {
    log('debug', source, ...args);
  },
  info(source: string, ...args: unknown[]): void {
    log('info', source, ...args);
  },
  warn(source: string, ...args: unknown[]): void {
    log('warn', source, ...args);
  },
  error(source: string, ...args: unknown[]): void {
    log('error', source, ...args);
  },
  modelDebug(source: string, ...args: unknown[]): void {
    log('debug', source, { __logScope: 'model' }, ...args);
  },
  modelInfo(source: string, ...args: unknown[]): void {
    log('info', source, { __logScope: 'model' }, ...args);
  },
  modelWarn(source: string, ...args: unknown[]): void {
    log('warn', source, { __logScope: 'model' }, ...args);
  },
  modelError(source: string, ...args: unknown[]): void {
    log('error', source, { __logScope: 'model' }, ...args);
  },
  backendDebug(source: string, ...args: unknown[]): void {
    log('debug', source, { __logScope: 'backend' }, ...args);
  },
  backendInfo(source: string, ...args: unknown[]): void {
    log('info', source, { __logScope: 'backend' }, ...args);
  },
  backendWarn(source: string, ...args: unknown[]): void {
    log('warn', source, { __logScope: 'backend' }, ...args);
  },
  backendError(source: string, ...args: unknown[]): void {
    log('error', source, { __logScope: 'backend' }, ...args);
  },
  debugFallback(...args: unknown[]): void {
    logger.debug(DEFAULT_SOURCE, ...args);
  },
  infoFallback(...args: unknown[]): void {
    logger.info(DEFAULT_SOURCE, ...args);
  },
  warnFallback(...args: unknown[]): void {
    logger.warn(DEFAULT_SOURCE, ...args);
  },
  errorFallback(...args: unknown[]): void {
    logger.error(DEFAULT_SOURCE, ...args);
  },
};
