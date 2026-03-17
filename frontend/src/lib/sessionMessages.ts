import type { ToolStatus } from '../types';

export function snapshotToolStatus(ok?: boolean): ToolStatus {
  if (ok === undefined) {
    return 'running';
  }
  return ok ? 'ok' : 'fail';
}
