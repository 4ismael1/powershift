import { describe, expect, it, vi } from 'vitest';
import {
  addProfileToConfig,
  addAssociatedProcessToProfile,
  createProfileFromExecutable,
  getAppConfig,
  normalizeProfilePriorities,
  planToLevel,
  profilesToUiGames,
  RESTORE_NOTHING_OPTION,
  RESTORE_PREVIOUS_OPTION,
  removeAssociatedProcessFromProfile,
  removeProfileFromConfig,
  saveAppConfig,
  takeConfigRecoveryWarning,
  updateAppSettingsConfig,
  updateAssociatedProcessRole,
  updateProfileConfig,
  type AppConfig,
} from './configApi';
import type { InvokeFn } from './powerApi';

function configWithProfiles(): AppConfig {
  return {
    version: 1,
    agent: {
      enabled: true,
      start_with_windows: false,
      start_minimized: true,
      show_tray_icon: true,
    },
    automation: {
      enabled: true,
      notifications_enabled: true,
      default_close_delay_seconds: 30,
    },
    ui: {
      close_button_behavior: 'hide_window',
    },
    profiles: [
      {
        id: 'notepad',
        name: 'Notepad Test',
        enabled: true,
        main_executable: { name: 'notepad.exe', path: 'C:\\Windows\\notepad.exe' },
        associated_processes: [{ name: 'helper.exe', path: null, match_mode: 'name' }],
        activation: { match_mode: 'path_or_name', require_main_process: true },
        power: {
          on_start_plan_id: 'high',
          on_close_behavior: 'specific_plan',
          on_close_plan_id: 'balanced',
          close_delay_seconds: 45,
          priority: 90,
        },
        notifications: { on_activate: true, on_restore: false, on_error: true },
        ui: { icon_cache_key: null, accent: null },
      },
    ],
  };
}

