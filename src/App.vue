<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {
  Activity,
  CheckCircle2,
  Cpu,
  FilePlus2,
  Folder,
  Gauge,
  List,
  Menu,
  Minus,
  Play,
  Plus,
  Search,
  Settings,
  SlidersHorizontal,
  Sparkles,
  RefreshCw,
  Trash2,
  X,
  Zap,
} from '@lucide/vue';
import {
  addAssociatedProcessToProfile,
  addProfileToConfig,
  getAppConfig,
  normalizeProfilePriorities,
  planToLevel,
  profilesToUiGames,
  removeAssociatedProcessFromProfile,
  removeProfileFromConfig,
  saveAppConfig,
  updateAppSettingsConfig,
  updateProfileConfig,
  type AppSettingsUpdate,
  type AppConfig,
  type ProfileUpdate,
  type UiGameProfile,
} from '@/services/configApi';
import { detectProfileCandidates, type ProfileCandidate } from '@/services/autoDetect';
import { clearEvents, formatEventTime, getRecentEvents, type EventLogEntry } from '@/services/eventsApi';
import {
  agentTaskInstalled,
  agentStateSignature,
  agentStateTone,
  applyAgentStateToGames,
  describeAgentState,
  getAgentState,
  installAgentTask,
  shouldAutoInstallElevatedAgent,
  startAgentTask,
  wakeAgent,
  type AgentStateTone,
  type PublishedAgentState,
} from '@/services/agentApi';
import { pickExecutable } from '@/services/executableDialog';
import { gameSortModeLabel, nextGameSortMode, sortGames, type GameSortMode } from '@/services/gameList';
import { loadProfileIcons, type IconMap } from '@/services/iconApi';
import { filterProcesses, getOpenProcesses, type ProcessInfo } from '@/services/processApi';
import {
  getActivePowerPlan,
  getPowerPlans,
  resolvePlanId,
  setActivePowerPlan,
  toPowerPlanOptions,
  type InvokeFn,
  type PowerPlan,
} from '@/services/powerApi';
import { openExecutableFolder, openExternalUrl } from '@/services/shellApi';

type PowerLevel = 'max' | 'high' | 'balanced';
type GameStatus = 'active' | 'inactive' | 'disabled';
type GameProfile = UiGameProfile;
type ProcessDrawerMode = 'processes' | 'candidates' | 'associate';

const APP_VERSION = '0.1.0';
const GITHUB_PROFILE_URL = 'https://github.com/4ismael1';

const games = ref<GameProfile[]>([]);

const query = ref('');
const selectedId = ref('');
const listSortMode = ref<GameSortMode>('configured');
const automatic = ref(true);
const powerPlans = ref<PowerPlan[]>([]);
const activePowerPlan = ref<PowerPlan | null>(null);
const currentConfig = ref<AppConfig | null>(null);
const processPanelOpen = ref(false);
const settingsPanelOpen = ref(false);
const eventsPanelOpen = ref(false);
const processDrawerMode = ref<ProcessDrawerMode>('processes');
const openProcesses = ref<ProcessInfo[]>([]);
const detectedCandidates = ref<ProfileCandidate[]>([]);
const recentEvents = ref<EventLogEntry[]>([]);
const processQuery = ref('');
const agentStatusText = ref('Escuchando eventos de procesos');
const agentStatusTone = ref<AgentStateTone>('idle');
const powerLoading = ref(false);
const drawerLoading = ref(false);
const agentSetupLoading = ref(false);
const powerError = ref('');
const profileIcons = ref<IconMap>({});
const processIcons = ref<IconMap>({});
const agentTaskReady = ref(false);
const appReady = ref(false);
const notice = ref<{ kind: 'success' | 'info' | 'error'; message: string } | null>(null);
let noticeTimer: number | undefined;
let agentSnapshotTimer: number | undefined;
let processIconLoadId = 0;
let lastAgentStateSignature: string | undefined;
let lastPublishedAgentState: PublishedAgentState | null = null;
let agentSnapshotInFlight = false;

const tauriInvoke: InvokeFn = (command, args) => invoke(command, args);

const selectedGame = computed(() => games.value.find((game) => game.id === selectedId.value) ?? games.value[0] ?? null);
const filteredGames = computed(() => {
  const value = query.value.trim().toLowerCase();
  const visibleGames = value
    ? games.value.filter((game) => `${game.name} ${game.exe}`.toLowerCase().includes(value))
    : games.value;
  return sortGames(visibleGames, listSortMode.value);
});
const activeGame = computed(() => games.value.find((game) => game.status === 'active'));
const globalNotificationsEnabled = computed(() => currentConfig.value?.automation.notifications_enabled ?? true);
const filteredOpenProcesses = computed(() => filterProcesses(openProcesses.value, processQuery.value));
const filteredCandidates = computed(() => {
  const value = processQuery.value.trim().toLowerCase();
  if (!value) return detectedCandidates.value;
  return detectedCandidates.value.filter((candidate) =>
    `${candidate.name} ${candidate.executableName} ${candidate.executablePath} ${candidate.pid}`
      .toLowerCase()
      .includes(value),
  );
});
const drawerTitle = computed(() => {
  if (processDrawerMode.value === 'candidates') return 'Auto detectar';
  if (processDrawerMode.value === 'associate') return 'Asociar proceso';
  return 'Procesos abiertos';
});
const drawerCount = computed(() =>
  processDrawerMode.value === 'candidates' ? detectedCandidates.value.length : openProcesses.value.length,
);
const powerPlanOptions = computed(() => toPowerPlanOptions(powerPlans.value));
const controllingGame = computed(
  () =>
    games.value.find(
      (game) => game.status === 'active' && ['Controlando', 'Plan aplicado'].includes(game.lastEvent),
    ) ?? activeGame.value,
);
const currentPowerPlanName = computed(() => {
  if (activePowerPlan.value) return activePowerPlan.value.name;
  if (powerError.value) return 'No disponible';
  if (powerLoading.value) return 'Cargando...';
  return 'Sin leer';
});
const elevatedAgentActionLabel = computed(() => {
  if (agentSetupLoading.value) return 'Instalando';
  if (!agentTaskReady.value) return 'Reparar';
  return agentStatusTone.value === 'ready' ? 'Revaluar' : 'Iniciar';
});

