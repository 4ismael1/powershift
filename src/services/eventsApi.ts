import type { InvokeFn } from './powerApi';

export interface EventLogEntry {
  timestamp_ms: number;
  level: string;
  kind: string;
  message: string;
  profile_name?: string | null;
  plan_id?: string | null;
}

export async function getRecentEvents(invokeFn: InvokeFn, limit = 50): Promise<EventLogEntry[]> {
  return invokeFn<EventLogEntry[]>('get_recent_events', { limit });
}

export async function clearEvents(invokeFn: InvokeFn): Promise<void> {
  return invokeFn<void>('clear_events');
}

export function formatEventTime(timestampMs: number, dateFactory: typeof Date = Date): string {
  return new dateFactory(timestampMs).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  });
}
