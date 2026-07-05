import type { InvokeFn } from './powerApi';

export interface ProcessInfo {
  pid: number;
  name: string;
  path?: string | null;
}

export async function getOpenProcesses(invokeFn: InvokeFn): Promise<ProcessInfo[]> {
  return invokeFn<ProcessInfo[]>('get_open_processes');
}

export function filterProcesses(processes: ProcessInfo[], query: string): ProcessInfo[] {
  const value = query.trim().toLowerCase();
  if (!value) return processes;

  return processes.filter((process) =>
    `${process.name} ${process.path ?? ''} ${process.pid}`.toLowerCase().includes(value),
  );
}
