import type { InvokeFn } from './powerApi';
import type { UiGameProfile } from './configApi';

export interface AgentScanResult {
  matched_profile_id?: string | null;
  matched_profile_name?: string | null;
  target_plan_id?: string | null;
  restore_profile_id?: string | null;
  restore_profile_name?: string | null;
  active_profiles?: AgentActiveProfile[];
  changed_power_plan: boolean;
  restore_scheduled: boolean;
  restored_power_plan: boolean;
}

export interface AgentActiveProfile {
  profile_id: string;
  profile_name: string;
  plan_id: string;
  priority: number;
  matched_processes: string[];
}

export interface PublishedAgentState {
  pid: number;
  status: 'starting' | 'running' | 'paused' | 'error' | 'stopped';
  updated_at_ms: number;
  last_scan?: AgentScanResult | null;
  last_error?: string | null;
  process_alive?: boolean | null;
  ipc_connected?: boolean | null;
  wmi_watchers?: WmiWatcherStatus;
}

export interface WmiWatcherChannelStatus {
  state: 'starting' | 'running' | 'degraded';
  last_transition_ms: number;
  retry_in_ms?: number | null;
  last_error?: string | null;
}

export interface WmiWatcherStatus {
  starts: WmiWatcherChannelStatus;
  stops: WmiWatcherChannelStatus;
}

export type AgentStateTone = 'ready' | 'idle' | 'warning' | 'error';

export const AGENT_STATE_STALE_MS = 90_000;

export async function getAgentState(invokeFn: InvokeFn): Promise<PublishedAgentState | null> {
  return invokeFn<PublishedAgentState | null>('get_agent_state');
}

export async function wakeAgent(invokeFn: InvokeFn): Promise<void> {
  return invokeFn<void>('wake_agent');
}

export async function installAgentTask(invokeFn: InvokeFn): Promise<void> {
  return invokeFn<void>('install_agent_task');
}

export async function startAgentTask(invokeFn: InvokeFn): Promise<void> {
  return invokeFn<void>('start_agent_task');
}

export async function agentTaskInstalled(invokeFn: InvokeFn): Promise<boolean> {
  return invokeFn<boolean>('agent_task_installed');
}

export function isAgentStateStale(state: PublishedAgentState | null, now = Date.now()): boolean {
  if (!state) return false;
  if (state.ipc_connected === true) return false;
  if (state.process_alive === false) return true;
  return now - state.updated_at_ms > AGENT_STATE_STALE_MS;
}

export function describeAgentScan(scan: AgentScanResult): string {
  const activeCount = scan.active_profiles?.length ?? (scan.matched_profile_id ? 1 : 0);
  if (scan.matched_profile_name) {
    return activeCount > 1 ? `${scan.matched_profile_name} controla (${activeCount} activos)` : `${scan.matched_profile_name} activo`;
  }
  if (activeCount > 0) return `${activeCount} perfiles activos`;
  if (scan.restore_scheduled) {
    return scan.restore_profile_name ? `Restauración de ${scan.restore_profile_name} programada` : 'Restauración programada';
  }
  if (scan.restored_power_plan) {
    return scan.restore_profile_name ? `Plan restaurado para ${scan.restore_profile_name}` : 'Plan restaurado';
  }
  return 'Sin perfiles activos';
}

export function describeAgentState(
  state: PublishedAgentState | null,
  taskReady: boolean,
  now = Date.now(),
): string {
  if (!state) {
    return taskReady ? 'Agente elevado pendiente de iniciar' : 'Instala el agente elevado para eventos WMI';
  }

  if (isAgentStateStale(state, now)) return 'Agente sin respuesta reciente';

  if (state.last_error) {
    return state.last_error;
  }

  if (wmiWatchersDegraded(state)) {
    return 'Agente activo; eventos WMI degradados, usando respaldo adaptativo';
  }

  if (state.last_scan) {
    return describeAgentScan(state.last_scan);
  }

  if (state.status === 'running') return 'Agente elevado activo';
  if (state.status === 'starting') return 'Agente iniciando';
  if (state.status === 'paused') return 'Agente pausado';
  if (state.status === 'stopped') return 'Agente detenido';
  return 'Agente con error';
}

export function agentStateTone(
  state: PublishedAgentState | null,
  taskReady: boolean,
  now = Date.now(),
): AgentStateTone {
  if (!state) return taskReady ? 'warning' : 'idle';
  if (isAgentStateStale(state, now)) return 'warning';
  if (state.last_error || state.status === 'error' || state.status === 'stopped') return 'error';
  if (state.status === 'starting' || state.status === 'paused' || wmiWatchersDegraded(state)) return 'warning';
  return 'ready';
}

export function agentStateSignature(state: PublishedAgentState | null): string {
  if (!state) return 'none';

  const scan = state.last_scan;
  const activeProfiles = (scan?.active_profiles ?? [])
    .map((profile) => `${profile.profile_id}:${profile.plan_id}:${profile.priority}`)
    .join(',');
  return [
    state.pid,
    state.process_alive === true ? 'alive' : state.process_alive === false ? 'dead' : 'unknown',
    state.ipc_connected === true ? 'ipc' : 'fallback',
    state.status,
    state.last_error ?? '',
    state.wmi_watchers?.starts.state ?? '',
    state.wmi_watchers?.stops.state ?? '',
    scan?.matched_profile_id ?? '',
    scan?.matched_profile_name ?? '',
    scan?.target_plan_id ?? '',
    scan?.restore_profile_id ?? '',
    scan?.restore_profile_name ?? '',
    activeProfiles,
    scan?.changed_power_plan ? '1' : '0',
    scan?.restore_scheduled ? '1' : '0',
    scan?.restored_power_plan ? '1' : '0',
  ].join('|');
}

function wmiWatchersDegraded(state: PublishedAgentState): boolean {
  return state.wmi_watchers?.starts.state === 'degraded' || state.wmi_watchers?.stops.state === 'degraded';
}

export function applyAgentScanToGames(games: UiGameProfile[], scan: AgentScanResult): UiGameProfile[] {
  const activeIds = new Set((scan.active_profiles ?? []).map((profile) => profile.profile_id));
  if (scan.matched_profile_id) {
    activeIds.add(scan.matched_profile_id);
  }
  const winnerId = scan.matched_profile_id ?? null;

  return games.map((game) => {
    if (!game.enabled) {
      return { ...game, status: 'disabled', lastEvent: 'Deshabilitado' };
    }

    if (activeIds.has(game.id)) {
      const controlsPowerPlan = game.id === winnerId;
      return {
        ...game,
        status: 'active',
        lastEvent: controlsPowerPlan ? (scan.changed_power_plan ? 'Plan aplicado' : 'Controlando') : 'Activo',
      };
    }

    return { ...game, status: 'inactive', lastEvent: 'Inactivo' };
  });
}

export function applyAgentStateToGames(
  games: UiGameProfile[],
  state: PublishedAgentState | null,
  now = Date.now(),
): UiGameProfile[] {
  if (!state?.last_scan || isAgentStateStale(state, now)) {
    return games.map((game) => (game.enabled ? { ...game, status: 'inactive', lastEvent: 'Agente pendiente' } : game));
  }

  return applyAgentScanToGames(games, state.last_scan);
}
