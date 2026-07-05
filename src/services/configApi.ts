import type { InvokeFn, PowerPlan } from './powerApi';

export interface AppConfig {
  version: number;
  agent: AgentSettings;
  automation: AutomationSettings;
  ui: UiSettings;
  profiles: ProfileConfig[];
}

export interface AgentSettings {
  enabled: boolean;
  start_with_windows: boolean;
  start_minimized: boolean;
  show_tray_icon: boolean;
  single_instance: boolean;
}

export interface AutomationSettings {
  enabled: boolean;
  notifications_enabled: boolean;
  default_restore_behavior: string;
  conflict_strategy: string;
  respect_manual_plan_changes: boolean;
  default_close_delay_seconds: number;
}

export interface UiSettings {
  theme: string;
  language: string;
  close_button_behavior: string;
  compact_mode: boolean;
}

export interface ProfileConfig {
  id: string;
  name: string;
  enabled: boolean;
  main_executable: {
    name: string;
    path?: string | null;
  };
  associated_processes: Array<{
    name: string;
    path?: string | null;
    match_mode: string;
  }>;
  activation: {
    match_mode: string;
    require_main_process: boolean;
  };
  power: {
    on_start_plan_id: string;
    on_close_behavior: string;
    on_close_plan_id?: string | null;
    close_delay_seconds: number;
    priority: number;
  };
  notifications: {
    on_activate: boolean;
    on_restore: boolean;
    on_error: boolean;
  };
  ui: {
    icon_cache_key?: string | null;
    accent?: string | null;
  };
}

export interface UiGameProfile {
  id: string;
  name: string;
  exe: string;
  path: string;
  iconText: string;
  iconClass: string;
  level: 'max' | 'high' | 'balanced';
  status: 'active' | 'inactive' | 'disabled';
  enabled: boolean;
  notify: boolean;
  startPlan: string;
  closePlan: string;
  closeDelay: string;
  processes: string[];
  lastEvent: string;
}

export interface ProfileUpdate {
  name?: string;
  executablePath?: string;
  enabled?: boolean;
  notify?: boolean;
  startPlan?: string;
  closePlan?: string;
  closeDelay?: string;
}

export interface AssociatedProcessInput {
  name: string;
  path?: string | null;
}

export interface AppSettingsUpdate {
  agentEnabled?: boolean;
  automationEnabled?: boolean;
  startWithWindows?: boolean;
  startMinimized?: boolean;
  showTrayIcon?: boolean;
  closeButtonBehavior?: string;
  notificationsEnabled?: boolean;
}

export async function getAppConfig(invokeFn: InvokeFn): Promise<AppConfig> {
  return invokeFn<AppConfig>('get_app_config');
}

export async function saveAppConfig(invokeFn: InvokeFn, config: AppConfig): Promise<void> {
  return invokeFn<void>('save_app_config', { config });
}

export function profilesToUiGames(config: AppConfig, powerPlans: PowerPlan[] = []): UiGameProfile[] {
  return config.profiles.map((profile) => ({
    id: profile.id,
    name: profile.name,
    exe: profile.main_executable.name,
    path: profile.main_executable.path ?? '',
    iconText: initials(profile.name),
    iconClass: 'custom',
    level: planToLevel(profile.power.on_start_plan_id, powerPlans, profile.power.priority),
    status: profile.enabled ? 'inactive' : 'disabled',
    enabled: profile.enabled,
    notify: profile.notifications.on_activate || profile.notifications.on_restore,
    startPlan: profile.power.on_start_plan_id,
    closePlan:
      profile.power.on_close_behavior === 'specific_plan'
        ? profile.power.on_close_plan_id || ''
        : 'Restaurar plan anterior',
    closeDelay: `${profile.power.close_delay_seconds} s`,
    processes: [
      profile.main_executable.name,
      ...profile.associated_processes.map((process) => process.name).filter(Boolean),
    ],
    lastEvent: profile.enabled ? 'Inactivo' : 'Deshabilitado',
  }));
}

export function normalizeProfilePriorities(config: AppConfig, powerPlans: PowerPlan[] = []): AppConfig {
  return {
    ...config,
    profiles: config.profiles.map((profile) => {
      const priority = priorityForPlan(profile.power.on_start_plan_id, powerPlans);
      if (profile.power.priority === priority) return profile;

      return {
        ...profile,
        power: {
          ...profile.power,
          priority,
        },
      };
    }),
  };
}

