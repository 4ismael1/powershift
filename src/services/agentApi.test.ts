import { describe, expect, it, vi } from 'vitest';
import {
  AGENT_STATE_STALE_MS,
  agentStateSignature,
  agentTaskInstalled,
  agentStateTone,
  applyAgentScanToGames,
  applyAgentStateToGames,
  describeAgentScan,
  describeAgentState,
  getAgentState,
  installAgentTask,
  isAgentStateStale,
  promoteActiveProfile,
  startAgentTask,
  wakeAgent,
} from './agentApi';
import type { UiGameProfile } from './configApi';
import type { InvokeFn } from './powerApi';

describe('agentApi', () => {
  function game(id: string, enabled = true): UiGameProfile {
    return {
      id,
      name: id,
      exe: `${id}.exe`,
      path: `C:\\Games\\${id}\\${id}.exe`,
      iconText: id.slice(0, 2).toUpperCase(),
      iconClass: 'custom',
      level: 'high',
      status: enabled ? 'inactive' : 'disabled',
      enabled,
      notify: true,
      startPlan: 'high',
      closePlan: 'Restaurar plan anterior',
      closeDelay: '30 s',
      associatedProcesses: [],
      lastEvent: enabled ? 'Inactivo' : 'Deshabilitado',
    };
  }

  function activeProfile(id: string, planId = 'high', priority = 70) {
    return {
      profile_id: id,
      profile_name: id,
      plan_id: planId,
      priority,
      matched_processes: [`${id}.exe`],
    };
  }

  it('calls Tauri commands for agent supervision', async () => {
    const state = {
      pid: 10,
      status: 'running' as const,
      updated_at_ms: 1,
      matched_profile_id: 'demo',
    };
    const mockInvoke = vi.fn().mockImplementation((command: string) => {
      if (command === 'get_agent_state') return Promise.resolve(state);
      if (command === 'agent_task_installed') return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    const invokeFn = mockInvoke as unknown as InvokeFn;

    await expect(getAgentState(invokeFn)).resolves.toEqual(state);
    await expect(wakeAgent(invokeFn)).resolves.toBeUndefined();
    await expect(installAgentTask(invokeFn)).resolves.toBeUndefined();
    await expect(promoteActiveProfile(invokeFn, 'chrome')).resolves.toBeUndefined();
    await expect(startAgentTask(invokeFn)).resolves.toBeUndefined();
    await expect(agentTaskInstalled(invokeFn)).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith('get_agent_state');
    expect(mockInvoke).toHaveBeenCalledWith('wake_agent');
    expect(mockInvoke).toHaveBeenCalledWith('install_agent_task');
    expect(mockInvoke).toHaveBeenCalledWith('promote_active_profile', { profile_id: 'chrome' });
    expect(mockInvoke).toHaveBeenCalledWith('start_agent_task');
    expect(mockInvoke).toHaveBeenCalledWith('agent_task_installed');
  });

  it('marks the matched profile active and keeps the rest inactive', () => {
    const games = [game('chrome'), game('game')];

    const result = applyAgentScanToGames(games, {
      matched_profile_id: 'game',
      matched_profile_name: 'Game',
      target_plan_id: 'high',
      changed_power_plan: true,
      restore_scheduled: false,
      restored_power_plan: false,
    });

    expect(result.map((item) => item.status)).toEqual(['inactive', 'active']);
    expect(result[1].lastEvent).toBe('Plan aplicado');
    expect(games[1].status).toBe('inactive');
  });

  it('marks every active profile while only the winning profile controls the plan', () => {
    const games = [game('chrome'), game('game'), game('node')];

    const result = applyAgentScanToGames(games, {
      matched_profile_id: 'game',
      matched_profile_name: 'Game',
      target_plan_id: 'high',
      active_profiles: [activeProfile('chrome', 'balanced', 20), activeProfile('game', 'high', 90)],
      changed_power_plan: true,
      restore_scheduled: false,
      restored_power_plan: false,
    });

    expect(result.map((item) => item.status)).toEqual(['active', 'active', 'inactive']);
    expect(result[0].lastEvent).toBe('Activo');
    expect(result[1].lastEvent).toBe('Plan aplicado');
  });

  it('clears active state when no profile matches and preserves disabled profiles', () => {
    const games = [game('chrome'), { ...game('game'), status: 'active' as const }, game('disabled', false)];

    const result = applyAgentScanToGames(games, {
      matched_profile_id: null,
      matched_profile_name: null,
      target_plan_id: null,
      changed_power_plan: false,
      restore_scheduled: true,
      restored_power_plan: false,
    });

    expect(result.map((item) => item.status)).toEqual(['inactive', 'inactive', 'disabled']);
  });

  it('applies the last published agent state to games', () => {
    const games = [game('chrome'), game('game')];
    const now = 200_000;

    const result = applyAgentStateToGames(games, {
      pid: 42,
      status: 'running',
      updated_at_ms: now,
      last_error: null,
      last_scan: {
        matched_profile_id: 'chrome',
        matched_profile_name: 'Chrome',
        target_plan_id: 'high',
        changed_power_plan: false,
        restore_scheduled: false,
        restored_power_plan: false,
      },
    }, now);

    expect(result.map((item) => item.status)).toEqual(['active', 'inactive']);
  });

  it('keeps remaining active profiles active after a configured profile is removed locally', () => {
    const gamesAfterDelete = [game('chrome'), game('node')];
    const now = 200_000;

    const result = applyAgentStateToGames(gamesAfterDelete, {
      pid: 42,
      status: 'running',
      updated_at_ms: now,
      last_error: null,
      last_scan: {
        matched_profile_id: 'game',
        matched_profile_name: 'Game',
        target_plan_id: 'high',
        active_profiles: [activeProfile('chrome', 'balanced', 20), activeProfile('game', 'high', 90)],
        changed_power_plan: false,
        restore_scheduled: false,
        restored_power_plan: false,
      },
    }, now);

    expect(result.map((item) => item.status)).toEqual(['active', 'inactive']);
    expect(result[0].lastEvent).toBe('Activo');
  });

  it('treats stale agent state as pending instead of showing old active games', () => {
    const games = [game('chrome'), game('game')];
    const now = 500_000;

    const result = applyAgentStateToGames(games, {
      pid: 42,
      status: 'running',
      updated_at_ms: now - AGENT_STATE_STALE_MS - 1,
      last_error: null,
      last_scan: {
        matched_profile_id: 'chrome',
        matched_profile_name: 'Chrome',
        target_plan_id: 'high',
        changed_power_plan: false,
        restore_scheduled: false,
        restored_power_plan: false,
      },
    }, now);

    expect(result.map((item) => item.status)).toEqual(['inactive', 'inactive']);
    expect(result[0].lastEvent).toBe('Agente pendiente');
  });

  it('describes scan and agent state for the status surface', () => {
    const scan = {
      matched_profile_id: 'chrome',
      matched_profile_name: 'Chrome',
      target_plan_id: 'high',
      changed_power_plan: false,
      restore_scheduled: false,
      restored_power_plan: false,
    };

    expect(describeAgentScan(scan)).toBe('Chrome activo');
    expect(describeAgentState(null, true, 100)).toBe('Agente elevado pendiente de iniciar');
    expect(describeAgentState({
      pid: 42,
      status: 'running',
      updated_at_ms: 100,
      last_error: null,
      last_scan: scan,
    }, true, 100)).toBe('Chrome activo');
  });

  it('describes the winning profile when multiple profiles are active', () => {
    expect(describeAgentScan({
      matched_profile_id: 'game',
      matched_profile_name: 'Game',
      target_plan_id: 'high',
      active_profiles: [activeProfile('chrome', 'balanced', 20), activeProfile('game', 'high', 90)],
      changed_power_plan: false,
      restore_scheduled: false,
      restored_power_plan: false,
    })).toBe('Game controla (2 activos)');
  });

  it('describes scheduled and completed restores with profile identity', () => {
    expect(describeAgentScan({
      matched_profile_id: null,
      matched_profile_name: null,
      target_plan_id: null,
      restore_profile_name: 'Chrome',
      changed_power_plan: false,
      restore_scheduled: true,
      restored_power_plan: false,
    })).toBe('Restauración de Chrome programada');

    expect(describeAgentScan({
      matched_profile_id: null,
      matched_profile_name: null,
      target_plan_id: 'balanced',
      restore_profile_name: 'Chrome',
      changed_power_plan: true,
      restore_scheduled: false,
      restored_power_plan: true,
    })).toBe('Plan restaurado para Chrome');
  });

  it('detects stale state and exposes a status tone', () => {
    const state = {
      pid: 42,
      status: 'running' as const,
      updated_at_ms: 100,
      last_error: null,
      last_scan: null,
    };

    expect(isAgentStateStale(state, 100 + AGENT_STATE_STALE_MS)).toBe(false);
    expect(isAgentStateStale(state, 101 + AGENT_STATE_STALE_MS)).toBe(true);
    expect(isAgentStateStale({ ...state, process_alive: true }, 101 + AGENT_STATE_STALE_MS)).toBe(true);
    expect(isAgentStateStale({ ...state, process_alive: true, ipc_connected: true }, 101 + AGENT_STATE_STALE_MS)).toBe(false);
    expect(isAgentStateStale({ ...state, process_alive: false }, 100)).toBe(true);
    expect(describeAgentState(state, true, 101 + AGENT_STATE_STALE_MS)).toBe('Agente sin respuesta reciente');
    expect(describeAgentState({ ...state, process_alive: true, ipc_connected: true }, true, 101 + AGENT_STATE_STALE_MS)).toBe('Agente elevado activo');
    expect(agentStateTone(state, true, 101 + AGENT_STATE_STALE_MS)).toBe('warning');
    expect(agentStateTone({ ...state, process_alive: true, ipc_connected: true }, true, 101 + AGENT_STATE_STALE_MS)).toBe('ready');
    expect(agentStateTone({ ...state, status: 'error', updated_at_ms: 200, last_error: 'fallo' }, true, 200)).toBe('error');
    expect(agentStateTone({ ...state, status: 'running', updated_at_ms: 200 }, true, 200)).toBe('ready');
  });

  it('surfaces degraded WMI watchers even while the agent is alive', () => {
    const state = {
      pid: 42,
      status: 'running' as const,
      updated_at_ms: 200,
      process_alive: true,
      last_error: null,
      last_scan: null,
      wmi_watchers: {
        starts: { state: 'running' as const, last_transition_ms: 100 },
        stops: {
          state: 'degraded' as const,
          last_transition_ms: 150,
          retry_in_ms: 1000,
          last_error: 'WMI unavailable',
        },
      },
    };

    expect(describeAgentState(state, true, 200)).toContain('WMI degradados');
    expect(agentStateTone(state, true, 200)).toBe('warning');
  });

  it('creates a stable signature for unchanged agent snapshots', () => {
    const state = {
      pid: 42,
      status: 'running' as const,
      updated_at_ms: 1000,
      last_error: null,
      last_scan: {
        matched_profile_id: 'chrome',
        matched_profile_name: 'Chrome',
        target_plan_id: 'high',
        active_profiles: [activeProfile('chrome', 'high', 70)],
        changed_power_plan: false,
        restore_scheduled: false,
        restored_power_plan: false,
      },
    };
    const same = { ...state, last_scan: { ...state.last_scan } };
    const changed = {
      ...state,
      last_scan: {
        ...state.last_scan,
        matched_profile_id: 'apex',
        matched_profile_name: 'Apex',
      },
    };
    const changedActiveProfiles = {
      ...state,
      last_scan: {
        ...state.last_scan,
        active_profiles: [activeProfile('chrome', 'high', 70), activeProfile('node', 'balanced', 20)],
      },
    };
    const heartbeatOnly = {
      ...state,
      updated_at_ms: 30_000,
    };

    expect(agentStateSignature(state)).toBe(agentStateSignature(same));
    expect(agentStateSignature(state)).toBe(agentStateSignature(heartbeatOnly));
    expect(agentStateSignature(state)).not.toBe(agentStateSignature(changed));
    expect(agentStateSignature(state)).not.toBe(agentStateSignature(changedActiveProfiles));
    expect(agentStateSignature(null)).toBe('none');
  });
});
