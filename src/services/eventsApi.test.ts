import { describe, expect, it, vi } from 'vitest';
import { clearEvents, formatEventTime, getRecentEvents } from './eventsApi';
import type { InvokeFn } from './powerApi';

describe('eventsApi', () => {
  it('calls Tauri command with a limit', async () => {
    const events = [
      {
        timestamp_ms: 1,
        level: 'info',
        kind: 'profile_activated',
        message: 'Demo activo',
        profile_name: 'Demo',
        plan_id: 'high',
      },
    ];
    const mockInvoke = vi.fn().mockResolvedValue(events);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    await expect(getRecentEvents(invokeFn, 25)).resolves.toEqual(events);

    expect(mockInvoke).toHaveBeenCalledWith('get_recent_events', { limit: 25 });
  });

  it('formats an event timestamp as local short time', () => {
    expect(formatEventTime(Date.UTC(2026, 0, 1, 13, 5))).toMatch(/\d{1,2}:\d{2}/);
  });

  it('calls Tauri command to clear event history', async () => {
    const mockInvoke = vi.fn().mockResolvedValue(undefined);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    await expect(clearEvents(invokeFn)).resolves.toBeUndefined();

    expect(mockInvoke).toHaveBeenCalledWith('clear_events');
  });
});