export function addProfileToConfig(config: AppConfig, executablePath: string, powerPlans: PowerPlan[]): AppConfig {
  const profile = createProfileFromExecutable(executablePath, {
    defaultDelaySeconds: config.automation.default_close_delay_seconds,
    defaultNotificationsEnabled: config.automation.notifications_enabled,
    existingIds: config.profiles.map((item) => item.id),
    powerPlans,
  });

  return {
    ...config,
    profiles: [...config.profiles, profile],
  };
}

export function removeProfileFromConfig(config: AppConfig, profileId: string): AppConfig {
  return {
    ...config,
    profiles: config.profiles.filter((profile) => profile.id !== profileId),
  };
}

export function updateProfileConfig(
  config: AppConfig,
  profileId: string,
  update: ProfileUpdate,
  powerPlans: PowerPlan[] = [],
): AppConfig {
  return {
    ...config,
    profiles: config.profiles.map((profile) => {
      if (profile.id !== profileId) return profile;
      const nextStartPlan = update.startPlan ?? profile.power.on_start_plan_id;

      const nextProfile: ProfileConfig = {
        ...profile,
        name: update.name?.trim() || profile.name,
        enabled: update.enabled ?? profile.enabled,
        main_executable: {
          ...profile.main_executable,
          path: update.executablePath?.trim() ?? profile.main_executable.path,
          name: update.executablePath ? fileNameFromPath(update.executablePath) : profile.main_executable.name,
        },
        power: {
          ...profile.power,
          on_start_plan_id: nextStartPlan,
          priority:
            update.startPlan && update.startPlan !== profile.power.on_start_plan_id
              ? priorityForPlan(nextStartPlan, powerPlans)
              : profile.power.priority,
          close_delay_seconds: update.closeDelay ? closeDelaySeconds(update.closeDelay) : profile.power.close_delay_seconds,
        },
        notifications:
          typeof update.notify === 'boolean'
            ? {
                ...profile.notifications,
                on_activate: update.notify,
                on_restore: update.notify,
              }
            : profile.notifications,
      };

      if (update.closePlan) {
        if (update.closePlan === 'Restaurar plan anterior') {
          nextProfile.power.on_close_behavior = 'previous_plan';
          nextProfile.power.on_close_plan_id = null;
        } else {
          nextProfile.power.on_close_behavior = 'specific_plan';
          nextProfile.power.on_close_plan_id = update.closePlan;
        }
      }

      return nextProfile;
    }),
  };
}

export function addAssociatedProcessToProfile(
  config: AppConfig,
  profileId: string,
  process: AssociatedProcessInput,
): AppConfig {
  const processName = process.name.trim();
  if (!processName) return config;

  return {
    ...config,
    profiles: config.profiles.map((profile) => {
      if (profile.id !== profileId) return profile;
      if (profile.main_executable.name.toLowerCase() === processName.toLowerCase()) return profile;
      if (profile.associated_processes.some((item) => item.name.toLowerCase() === processName.toLowerCase())) {
        return profile;
      }

      return {
        ...profile,
        associated_processes: [
          ...profile.associated_processes,
          {
            name: processName,
            path: process.path ?? null,
            match_mode: process.path ? 'path_or_name' : 'name',
          },
        ],
      };
    }),
  };
}

export function removeAssociatedProcessFromProfile(
  config: AppConfig,
  profileId: string,
  processName: string,
): AppConfig {
  const normalizedName = processName.trim().toLowerCase();
  if (!normalizedName) return config;

  return {
    ...config,
    profiles: config.profiles.map((profile) => {
      if (profile.id !== profileId) return profile;
      if (profile.main_executable.name.toLowerCase() === normalizedName) return profile;

      return {
        ...profile,
        associated_processes: profile.associated_processes.filter(
          (process) => process.name.toLowerCase() !== normalizedName,
        ),
      };
    }),
  };
}

export function updateAppSettingsConfig(config: AppConfig, update: AppSettingsUpdate): AppConfig {
  return {
    ...config,
    agent: {
      ...config.agent,
      enabled: update.agentEnabled ?? config.agent.enabled,
      start_with_windows: update.startWithWindows ?? config.agent.start_with_windows,
      start_minimized: update.startMinimized ?? config.agent.start_minimized,
      show_tray_icon: update.showTrayIcon ?? config.agent.show_tray_icon,
    },
    automation: {
      ...config.automation,
      enabled: update.automationEnabled ?? config.automation.enabled,
      notifications_enabled: update.notificationsEnabled ?? config.automation.notifications_enabled,
    },
    ui: {
      ...config.ui,
      close_button_behavior: update.closeButtonBehavior ?? config.ui.close_button_behavior,
    },
  };
}

