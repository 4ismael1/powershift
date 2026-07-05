import { describe, expect, it } from 'vitest';
import { detectProfileCandidates } from './autoDetect';
import type { AppConfig } from './configApi';
import type { ProcessInfo } from './processApi';

function emptyConfig(): AppConfig {
  return {
    version: 1,
    agent: {
      enabled: true,
      start_with_windows: false,
      start_minimized: true,
      show_tray_icon: true,
      single_instance: true,
    },
    automation: {
      enabled: true,
      notifications_enabled: true,
      default_restore_behavior: 'previous_plan',
      conflict_strategy: 'highest_priority',
      respect_manual_plan_changes: false,
      default_close_delay_seconds: 30,
    },
    ui: {
      theme: 'dark',
      language: 'es',
      close_button_behavior: 'hide_window',
      compact_mode: true,
    },
    profiles: [],
  };
}

describe('autoDetect', () => {
  it('detects likely game candidates from open executable processes', () => {
    const processes: ProcessInfo[] = [
      { pid: 10, name: 'explorer.exe', path: 'C:\\Windows\\explorer.exe' },
      { pid: 20, name: 'eldenring.exe', path: 'D:\\SteamLibrary\\steamapps\\common\\ELDEN RING\\eldenring.exe' },
    ];

    const candidates = detectProfileCandidates(emptyConfig(), processes);

    expect(candidates).toHaveLength(1);
    expect(candidates[0]).toMatchObject({
      executableName: 'eldenring.exe',
      executablePath: 'D:\\SteamLibrary\\steamapps\\common\\ELDEN RING\\eldenring.exe',
      reason: 'Ruta de juego detectada',
      score: 100,
    });
  });

  it('ignores already configured executables by name or path', () => {
    const config = emptyConfig();
    config.profiles.push({
      id: 'demo',
      name: 'Demo',
      enabled: true,
      main_executable: { name: 'demo.exe', path: 'C:\\Games\\Demo\\demo.exe' },
      associated_processes: [],
      activation: { match_mode: 'path_or_name', require_main_process: true },
      power: {
        on_start_plan_id: 'high',
        on_close_behavior: 'previous_plan',
        on_close_plan_id: null,
        close_delay_seconds: 30,
        priority: 70,
      },
      notifications: { on_activate: true, on_restore: true, on_error: true },
      ui: { icon_cache_key: null, accent: null },
    });

    const candidates = detectProfileCandidates(config, [
      { pid: 20, name: 'demo.exe', path: 'C:\\Games\\Demo\\demo.exe' },
      { pid: 21, name: 'other.exe', path: 'C:\\Games\\Other\\other.exe' },
    ]);

    expect(candidates.map((candidate) => candidate.executableName)).toEqual(['other.exe']);
  });

  it('deduplicates candidates by executable path and sorts by score', () => {
    const candidates = detectProfileCandidates(emptyConfig(), [
      { pid: 30, name: 'tool.exe', path: 'C:\\Tools\\tool.exe' },
      { pid: 40, name: 'game.exe', path: 'C:\\Epic Games\\Game\\game.exe' },
      { pid: 41, name: 'game.exe', path: 'C:\\Epic Games\\Game\\game.exe' },
    ]);

    expect(candidates.map((candidate) => candidate.executableName)).toEqual(['game.exe', 'tool.exe']);
  });
});
