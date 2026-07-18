// @vitest-environment happy-dom

import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AppConfig } from '@/services/configApi';

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn().mockResolvedValue(vi.fn()),
  getVersion: vi.fn().mockResolvedValue('1.1.0'),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: tauri.listen }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: tauri.getVersion }));

import App from './App.vue';

function profile(id: string, name: string, exe: string, plan: string, priority: number) {
  return {
    id,
    name,
    enabled: true,
    main_executable: { name: exe, path: `C:\\Games\\${name}\\${exe}` },
    associated_processes: [],
    activation: { match_mode: 'path_or_name' as const, require_main_process: true },
    power: {
      on_start_plan_id: plan,
      on_close_behavior: 'previous_plan' as const,
      on_close_plan_id: null,
      close_delay_seconds: 30,
      priority,
    },
    notifications: { on_activate: true, on_restore: true, on_error: true },
    ui: { icon_cache_key: null, accent: null },
  };
}

function config(): AppConfig {
  return {
    version: 5,
    agent: {
      enabled: true,
      start_with_windows: true,
      start_minimized: true,
      show_tray_icon: true,
    },
    automation: {
      enabled: true,
      notifications_enabled: true,
      default_close_delay_seconds: 30,
    },
    ui: { close_button_behavior: 'hide_window' },
    profiles: [
      profile('fortnite', 'Fortnite', 'fortnite.exe', 'high', 70),
      profile('chrome', 'Chrome', 'chrome.exe', 'balanced', 30),
    ],
  };
}

function agentState(winner: 'fortnite' | 'chrome') {
  return {
    pid: 42,
    status: 'running',
    updated_at_ms: Date.now(),
    process_alive: true,
    ipc_connected: true,
    last_error: null,
    last_scan: {
      matched_profile_id: winner,
      matched_profile_name: winner === 'fortnite' ? 'Fortnite' : 'Chrome',
      target_plan_id: winner === 'fortnite' ? 'high' : 'balanced',
      active_profiles: [
        { profile_id: 'fortnite', profile_name: 'Fortnite', plan_id: 'high', priority: 70, matched_processes: ['fortnite.exe'] },
        { profile_id: 'chrome', profile_name: 'Chrome', plan_id: 'balanced', priority: 30, matched_processes: ['chrome.exe'] },
      ],
      changed_power_plan: false,
      restore_scheduled: false,
      restored_power_plan: false,
    },
  };
}

afterEach(() => {
  tauri.invoke.mockReset();
});

describe('App control handoff integration', () => {
  it('renders two active profiles and sends the selected handoff through Tauri', async () => {
    let winner: 'fortnite' | 'chrome' = 'fortnite';
    tauri.invoke.mockImplementation((command: string) => {
      if (command === 'get_app_config') return Promise.resolve(config());
      if (command === 'take_config_recovery_warning') return Promise.resolve(null);
      if (command === 'get_power_plans') return Promise.resolve([
        { id: 'high', name: 'Alto rendimiento' },
        { id: 'balanced', name: 'Equilibrado' },
      ]);
      if (command === 'get_active_power_plan') return Promise.resolve({ id: 'high', name: 'Alto rendimiento' });
      if (command === 'get_recent_events') return Promise.resolve([]);
      if (command === 'agent_task_installed') return Promise.resolve(true);
      if (command === 'get_agent_state') return Promise.resolve(agentState(winner));
      if (command === 'get_executable_icon') return Promise.resolve(null);
      if (command === 'promote_active_profile') {
        winner = 'chrome';
        return Promise.resolve();
      }
      return Promise.resolve();
    });

    const wrapper = mount(App);
    await vi.waitFor(() => expect(wrapper.findAll('.game-select')).toHaveLength(2));
    await wrapper.findAll('.game-select')[1].trigger('click');
    await flushPromises();

    const handoff = wrapper.get('button[title*="traspaso dura"]');
    await handoff.trigger('click');
    await vi.waitFor(() => {
      expect(tauri.invoke).toHaveBeenCalledWith('promote_active_profile', {
        profile_id: 'chrome',
      });
    });
    await vi.waitFor(() => expect(wrapper.text()).toContain('Traspaso solicitado para Chrome'));
    await vi.waitFor(() =>
      expect(wrapper.find('button[title*="traspaso dura"]').exists()).toBe(false),
    );

    wrapper.unmount();
  });
});