export interface CreateProfileOptions {
  defaultDelaySeconds?: number;
  defaultNotificationsEnabled?: boolean;
  existingIds?: string[];
  powerPlans?: PowerPlan[];
}

export function createProfileFromExecutable(executablePath: string, options: CreateProfileOptions = {}): ProfileConfig {
  const normalizedPath = executablePath.trim();
  const exeName = fileNameFromPath(normalizedPath);

  if (!exeName.toLowerCase().endsWith('.exe')) {
    throw new Error('Selecciona un ejecutable .exe');
  }

  const displayName = displayNameFromExe(exeName);
  const startPlan = preferredStartPlan(options.powerPlans ?? []);

  if (!startPlan) {
    throw new Error('No hay planes de energia disponibles');
  }

  return {
    id: uniqueProfileId(slugify(displayName), options.existingIds ?? []),
    name: displayName,
    enabled: true,
    main_executable: {
      name: exeName,
      path: normalizedPath,
    },
    associated_processes: [],
    activation: {
      match_mode: 'path_or_name',
      require_main_process: true,
    },
    power: {
      on_start_plan_id: startPlan,
      on_close_behavior: 'previous_plan',
      on_close_plan_id: null,
      close_delay_seconds: options.defaultDelaySeconds ?? 30,
      priority: priorityForPlan(startPlan, options.powerPlans ?? []),
    },
    notifications: {
      on_activate: options.defaultNotificationsEnabled ?? true,
      on_restore: options.defaultNotificationsEnabled ?? true,
      on_error: true,
    },
    ui: {
      icon_cache_key: null,
      accent: null,
    },
  };
}

export function planToLevel(planId: string, powerPlans: PowerPlan[] = [], fallbackPriority = 70): UiGameProfile['level'] {
  const plan = powerPlans.find((item) => item.id === planId);
  const value = normalizePlanText(`${planId} ${plan?.name ?? ''}`);
  if (value.includes('ultimate') || value.includes('max') || value.includes('máximo') || value.includes('maximo')) {
    return 'max';
  }
  if (value.includes('high') || value.includes('alto') || value.includes('performance') || value.includes('rendimiento')) {
    return 'high';
  }
  if (value.includes('balanced') || value.includes('equilibrado') || value.includes('balanceado') || value.includes('power saver')) {
    return 'balanced';
  }
  return priorityToLevel(fallbackPriority);
}

function normalizePlanText(value: string): string {
  return value
    .toLowerCase()
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '');
}

function priorityToLevel(priority: number): UiGameProfile['level'] {
  if (priority >= 90) return 'max';
  if (priority >= 60) return 'high';
  return 'balanced';
}

function priorityForPlan(planId: string, powerPlans: PowerPlan[] = []): number {
  const level = planToLevel(planId, powerPlans, 70);
  if (level === 'max') return 90;
  if (level === 'high') return 70;
  return 30;
}

function initials(name: string): string {
  const parts = name
    .split(/\s+/)
    .map((part) => part.trim())
    .filter(Boolean);
  const value = parts.length >= 2 ? `${parts[0][0]}${parts[1][0]}` : name.slice(0, 2);
  return value.toUpperCase();
}

function fileNameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? '';
}

function closeDelaySeconds(value: string): number {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : 30;
}

function displayNameFromExe(exeName: string): string {
  const withoutExtension = exeName.replace(/\.exe$/i, '');
  return withoutExtension
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((part) => `${part[0]?.toUpperCase() ?? ''}${part.slice(1)}`)
    .join(' ');
}

function preferredStartPlan(powerPlans: PowerPlan[]): string {
  const highPlan = powerPlans.find((plan) => {
    const name = plan.name.toLowerCase();
    return name.includes('alto rendimiento') || name.includes('high performance') || name.includes('ultimate');
  });

  return highPlan?.id ?? powerPlans[0]?.id ?? '';
}

function slugify(value: string): string {
  const slug = value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || 'profile';
}

function uniqueProfileId(baseId: string, existingIds: string[]): string {
  const taken = new Set(existingIds.map((id) => id.toLowerCase()));
  if (!taken.has(baseId.toLowerCase())) return baseId;

  let suffix = 2;
  while (taken.has(`${baseId}-${suffix}`.toLowerCase())) {
    suffix += 1;
  }
  return `${baseId}-${suffix}`;
}