function levelLabel(level: PowerLevel) {
  if (level === 'max') return 'Máximo rendimiento';
  if (level === 'high') return 'Alto rendimiento';
  return 'Equilibrado';
}

function statusLabel(status: GameStatus) {
  if (status === 'active') return 'Activo';
  if (status === 'disabled') return 'Deshabilitado';
  return 'Inactivo';
}

function eventKindLabel(kind: string) {
  if (kind === 'profile_activated') return 'Perfil activado';
  if (kind === 'power_plan_restored') return 'Plan restaurado';
  if (kind === 'restore_scheduled') return 'Restauracion programada';
  if (kind === 'agent_error') return 'Error del agente';
  return kind.split('_').join(' ');
}

async function updateSelectedGame(field: 'startPlan' | 'closePlan' | 'closeDelay', value: string) {
  const game = selectedGame.value;
  if (game) {
    game[field] = value;
    if (field === 'startPlan') {
      game.level = planToLevel(value, powerPlans.value);
    }
    await persistSelectedProfile({ [field]: value });
  }
}

async function refreshPowerState() {
  powerLoading.value = true;
  try {
    const [plans, active] = await Promise.all([getPowerPlans(tauriInvoke), getActivePowerPlan(tauriInvoke)]);
    powerPlans.value = plans;
    syncProfilePlanSelections(plans);
    activePowerPlan.value = active;
    powerError.value = '';
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function refreshConfig() {
  try {
    const loadedConfig = await getAppConfig(tauriInvoke);
    const config = normalizeProfilePriorities(loadedConfig, powerPlans.value);
    if (JSON.stringify(config.profiles) !== JSON.stringify(loadedConfig.profiles)) {
      await saveAppConfig(tauriInvoke, config);
    }
    currentConfig.value = config;
    automatic.value = config.automation.enabled;
    applyConfigProfiles(config);
    selectedId.value = games.value[0]?.id ?? '';
    syncProfilePlanSelections(powerPlans.value);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  }
}

async function refreshRecentEvents() {
  try {
    recentEvents.value = await getRecentEvents(tauriInvoke, 50);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  }
}

async function clearEventHistory() {
  if (!window.confirm('Borrar el historial de eventos de PowerShift?')) return;

  powerError.value = '';
  powerLoading.value = true;
  try {
    await clearEvents(tauriInvoke);
    recentEvents.value = [];
    showNotice('success', 'Historial de eventos borrado.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
  } finally {
    powerLoading.value = false;
  }
}

async function toggleEventsPanel() {
  eventsPanelOpen.value = !eventsPanelOpen.value;
  if (!eventsPanelOpen.value) return;
  settingsPanelOpen.value = false;
  processPanelOpen.value = false;
  await refreshRecentEvents();
}

async function addExecutableProfile() {
  powerError.value = '';
  const executablePath = await pickExecutable();
  if (!executablePath) return;

  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = addProfileToConfig(config, executablePath, powerPlans.value);
    await saveAppConfig(tauriInvoke, nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = nextConfig.profiles[nextConfig.profiles.length - 1]?.id ?? '';
    syncProfilePlanSelections(powerPlans.value);
    showNotice('success', 'Perfil agregado y listo para detectar.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function refreshActivePowerPlanSilently() {
  try {
    activePowerPlan.value = await getActivePowerPlan(tauriInvoke);
    powerError.value = '';
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  }
}

async function persistAppSettings(update: AppSettingsUpdate) {
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = updateAppSettingsConfig(config, update);
    await saveAppConfig(tauriInvoke, nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    automatic.value = nextConfig.automation.enabled;
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function toggleAutomatic() {
  const enabled = !automatic.value;
  await persistAppSettings({ automationEnabled: enabled });
  agentStatusText.value = enabled ? 'Agente activo' : 'Automatización pausada';
  showNotice('info', enabled ? 'Automatización activada.' : 'Automatización pausada.');
}

async function openGithubProfile() {
  try {
    await openExternalUrl(tauriInvoke, GITHUB_PROFILE_URL);
  } catch (error) {
    showNotice('error', error instanceof Error ? error.message : String(error));
  }
}

function cycleGameSortMode() {
  listSortMode.value = nextGameSortMode(listSortMode.value);
  showNotice('info', `Lista: ${gameSortModeLabel(listSortMode.value)}.`);
}

async function refreshAgentTaskInstalled() {
  try {
    agentTaskReady.value = await agentTaskInstalled(tauriInvoke);
  } catch {
    agentTaskReady.value = false;
  }
}

async function installElevatedAgent(options: { automatic?: boolean } = {}) {
  const automaticInstall = options.automatic ?? false;
  powerError.value = '';
  if (automaticInstall) {
    agentSetupLoading.value = true;
    agentStatusText.value = 'Preparando agente elevado';
    agentStatusTone.value = 'warning';
    showNotice('info', 'Windows pedirá permiso para instalar el agente elevado.');
  } else {
    powerLoading.value = true;
    showNotice('info', 'Windows pedira permiso para reparar el agente elevado.');
  }
  try {
    await installAgentTask(tauriInvoke);
    agentTaskReady.value = true;
    await refreshAgentSnapshot({ forceLinkedRefresh: true });
    showNotice('success', automaticInstall ? 'Agente elevado preparado.' : 'Agente elevado instalado e iniciado.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    agentStatusText.value = automaticInstall ? 'Instalación del agente pendiente' : agentStatusText.value;
    agentStatusTone.value = automaticInstall ? 'warning' : agentStatusTone.value;
    if (automaticInstall) {
      settingsPanelOpen.value = true;
      processPanelOpen.value = false;
      eventsPanelOpen.value = false;
      showNotice('error', 'No se pudo instalar automáticamente. Reintenta desde Configuración.');
    } else {
      showNotice('error', powerError.value);
    }
  } finally {
    if (automaticInstall) {
      agentSetupLoading.value = false;
    } else {
      powerLoading.value = false;
    }
  }
}

async function startElevatedAgent() {
  powerError.value = '';
  powerLoading.value = true;
  try {
    await startAgentTask(tauriInvoke);
    agentTaskReady.value = true;
    await refreshAgentSnapshot({ forceLinkedRefresh: true });
    showNotice('success', 'Agente elevado iniciado.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
  } finally {
    powerLoading.value = false;
  }
}

async function handleElevatedAgentAction() {
  if (!agentTaskReady.value) {
    await installElevatedAgent();
    return;
  }

  if (agentStatusTone.value === 'ready') {
    await runAgentScanNow();
    return;
  }

  await startElevatedAgent();
}

async function openProcessPanel() {
  if (processPanelOpen.value && processDrawerMode.value === 'processes') {
    processPanelOpen.value = false;
    return;
  }
  processPanelOpen.value = true;
  settingsPanelOpen.value = false;
  eventsPanelOpen.value = false;
  processDrawerMode.value = 'processes';
  processQuery.value = '';
  await refreshOpenProcesses();
}

async function openAssociateProcessPanel() {
  if (!selectedGame.value) return;
  if (processPanelOpen.value && processDrawerMode.value === 'associate') {
    processPanelOpen.value = false;
    return;
  }
  processPanelOpen.value = true;
  settingsPanelOpen.value = false;
  eventsPanelOpen.value = false;
  processDrawerMode.value = 'associate';
  processQuery.value = '';
  await refreshOpenProcesses();
}

async function autoDetectProfiles() {
  powerError.value = '';
  processDrawerMode.value = 'candidates';
  processPanelOpen.value = true;
  settingsPanelOpen.value = false;
  eventsPanelOpen.value = false;
  processQuery.value = '';
  detectedCandidates.value = [];
  processIcons.value = {};
  drawerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    currentConfig.value = config;
    openProcesses.value = await getOpenProcesses(tauriInvoke);
    detectedCandidates.value = detectProfileCandidates(config, openProcesses.value);
    void refreshProcessIcons();
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    drawerLoading.value = false;
  }
}

async function addDetectedCandidate(candidate: ProfileCandidate) {
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = addProfileToConfig(config, candidate.executablePath, powerPlans.value);
    await saveAppConfig(tauriInvoke, nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = nextConfig.profiles[nextConfig.profiles.length - 1]?.id ?? '';
    detectedCandidates.value = detectProfileCandidates(nextConfig, openProcesses.value);
    await refreshProcessIcons();
    syncProfilePlanSelections(powerPlans.value);
    showNotice('success', 'Candidato agregado como perfil.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function associateOpenProcess(process: ProcessInfo) {
  if (!selectedGame.value) return;

  const profileId = selectedGame.value.id;
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = addAssociatedProcessToProfile(config, profileId, {
      name: process.name,
      path: process.path ?? null,
    });
    await saveAppConfig(tauriInvoke, nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = profileId;
    syncProfilePlanSelections(powerPlans.value);
    showNotice('success', 'Proceso asociado al perfil.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function persistSelectedProfile(update: ProfileUpdate) {
  if (!selectedGame.value) return;
  if (typeof update.notify === 'boolean' && !globalNotificationsEnabled.value) return;

  const profileId = selectedGame.value.id;
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = updateProfileConfig(config, profileId, update, powerPlans.value);
    await saveAppConfig(tauriInvoke, nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = profileId;
    syncProfilePlanSelections(powerPlans.value);
    showNotice('success', 'Perfil actualizado.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function deleteProfile(profileId: string) {
  const profile = games.value.find((game) => game.id === profileId);
  if (!profile) return;
  if (!window.confirm(`Eliminar "${profile.name}" de PowerShift?`)) return;

  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = removeProfileFromConfig(config, profileId);
    await saveAppConfig(tauriInvoke, nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = games.value[0]?.id ?? '';
    syncProfilePlanSelections(powerPlans.value);
    showNotice('success', 'Perfil eliminado.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function removeAssociatedProcess(processName: string) {
  if (!selectedGame.value) return;
  if (processName.toLowerCase() === selectedGame.value.exe.toLowerCase()) return;

  const profileId = selectedGame.value.id;
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = removeAssociatedProcessFromProfile(config, profileId, processName);
    await saveAppConfig(tauriInvoke, nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = profileId;
    syncProfilePlanSelections(powerPlans.value);
    showNotice('success', 'Proceso quitado.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function openSelectedExecutableFolder() {
  if (!selectedGame.value?.path) {
    showNotice('error', 'Este perfil no tiene ruta de ejecutable.');
    return;
  }

  try {
    await openExecutableFolder(tauriInvoke, selectedGame.value.path);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
  }
}

function applyConfigProfiles(config: AppConfig) {
  const configuredGames = profilesToUiGames(config, powerPlans.value);
  games.value = lastPublishedAgentState
    ? applyAgentStateToGames(configuredGames, lastPublishedAgentState, Date.now())
    : configuredGames;
  profileIcons.value = {};
  void refreshProfileIcons();
}

async function refreshProfileIcons() {
  try {
    profileIcons.value = await loadProfileIcons(tauriInvoke, games.value);
  } catch {
    profileIcons.value = {};
  }
}

async function refreshOpenProcesses() {
  powerError.value = '';
  drawerLoading.value = true;
  try {
    openProcesses.value = await getOpenProcesses(tauriInvoke);
    void refreshProcessIcons();
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    drawerLoading.value = false;
  }
}

async function refreshDrawer() {
  if (processDrawerMode.value === 'candidates') {
    await autoDetectProfiles();
  } else {
    await refreshOpenProcesses();
  }
}

function processIconKey(process: ProcessInfo) {
  return `${process.pid}-${process.path ?? process.name}`;
}

function candidateIconKey(candidate: ProfileCandidate) {
  return `candidate-${candidate.executablePath}`;
}

async function refreshProcessIcons() {
  const loadId = ++processIconLoadId;
  const processItems = (processDrawerMode.value === 'candidates' ? [] : openProcesses.value)
    .filter((process) => process.path)
    .slice(0, 80)
    .map((process) => ({ id: processIconKey(process), path: process.path ?? '' }));
  const candidateItems =
    processDrawerMode.value === 'candidates'
      ? detectedCandidates.value.slice(0, 80).map((candidate) => ({
          id: candidateIconKey(candidate),
          path: candidate.executablePath,
        }))
      : [];

  try {
    const icons = await loadProfileIcons(tauriInvoke, [...processItems, ...candidateItems]);
    if (loadId === processIconLoadId) {
      processIcons.value = icons;
    }
  } catch {
    if (loadId === processIconLoadId) {
      processIcons.value = {};
    }
  }
}

function syncProfilePlanSelections(plans: PowerPlan[]) {
  for (const game of games.value) {
    const startPlanId = resolvePlanId(plans, game.startPlan);
    if (startPlanId) {
      game.startPlan = startPlanId;
      game.level = planToLevel(startPlanId, plans);
    }

    const closePlanId = resolvePlanId(plans, game.closePlan);
    if (closePlanId) game.closePlan = closePlanId;
  }
}

async function testSelectedProfile() {
  if (!selectedGame.value) return;

  const planId = resolvePlanId(powerPlans.value, selectedGame.value.startPlan);
  if (!planId) {
    powerError.value = `No se encontro el plan "${selectedGame.value.startPlan}"`;
    return;
  }

  powerLoading.value = true;
  try {
    await setActivePowerPlan(tauriInvoke, planId);
    activePowerPlan.value = await getActivePowerPlan(tauriInvoke);
    powerError.value = '';
    showNotice('success', `Perfil probado: ${activePowerPlan.value.name} aplicado.`);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
  } finally {
    powerLoading.value = false;
  }
}

function applyAgentState(state: PublishedAgentState | null) {
  lastPublishedAgentState = state;
  const now = Date.now();
  games.value = applyAgentStateToGames(games.value, state, now);
  agentStatusText.value = describeAgentState(state, agentTaskReady.value, now);
  agentStatusTone.value = agentStateTone(state, agentTaskReady.value, now);
}

async function refreshAgentSnapshot(
  options: { refreshPower?: boolean; refreshEvents?: boolean; forceLinkedRefresh?: boolean } = {},
) {
  if (agentSnapshotInFlight) return;

  agentSnapshotInFlight = true;
  const refreshPower = options.refreshPower ?? true;
  const refreshEvents = options.refreshEvents ?? true;
  try {
    const state = await getAgentState(tauriInvoke);
    const signature = agentStateSignature(state);
    const stateChanged = signature !== lastAgentStateSignature;
    lastAgentStateSignature = signature;
    const refreshLinkedState = options.forceLinkedRefresh || stateChanged;

    applyAgentState(state);
    if (refreshPower && refreshLinkedState) await refreshActivePowerPlanSilently();
    if (refreshEvents && refreshLinkedState) await refreshRecentEvents();
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    agentSnapshotInFlight = false;
  }
}

async function bootstrapElevatedAgentIfNeeded() {
  const config = currentConfig.value;
  if (!config) return;
  if (!shouldAutoInstallElevatedAgent(config.agent.enabled, agentTaskReady.value, '__TAURI_INTERNALS__' in window)) return;

  await installElevatedAgent({ automatic: true });
}

async function runAgentScanNow() {
  powerError.value = '';
  powerLoading.value = true;
  try {
    await wakeAgent(tauriInvoke);
    await new Promise((resolve) => window.setTimeout(resolve, 500));
    await refreshAgentSnapshot({ forceLinkedRefresh: true });
    showNotice('success', 'Agente despertado y perfiles revaluados.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
  } finally {
    powerLoading.value = false;
  }
}

function toggleSettingsPanel() {
  settingsPanelOpen.value = !settingsPanelOpen.value;
  if (settingsPanelOpen.value) {
    processPanelOpen.value = false;
    eventsPanelOpen.value = false;
  }
}

function showNotice(kind: 'success' | 'info' | 'error', message: string) {
  notice.value = { kind, message };
  if (noticeTimer) window.clearTimeout(noticeTimer);
  noticeTimer = window.setTimeout(() => {
    notice.value = null;
    noticeTimer = undefined;
  }, 3200);
}

async function initializeApp() {
  try {
    await refreshConfig();
  } finally {
    appReady.value = true;
  }
  await finishStartupRefresh();
}

async function finishStartupRefresh() {
  await Promise.allSettled([refreshPowerState(), refreshRecentEvents(), refreshAgentTaskInstalled()]);
  await refreshAgentSnapshot();
  window.setTimeout(() => {
    void bootstrapElevatedAgentIfNeeded();
  }, 900);
}

onMounted(() => {
  void initializeApp();
  agentSnapshotTimer = window.setInterval(() => {
    void refreshAgentSnapshot();
  }, 2500);
});

onBeforeUnmount(() => {
  if (noticeTimer) window.clearTimeout(noticeTimer);
  if (agentSnapshotTimer) window.clearInterval(agentSnapshotTimer);
});

async function minimize() {
  const appWindow = await getTauriWindow();
  await appWindow?.minimize().catch(() => undefined);
}

async function toggleMaximize() {
  const appWindow = await getTauriWindow();
  await appWindow?.toggleMaximize().catch(() => undefined);
}

async function close() {
  if (!('__TAURI_INTERNALS__' in window)) return;
  await invoke('handle_close_button').catch((error) => {
    powerError.value = error instanceof Error ? error.message : String(error);
  });
}

async function startWindowDrag(event: PointerEvent) {
  if (event.button !== 0) return;
  const target = event.target as HTMLElement;
  if (target.closest('button, input, select')) return;
  const appWindow = await getTauriWindow();
  await appWindow?.startDragging().catch(() => undefined);
}

async function getTauriWindow() {
  if (!('__TAURI_INTERNALS__' in window)) return undefined;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow();
}
</script>

<template>
  <div class="app-frame" :class="{ ready: appReady }">
    <header class="titlebar" data-tauri-drag-region @pointerdown="startWindowDrag">
      <div class="brand" data-tauri-drag-region>
        <div class="brand-mark">
          <Zap :size="27" stroke-width="2.7" fill="currentColor" />
        </div>
        <span data-tauri-drag-region>PowerShift</span>
      </div>

      <div class="title-status">
        <button class="mode-toggle" :class="{ enabled: automatic }" @click="toggleAutomatic">
          <span class="mode-dot"></span>
          <span>Automático</span>
          <strong>{{ automatic ? 'ON' : 'OFF' }}</strong>
        </button>
        <div class="current-plan">
          <span>Plan actual:</span>
          <strong :title="powerError || currentPowerPlanName">{{ currentPowerPlanName }}</strong>
        </div>
      </div>

      <div class="window-actions">
        <button class="icon-button" :class="{ active: settingsPanelOpen }" aria-label="Configuración" @click="toggleSettingsPanel">
          <Settings :size="18" />
        </button>
        <div class="caption-group">
          <button class="caption-button" aria-label="Minimizar" @click="minimize">
            <Minus :size="18" />
          </button>
          <button class="caption-button" aria-label="Maximizar" @click="toggleMaximize">
            <span class="maximize-glyph"></span>
          </button>
          <button class="caption-button close" aria-label="Cerrar" @click="close">
            <X :size="18" />
          </button>
        </div>
      </div>
    </header>

    <section class="command-strip">
      <label class="search-box">
        <Search :size="20" />
        <input v-model="query" type="search" placeholder="Buscar juego..." />
      </label>

      <button class="primary-action" :disabled="powerLoading" @click="autoDetectProfiles">
        <Sparkles :size="18" />
        <span>Auto detectar</span>
      </button>
      <button class="secondary-action" :disabled="powerLoading" @click="addExecutableProfile">
        <FilePlus2 :size="19" />
        <span>Agregar exe</span>
      </button>
      <button class="secondary-action wide" :disabled="powerLoading" @click="openProcessPanel">
        <List :size="19" />
        <span>Procesos abiertos</span>
      </button>
    </section>

    <main class="workspace">
      <aside class="game-panel">
        <div class="panel-heading">
          <span>Juegos configurados ({{ games.length }})</span>
          <button
            class="subtle-icon"
            :class="{ active: listSortMode !== 'configured' }"
            :aria-label="`Ordenar: ${gameSortModeLabel(listSortMode)}`"
            :title="`Ordenar: ${gameSortModeLabel(listSortMode)}`"
            @click="cycleGameSortMode"
          >
            <SlidersHorizontal :size="18" />
          </button>
        </div>

        <div class="game-list">
          <div v-if="filteredGames.length === 0" class="empty-state">
            <strong>Sin perfiles configurados</strong>
            <span>Agrega un ejecutable para empezar a automatizar planes.</span>
          </div>

          <div
            v-for="game in filteredGames"
            :key="game.id"
            role="button"
            tabindex="0"
            class="game-row"
            :class="{ selected: game.id === selectedId, active: game.status === 'active', disabled: game.status === 'disabled' }"
            @click="selectedId = game.id"
            @keydown.enter="selectedId = game.id"
            @keydown.space.prevent="selectedId = game.id"
          >
            <span class="row-state"></span>
            <span class="game-icon" :class="{ [game.iconClass]: !profileIcons[game.id] }">
              <img v-if="profileIcons[game.id]" :src="profileIcons[game.id]" alt="" />
              <template v-else>{{ game.iconText }}</template>
            </span>
            <span class="game-copy">
              <strong>{{ game.name }}</strong>
              <small>{{ game.exe }}</small>
            </span>
            <span class="row-meta">
              <span class="level-badge" :class="game.level">{{ levelLabel(game.level) }}</span>
              <span class="state-label" :class="game.status">
                <span></span>
                {{ game.status === 'active' ? game.lastEvent : statusLabel(game.status) }}
              </span>
            </span>
            <button class="row-action" :disabled="powerLoading" aria-label="Eliminar perfil" @click.stop="deleteProfile(game.id)">
              <Trash2 :size="17" />
            </button>
          </div>
        </div>
      </aside>

      <section class="details-panel">
        <div v-if="!selectedGame" class="details-empty-state">
          <div class="brand-mark">
            <Zap :size="32" stroke-width="2.7" fill="currentColor" />
          </div>
          <strong>No hay perfil seleccionado</strong>
          <span>La configuracion real esta vacia. El siguiente paso es activar Agregar exe.</span>
        </div>

        <template v-else>
        <div class="profile-header">
          <span class="selected-art" :class="{ [selectedGame.iconClass]: !profileIcons[selectedGame.id] }">
            <img v-if="profileIcons[selectedGame.id]" :src="profileIcons[selectedGame.id]" alt="" />
            <template v-else>{{ selectedGame.iconText }}</template>
          </span>

          <div class="identity-fields">
            <label>
              <span>Nombre del juego</span>
              <input
                :value="selectedGame.name"
                @change="persistSelectedProfile({ name: ($event.target as HTMLInputElement).value })"
              />
            </label>
            <label>
              <span>Ejecutable</span>
              <div class="path-field">
                <input :value="selectedGame.path" readonly />
                <button class="square-tool" aria-label="Abrir carpeta del ejecutable" @click="openSelectedExecutableFolder">
                  <Folder :size="20" />
                </button>
              </div>
            </label>
          </div>
        </div>

        <div class="settings-grid">
          <div class="profile-column">
            <div class="setting-row compact">
              <span>Perfil activo</span>
              <button
                class="switch"
                :class="{ on: selectedGame.enabled }"
                aria-label="Activar perfil"
                @click="persistSelectedProfile({ enabled: !selectedGame.enabled })"
              >
                <span></span>
              </button>
            </div>

            <label class="select-field">
              <span>Plan al iniciar</span>
              <select
                :value="selectedGame.startPlan"
                @change="updateSelectedGame('startPlan', ($event.target as HTMLSelectElement).value)"
              >
                <option v-for="plan in powerPlanOptions" :key="`start-${plan.id}`" :value="plan.id">
                  {{ plan.name }}
                </option>
              </select>
            </label>

            <label class="select-field">
              <span>Al cerrar</span>
              <select
                :value="selectedGame.closePlan"
                @change="updateSelectedGame('closePlan', ($event.target as HTMLSelectElement).value)"
              >
                <option v-for="plan in powerPlanOptions" :key="`close-${plan.id}`" :value="plan.id">
                  {{ plan.name }}
                </option>
                <option>Restaurar plan anterior</option>
              </select>
            </label>

            <label class="select-field">
              <span>Retardo al cerrar</span>
              <select
                :value="selectedGame.closeDelay"
                @change="updateSelectedGame('closeDelay', ($event.target as HTMLSelectElement).value)"
              >
                <option>15 s</option>
                <option>30 s</option>
                <option>45 s</option>
                <option>60 s</option>
              </select>
            </label>

          </div>

          <div class="process-column">
            <div class="setting-row compact">
              <span>Mostrar notificación</span>
              <button
                class="switch"
                :class="{ on: selectedGame.notify && globalNotificationsEnabled }"
                :disabled="!globalNotificationsEnabled"
                aria-label="Mostrar notificación"
                @click="persistSelectedProfile({ notify: !selectedGame.notify })"
              >
                <span></span>
              </button>
            </div>

            <div class="process-list">
              <div class="process-title">Procesos asociados</div>
              <button
                v-for="process in selectedGame.processes"
                :key="process"
                class="process-row"
                :disabled="process.toLowerCase() === selectedGame.exe.toLowerCase() || powerLoading"
                @click="removeAssociatedProcess(process)"
              >
                <Cpu :size="15" />
                <span>{{ process }}</span>
                <Trash2 v-if="process.toLowerCase() !== selectedGame.exe.toLowerCase()" :size="15" />
                <Menu v-else :size="17" />
              </button>
            </div>

            <button class="add-process" @click="openAssociateProcessPanel">
              <Plus :size="18" />
              <span>Agregar proceso</span>
            </button>

            <button class="test-button profile-test-button" :disabled="powerLoading" @click="testSelectedProfile">
              <Play :size="15" fill="currentColor" />
              <span>{{ powerLoading ? 'Aplicando...' : 'Probar perfil' }}</span>
            </button>
          </div>
        </div>
        </template>
      </section>
    </main>

    <section v-if="processPanelOpen" class="process-drawer">
      <header class="drawer-header">
        <div>
          <strong>{{ drawerTitle }}</strong>
          <span>{{ drawerCount }} detectados</span>
        </div>
        <button class="icon-button" aria-label="Cerrar procesos abiertos" @click="processPanelOpen = false">
          <X :size="18" />
        </button>
      </header>

      <label class="drawer-search">
        <Search :size="18" />
        <input v-model="processQuery" type="search" placeholder="Filtrar..." />
      </label>

      <div class="drawer-list">
        <template v-if="processDrawerMode === 'candidates'">
          <div v-if="drawerLoading" class="drawer-empty">
            <strong>Buscando procesos</strong>
            <span>Detectando candidatos abiertos...</span>
          </div>
          <div v-else-if="filteredCandidates.length === 0" class="drawer-empty">
            <strong>Sin candidatos</strong>
            <span>Abre un juego y vuelve a ejecutar la detección.</span>
          </div>
          <div v-for="candidate in drawerLoading ? [] : filteredCandidates" :key="candidate.id" class="open-process-row candidate-row">
            <span class="drawer-app-icon">
              <img v-if="processIcons[candidateIconKey(candidate)]" :src="processIcons[candidateIconKey(candidate)]" alt="" />
              <Cpu v-else :size="15" />
            </span>
            <span>
              <strong>{{ candidate.name }}</strong>
              <small>{{ candidate.executablePath }}</small>
            </span>
            <button class="mini-add" :disabled="powerLoading" @click="addDetectedCandidate(candidate)">Agregar</button>
          </div>
        </template>

        <template v-else>
          <div v-if="drawerLoading" class="drawer-empty">
            <strong>Leyendo procesos</strong>
            <span>Actualizando lista...</span>
          </div>
          <div v-for="process in drawerLoading ? [] : filteredOpenProcesses" :key="`${process.pid}-${process.name}`" class="open-process-row">
            <span class="drawer-app-icon">
              <img v-if="processIcons[processIconKey(process)]" :src="processIcons[processIconKey(process)]" alt="" />
              <Cpu v-else :size="15" />
            </span>
            <span>
              <strong>{{ process.name }}</strong>
              <small>{{ process.path ?? `PID ${process.pid}` }}</small>
            </span>
            <button
              v-if="processDrawerMode === 'associate'"
              class="mini-add"
              :disabled="powerLoading"
              @click="associateOpenProcess(process)"
            >
              Asociar
            </button>
            <small v-else>{{ process.pid }}</small>
          </div>
        </template>
      </div>

      <footer class="drawer-footer">
        <button class="secondary-action" :disabled="powerLoading || drawerLoading" @click="refreshDrawer">
          <RefreshCw :size="17" />
          <span>{{ drawerLoading ? 'Actualizando...' : 'Refrescar' }}</span>
        </button>
      </footer>
    </section>

    <section v-if="settingsPanelOpen" class="settings-drawer">
      <header class="drawer-header">
        <div>
          <strong>Configuración</strong>
          <span>Preferencias generales</span>
        </div>
        <button class="icon-button" aria-label="Cerrar configuracion" @click="settingsPanelOpen = false">
          <X :size="18" />
        </button>
      </header>

      <div class="settings-list" v-if="currentConfig">
        <div class="setting-line">
          <span>
            <strong>Automatización</strong>
            <small>Cambiar planes automaticamente</small>
          </span>
          <button
            class="switch"
            :class="{ on: currentConfig.automation.enabled }"
            @click="toggleAutomatic"
          >
            <span></span>
          </button>
        </div>

        <div class="setting-line">
          <span>
            <strong>Notificaciones</strong>
            <small>Permitir avisos del agente y perfiles nuevos</small>
          </span>
          <button
            class="switch"
            :class="{ on: currentConfig.automation.notifications_enabled }"
            @click="persistAppSettings({ notificationsEnabled: !currentConfig.automation.notifications_enabled })"
          >
            <span></span>
          </button>
        </div>

        <div class="setting-line">
          <span>
            <strong>Iniciar con Windows</strong>
            <small>Arranca el agente y la bandeja al iniciar sesion</small>
          </span>
          <button
            class="switch"
            :class="{ on: currentConfig.agent.start_with_windows }"
            @click="persistAppSettings({ startWithWindows: !currentConfig.agent.start_with_windows })"
          >
            <span></span>
          </button>
        </div>

        <div class="setting-line">
          <span>
            <strong>Iniciar en segundo plano</strong>
            <small>No abrir la ventana principal al iniciar</small>
          </span>
          <button
            class="switch"
            :class="{ on: currentConfig.agent.start_minimized }"
            @click="persistAppSettings({ startMinimized: !currentConfig.agent.start_minimized })"
          >
            <span></span>
          </button>
        </div>

        <div class="setting-line">
          <span>
            <strong>Icono en bandeja</strong>
            <small>Mantener el tray liviano para abrir PowerShift</small>
          </span>
          <button
            class="switch"
            :class="{ on: currentConfig.agent.show_tray_icon }"
            @click="persistAppSettings({ showTrayIcon: !currentConfig.agent.show_tray_icon })"
          >
            <span></span>
          </button>
        </div>

        <div class="setting-line">
          <span>
            <strong>Agente elevado</strong>
            <small>{{ agentTaskReady ? agentStatusText : 'Requerido para eventos WMI de procesos' }}</small>
          </span>
          <button class="secondary-action compact" :disabled="powerLoading || agentSetupLoading" @click="handleElevatedAgentAction">
            <RefreshCw v-if="agentTaskReady && agentStatusTone === 'ready'" :size="16" />
            <Play v-else :size="16" />
            <span>{{ elevatedAgentActionLabel }}</span>
          </button>
        </div>

        <div class="setting-line">
          <span>
            <strong>Historial de eventos</strong>
            <small>Diagnóstico y cambios recientes</small>
          </span>
          <button class="secondary-action compact" :disabled="powerLoading" @click="toggleEventsPanel">
            <List :size="16" />
            <span>Abrir</span>
          </button>
        </div>

        <label class="select-field">
          <span>Botón X</span>
          <select
            :value="currentConfig.ui.close_button_behavior"
            @change="persistAppSettings({ closeButtonBehavior: ($event.target as HTMLSelectElement).value })"
          >
            <option value="hide_window">Cerrar solo ventana</option>
            <option value="exit_app">Salir de PowerShift</option>
          </select>
        </label>

        <div class="settings-about">
          <span>PowerShift v{{ APP_VERSION }}</span>
          <button class="secondary-action compact" @click="openGithubProfile">
            <svg class="github-mark" viewBox="0 0 16 16" aria-hidden="true">
              <path
                fill="currentColor"
                d="M8 0.2a8 8 0 0 0-2.5 15.6c0.4 0.1 0.5-0.2 0.5-0.4v-1.4c-2.2 0.5-2.7-0.9-2.7-0.9-0.4-0.9-0.9-1.1-0.9-1.1-0.7-0.5 0.1-0.5 0.1-0.5 0.8 0.1 1.2 0.8 1.2 0.8 0.7 1.2 1.9 0.9 2.3 0.7 0.1-0.5 0.3-0.9 0.5-1.1-1.8-0.2-3.6-0.9-3.6-3.9 0-0.9 0.3-1.6 0.8-2.1-0.1-0.2-0.4-1 0.1-2.1 0 0 0.7-0.2 2.2 0.8a7.6 7.6 0 0 1 4 0c1.5-1 2.2-0.8 2.2-0.8 0.5 1.1 0.2 1.9 0.1 2.1 0.5 0.6 0.8 1.3 0.8 2.1 0 3-1.8 3.7-3.6 3.9 0.3 0.2 0.6 0.7 0.6 1.5v2.1c0 0.2 0.1 0.5 0.6 0.4A8 8 0 0 0 8 0.2Z"
              />
            </svg>
            <span>GitHub</span>
          </button>
        </div>
      </div>
    </section>

    <section v-if="eventsPanelOpen" class="events-drawer">
      <header class="drawer-header">
        <div>
          <strong>Eventos</strong>
          <span>{{ recentEvents.length }} recientes</span>
        </div>
        <button class="icon-button" aria-label="Cerrar eventos" @click="eventsPanelOpen = false">
          <X :size="18" />
        </button>
      </header>

      <div class="event-list">
        <div v-if="recentEvents.length === 0" class="drawer-empty">
          <strong>Sin eventos</strong>
          <span>El agente registrará cambios de plan, restauraciones y errores aquí.</span>
        </div>
        <div v-for="event in recentEvents" :key="`${event.timestamp_ms}-${event.kind}-${event.message}`" class="event-row">
          <span class="event-dot" :class="event.level"></span>
          <span class="event-copy">
            <strong>{{ event.message }}</strong>
            <small>{{ formatEventTime(event.timestamp_ms) }} · {{ eventKindLabel(event.kind) }}</small>
          </span>
        </div>
      </div>

      <footer class="drawer-footer">
        <button class="secondary-action danger" :disabled="powerLoading || recentEvents.length === 0" @click="clearEventHistory">
          <Trash2 :size="17" />
          <span>Borrar historial</span>
        </button>
        <button class="secondary-action" :disabled="powerLoading" @click="refreshRecentEvents">
          <RefreshCw :size="17" />
          <span>Refrescar</span>
        </button>
      </footer>
    </section>

    <footer class="statusbar">
      <div class="listener-state" :class="agentStatusTone">
        <Activity :size="18" />
        <span>{{ powerError ? powerError : agentStatusText }}</span>
        <Gauge :size="18" />
        <span class="live-dot"></span>
      </div>
    </footer>

    <div v-if="notice" class="toast" :class="notice.kind">
      <CheckCircle2 :size="17" />
      <span>{{ notice.message }}</span>
    </div>
  </div>
</template>