describe('configApi', () => {
  it('calls Tauri command to load app config', async () => {
    const config = configWithProfiles();
    const mockInvoke = vi.fn().mockResolvedValue(config);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    const result = await getAppConfig(invokeFn);

    expect(mockInvoke).toHaveBeenCalledWith('get_app_config');
    expect(result).toEqual(config);
  });

  it('calls Tauri command to save app config', async () => {
    const config = configWithProfiles();
    const mockInvoke = vi.fn().mockResolvedValue({ warnings: [] });
    const invokeFn = mockInvoke as unknown as InvokeFn;

    const result = await saveAppConfig(invokeFn, config);

    expect(mockInvoke).toHaveBeenCalledWith('save_app_config', { config });
    expect(result).toEqual({ warnings: [] });
  });

  it('consumes a one-time config recovery warning', async () => {
    const mockInvoke = vi.fn().mockResolvedValue('Configuración recuperada.');
    const invokeFn = mockInvoke as unknown as InvokeFn;

    const result = await takeConfigRecoveryWarning(invokeFn);

    expect(mockInvoke).toHaveBeenCalledWith('take_config_recovery_warning');
    expect(result).toBe('Configuración recuperada.');
  });

  it('maps persisted profiles to UI games', () => {
    const games = profilesToUiGames(configWithProfiles(), [{ id: 'high', name: 'Alto rendimiento' }]);

    expect(games).toEqual([
      {
        id: 'notepad',
        name: 'Notepad Test',
        exe: 'notepad.exe',
        path: 'C:\\Windows\\notepad.exe',
        iconText: 'NT',
        iconClass: 'custom',
        level: 'high',
        status: 'inactive',
        enabled: true,
        notify: true,
        startPlan: 'high',
        closePlan: 'balanced',
        closeDelay: '45 s',
        associatedProcesses: [{ name: 'helper.exe', role: 'companion' }],
        lastEvent: 'Inactivo',
      },
    ]);
  });

  it('maps disabled profiles and previous-plan restore state', () => {
    const config = configWithProfiles();
    config.profiles[0].enabled = false;
    config.profiles[0].power.priority = 30;
    config.profiles[0].power.on_start_plan_id = 'balanced';
    config.profiles[0].power.on_close_behavior = 'previous_plan';
    config.profiles[0].power.on_close_plan_id = null;

    const [game] = profilesToUiGames(config, [{ id: 'balanced', name: 'Equilibrado' }]);

    expect(game.status).toBe('disabled');
    expect(game.level).toBe('balanced');
    expect(game.closePlan).toBe(RESTORE_PREVIOUS_OPTION);
    expect(game.lastEvent).toBe('Deshabilitado');
  });

  it('preserves do-nothing restore behavior in the UI model', () => {
    const config = configWithProfiles();
    config.profiles[0].power.on_close_behavior = 'do_nothing';
    config.profiles[0].power.on_close_plan_id = null;

    const [game] = profilesToUiGames(config);
    const nextConfig = updateProfileConfig(config, 'notepad', {
      closePlan: RESTORE_NOTHING_OPTION,
    });

    expect(game.closePlan).toBe(RESTORE_NOTHING_OPTION);
    expect(nextConfig.profiles[0].power.on_close_behavior).toBe('do_nothing');
    expect(nextConfig.profiles[0].power.on_close_plan_id).toBeNull();
  });

  it('uses the selected start power plan for the level badge', () => {
    expect(planToLevel('balanced-id', [{ id: 'balanced-id', name: 'Equilibrado' }], 90)).toBe('balanced');
    expect(planToLevel('high-id', [{ id: 'high-id', name: 'Alto rendimiento' }], 30)).toBe('high');
    expect(planToLevel('ultimate-id', [{ id: 'ultimate-id', name: 'Máximo rendimiento' }], 30)).toBe('max');
  });

  it('normalizes persisted priorities from selected power plans', () => {
    const config = configWithProfiles();
    config.profiles = [
      {
        ...config.profiles[0],
        id: 'chrome',
        power: { ...config.profiles[0].power, on_start_plan_id: 'balanced-id', priority: 70 },
      },
      {
        ...config.profiles[0],
        id: 'node',
        power: { ...config.profiles[0].power, on_start_plan_id: 'ultimate-id', priority: 70 },
      },
    ];

    const normalized = normalizeProfilePriorities(config, [
      { id: 'balanced-id', name: 'Equilibrado' },
      { id: 'ultimate-id', name: 'Máximo rendimiento' },
    ]);

    expect(normalized.profiles.map((profile) => [profile.id, profile.power.priority])).toEqual([
      ['chrome', 30],
      ['node', 90],
    ]);
  });

  it('preserves an explicit priority when a custom plan cannot be classified', () => {
    const config = configWithProfiles();
    config.profiles[0].power.on_start_plan_id = 'custom-guid';
    config.profiles[0].power.priority = 42;

    const normalized = normalizeProfilePriorities(config, [
      { id: 'custom-guid', name: 'Mi plan personalizado' },
    ]);

    expect(normalized.profiles[0].power.priority).toBe(42);
    expect(planToLevel('custom-guid', [{ id: 'custom-guid', name: 'Mi plan personalizado' }], 42)).toBe(
      'balanced',
    );
  });

  it('creates a persisted profile from an executable path', () => {
    const profile = createProfileFromExecutable('C:\\Games\\my-game\\my_game.exe', {
      defaultDelaySeconds: 45,
      existingIds: [],
      powerPlans: [
        { id: 'balanced-id', name: 'Equilibrado' },
        { id: 'high-id', name: 'Alto rendimiento' },
      ],
    });

    expect(profile).toMatchObject({
      id: 'my-game',
      name: 'My Game',
      enabled: true,
      main_executable: { name: 'my_game.exe', path: 'C:\\Games\\my-game\\my_game.exe' },
      activation: { match_mode: 'path_or_name', require_main_process: true },
      power: {
        on_start_plan_id: 'high-id',
        on_close_behavior: 'previous_plan',
        on_close_plan_id: null,
        close_delay_seconds: 45,
        priority: 70,
      },
    });
  });

  it('stores max priority when creating a profile with a max performance plan', () => {
    const profile = createProfileFromExecutable('C:\\Games\\Max\\max.exe', {
      powerPlans: [{ id: 'ultimate-id', name: 'Máximo rendimiento' }],
    });

    expect(profile.power.on_start_plan_id).toBe('ultimate-id');
    expect(profile.power.priority).toBe(90);
  });

  it('creates a unique id when the executable profile already exists', () => {
    const profile = createProfileFromExecutable('C:\\Games\\Demo\\demo.exe', {
      existingIds: ['demo', 'demo-2'],
      powerPlans: [{ id: 'high-id', name: 'High performance' }],
    });

    expect(profile.id).toBe('demo-3');
  });

  it('rejects non executable paths', () => {
    expect(() =>
      createProfileFromExecutable('C:\\Games\\Demo\\readme.txt', {
        powerPlans: [{ id: 'high-id', name: 'Alto rendimiento' }],
      }),
    ).toThrow('Selecciona un ejecutable .exe');
  });

  it('rejects profile creation when no power plans are available', () => {
    expect(() => createProfileFromExecutable('C:\\Games\\Demo\\demo.exe')).toThrow(
      'No hay planes de energia disponibles',
    );
  });

  it('adds a profile to config without mutating the previous object', () => {
    const config = configWithProfiles();

    const nextConfig = addProfileToConfig(config, 'C:\\Games\\Demo\\demo.exe', [{ id: 'high-id', name: 'High performance' }]);

    expect(config.profiles).toHaveLength(1);
    expect(nextConfig.profiles).toHaveLength(2);
    expect(nextConfig.profiles[1].id).toBe('demo');
  });

  it('removes a profile by id without mutating the previous object', () => {
    const config = addProfileToConfig(configWithProfiles(), 'C:\\Games\\Demo\\demo.exe', [
      { id: 'high-id', name: 'High performance' },
    ]);

    const nextConfig = removeProfileFromConfig(config, 'notepad');

    expect(config.profiles).toHaveLength(2);
    expect(nextConfig.profiles).toHaveLength(1);
    expect(nextConfig.profiles[0].id).toBe('demo');
  });

  it('updates editable profile fields without mutating the original config', () => {
    const config = configWithProfiles();

    const nextConfig = updateProfileConfig(
      config,
      'notepad',
      {
        name: 'Nuevo Nombre',
        executablePath: 'D:\\Games\\Nuevo\\nuevo.exe',
        enabled: false,
        notify: false,
        startPlan: 'ultimate',
        closePlan: 'balanced',
        closeDelay: '60 s',
      },
      [{ id: 'ultimate', name: 'Máximo rendimiento' }],
    );

    expect(config.profiles[0].name).toBe('Notepad Test');
    expect(nextConfig.profiles[0]).toMatchObject({
      name: 'Nuevo Nombre',
      enabled: false,
      main_executable: { name: 'nuevo.exe', path: 'D:\\Games\\Nuevo\\nuevo.exe' },
      power: {
        on_start_plan_id: 'ultimate',
        on_close_behavior: 'specific_plan',
        on_close_plan_id: 'balanced',
        close_delay_seconds: 60,
        priority: 90,
      },
      notifications: { on_activate: false, on_restore: false, on_error: true },
    });
  });

  it('updates close behavior back to previous plan', () => {
    const config = configWithProfiles();

    const nextConfig = updateProfileConfig(config, 'notepad', {
      closePlan: RESTORE_PREVIOUS_OPTION,
    });

    expect(nextConfig.profiles[0].power.on_close_behavior).toBe('previous_plan');
    expect(nextConfig.profiles[0].power.on_close_plan_id).toBeNull();
  });

  it('keeps the previous name when an empty name is submitted', () => {
    const config = configWithProfiles();

    const nextConfig = updateProfileConfig(config, 'notepad', { name: '   ' });

    expect(nextConfig.profiles[0].name).toBe('Notepad Test');
  });

  it('adds an associated process to a profile', () => {
    const config = configWithProfiles();

    const nextConfig = addAssociatedProcessToProfile(config, 'notepad', {
      name: 'overlay.exe',
      path: 'C:\\Tools\\overlay.exe',
    });

    expect(nextConfig.profiles[0].associated_processes).toContainEqual({
      name: 'overlay.exe',
      path: 'C:\\Tools\\overlay.exe',
      match_mode: 'path_or_name',
      role: 'companion',
    });
    expect(config.profiles[0].associated_processes).toHaveLength(1);
  });

  it('changes an associated process between companion and alternate trigger', () => {
    const config = configWithProfiles();

    const alternate = updateAssociatedProcessRole(
      config,
      'notepad',
      'helper.exe',
      'alternate_trigger',
    );
    const unchanged = updateAssociatedProcessRole(
      alternate,
      'notepad',
      'helper.exe',
      'alternate_trigger',
    );

    expect(alternate.profiles[0].associated_processes[0].role).toBe('alternate_trigger');
    expect(config.profiles[0].associated_processes[0].role).toBeUndefined();
    expect(unchanged).toBe(alternate);
  });

  it('does not duplicate associated processes or add the main executable as associated', () => {
    const config = configWithProfiles();

    const withDuplicate = addAssociatedProcessToProfile(config, 'notepad', {
      name: 'helper.exe',
      path: null,
    });
    const withMain = addAssociatedProcessToProfile(config, 'notepad', {
      name: 'notepad.exe',
      path: null,
    });

    expect(withDuplicate.profiles[0].associated_processes).toHaveLength(1);
    expect(withMain.profiles[0].associated_processes).toHaveLength(1);
    expect(withDuplicate).toBe(config);
    expect(withMain).toBe(config);
  });

  it('removes an associated process without removing the main executable', () => {
    const config = configWithProfiles();

    const withoutHelper = removeAssociatedProcessFromProfile(config, 'notepad', 'helper.exe');
    const withoutMain = removeAssociatedProcessFromProfile(config, 'notepad', 'notepad.exe');

    expect(config.profiles[0].associated_processes).toHaveLength(1);
    expect(withoutHelper.profiles[0].associated_processes).toHaveLength(0);
    expect(withoutMain.profiles[0].associated_processes).toHaveLength(1);
    expect(withoutMain).toBe(config);
  });

  it('updates general app settings without mutating the original config', () => {
    const config = configWithProfiles();

    const nextConfig = updateAppSettingsConfig(config, {
      agentEnabled: false,
      automationEnabled: false,
      notificationsEnabled: false,
      startWithWindows: true,
      startMinimized: false,
      showTrayIcon: false,
      closeButtonBehavior: 'exit_app',
    });

    expect(config.automation.enabled).toBe(true);
    expect(nextConfig.agent.enabled).toBe(false);
    expect(nextConfig.automation.enabled).toBe(false);
    expect(nextConfig.automation.notifications_enabled).toBe(false);
    expect(nextConfig.agent.start_with_windows).toBe(true);
    expect(nextConfig.agent.start_minimized).toBe(false);
    expect(nextConfig.agent.show_tray_icon).toBe(false);
    expect(nextConfig.ui.close_button_behavior).toBe('exit_app');
  });

  it('can pause automation without disabling the resident agent', () => {
    const config = configWithProfiles();

    const nextConfig = updateAppSettingsConfig(config, {
      automationEnabled: false,
    });

    expect(nextConfig.automation.enabled).toBe(false);
    expect(nextConfig.agent.enabled).toBe(true);
  });

  it('uses global notification preference as the default for new profiles', () => {
    const config = configWithProfiles();
    config.automation.notifications_enabled = false;

    const nextConfig = addProfileToConfig(config, 'C:\\Games\\Quiet\\quiet.exe', [
      { id: 'high-id', name: 'High performance' },
    ]);

    expect(nextConfig.profiles[1].notifications).toMatchObject({
      on_activate: false,
      on_restore: false,
      on_error: true,
    });
  });
});
