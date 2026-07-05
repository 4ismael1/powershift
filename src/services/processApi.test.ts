import { describe, expect, it, vi } from 'vitest';
import { filterProcesses, getOpenProcesses } from './processApi';
import type { InvokeFn } from './powerApi';

describe('processApi', () => {
  const processes = [
    { pid: 10, name: 'explorer.exe', path: 'C:\\Windows\\explorer.exe' },
    { pid: 20, name: 'game.exe', path: 'C:\\Games\\game.exe' },
  ];

  it('calls Tauri command to list open processes', async () => {
    const mockInvoke = vi.fn().mockResolvedValue(processes);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    const result = await getOpenProcesses(invokeFn);

    expect(mockInvoke).toHaveBeenCalledWith('get_open_processes');
    expect(result).toEqual(processes);
  });

  it('filters by process name, path or pid', () => {
    expect(filterProcesses(processes, 'game')).toEqual([processes[1]]);
    expect(filterProcesses(processes, 'windows')).toEqual([processes[0]]);
    expect(filterProcesses(processes, '20')).toEqual([processes[1]]);
  });

  it('returns all processes for a blank query', () => {
    expect(filterProcesses(processes, '   ')).toEqual(processes);
  });
});
