<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import {
  Activity,
  CheckCircle2,
  FilePlus2,
  Gauge,
  List,
  Minus,
  Search,
  Settings,
  SlidersHorizontal,
  Sparkles,
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
  takeConfigRecoveryWarning,
  updateAppSettingsConfig,
  updateAssociatedProcessRole,
  updateProfileConfig,
  type AssociatedProcessRole,
  type AppSettingsUpdate,
  type AppConfig,
  type ConfigSaveOutcome,
  type ProfileUpdate,
  type UiGameProfile,
} from '@/services/configApi';
import { detectProfileCandidates, type ProfileCandidate } from '@/services/autoDetect';
import { clearEvents, getRecentEvents, type EventLogEntry } from '@/services/eventsApi';
import {
  agentTaskInstalled,
  agentStateSignature,
  agentStateTone,
  applyAgentStateToGames,
  describeAgentState,
  getAgentState,
  installAgentTask,
  promoteActiveProfile,
  startAgentTask,
  wakeAgent,
  type AgentStateTone,
  type PublishedAgentState,
} from '@/services/agentApi';
import { pickExecutable } from '@/services/executableDialog';
import { gameSortModeLabel, nextGameSortMode, sortGames, type GameSortMode } from '@/services/gameList';
import {
  candidateIconMapKey,
  loadProfileIcons,
  processIconMapKey,
  type IconMap,
} from '@/services/iconApi';
import { getOpenProcesses, type ProcessInfo } from '@/services/processApi';
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
import ConfirmDialog from '@/components/ConfirmDialog.vue';
import EventsDrawer from '@/components/EventsDrawer.vue';
import ProcessDrawer, { type ProcessDrawerMode } from '@/components/ProcessDrawer.vue';
import ProfileEditor from '@/components/ProfileEditor.vue';
import SettingsDrawer from '@/components/SettingsDrawer.vue';

type PowerLevel = 'max' | 'high' | 'balanced';
type GameStatus = 'active' | 'inactive' | 'disabled';
type GameProfile = UiGameProfile;
type ConfirmationRequest = {
  title: string;
  message: string;
  confirmLabel: string;
  action: () => Promise<void>;
};

const GITHUB_PROFILE_URL = 'https://github.com/4ismael1';

const games = ref<GameProfile[]>([]);
const appVersion = ref('');

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
const confirmation = ref<ConfirmationRequest | null>(null);
let noticeTimer: number | undefined;
let agentSnapshotTimer: number | undefined;
let unlistenAgentState: UnlistenFn | undefined;
let processIconLoadId = 0;
let lastAgentStateSignature: string | undefined;
let lastPublishedAgentState: PublishedAgentState | null = null;
let agentSnapshotInFlight = false;
let agentSnapshotPending = false;
let drawerTrigger: HTMLElement | null = null;

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
const closeDelayOptions = computed(() => {
  const delays = new Set([0, 5, 10, 15, 30, 45, 60, 120]);
  const configured = Number.parseInt(selectedGame.value?.closeDelay ?? '', 10);
  if (Number.isFinite(configured) && configured >= 0) delays.add(configured);
  return [...delays].sort((left, right) => left - right).map((seconds) => `${seconds} s`);
});
const drawerOpen = computed(
  () => processPanelOpen.value || settingsPanelOpen.value || eventsPanelOpen.value,
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

async function updateSelectedGame(field: 'startPlan' | 'closePlan' | 'closeDelay', value: string) {
  if (!selectedGame.value) return;
  await persistSelectedProfile({ [field]: value });
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
    const recoveryWarning = await takeConfigRecoveryWarning(tauriInvoke);
    const config = loadedConfig;
    currentConfig.value = config;
    automatic.value = config.automation.enabled;
    applyConfigProfiles(config);
    selectedId.value = games.value[0]?.id ?? '';
    syncProfilePlanSelections(powerPlans.value);
    if (recoveryWarning) showNotice('info', recoveryWarning);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  }
}

async function persistConfig(config: AppConfig): Promise<ConfigSaveOutcome> {
  return saveAppConfig(tauriInvoke, config);
}

function showSaveWarnings(outcome: ConfigSaveOutcome) {
  if (outcome.warnings.length > 0) {
    showNotice('info', `Cambios guardados. ${outcome.warnings.join(' ')}`);
  }
}

function showSaveOutcome(message: string, outcome: ConfigSaveOutcome) {
  if (outcome.warnings.length > 0) {
    showNotice('info', `${message} ${outcome.warnings.join(' ')}`);
  } else {
    showNotice('success', message);
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
  eventsPanelOpen.value = false;
  await nextTick();
  confirmation.value = {
    title: 'Borrar historial',
    message: 'Se eliminaran los eventos de diagnostico guardados en este equipo.',
    confirmLabel: 'Borrar historial',
    action: performClearEventHistory,
  };
}

async function performClearEventHistory() {
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
    const outcome = await persistConfig(nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = nextConfig.profiles[nextConfig.profiles.length - 1]?.id ?? '';
    syncProfilePlanSelections(powerPlans.value);
    showSaveOutcome('Perfil agregado y listo para detectar.', outcome);
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

async function persistAppSettings(update: AppSettingsUpdate): Promise<ConfigSaveOutcome | null> {
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = updateAppSettingsConfig(config, update);
    const outcome = await persistConfig(nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    automatic.value = nextConfig.automation.enabled;
    showSaveWarnings(outcome);
    return outcome;
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
    return null;
  } finally {
    powerLoading.value = false;
  }
}

async function toggleAutomatic() {
  const enabled = !automatic.value;
  const outcome = await persistAppSettings({ automationEnabled: enabled });
  if (!outcome) return;
  agentStatusText.value = enabled ? 'Agente activo' : 'Automatización pausada';
  if (outcome.warnings.length === 0) {
    showNotice('info', enabled ? 'Automatización activada.' : 'Automatización pausada.');
  }
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

async function installElevatedAgent() {
  powerError.value = '';
  powerLoading.value = true;
  agentSetupLoading.value = true;
  showNotice('info', 'Windows pedira permiso para reparar PowerShift.');
  try {
    await installAgentTask(tauriInvoke);
    agentTaskReady.value = true;
    await refreshAgentSnapshot({ forceLinkedRefresh: true });
    showNotice('success', 'Agente elevado reparado e iniciado.');
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
  } finally {
    agentSetupLoading.value = false;
    powerLoading.value = false;
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
  await refreshOpenProcesses();
}

async function autoDetectProfiles() {
  powerError.value = '';
  processDrawerMode.value = 'candidates';
  processPanelOpen.value = true;
  settingsPanelOpen.value = false;
  eventsPanelOpen.value = false;
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
    const outcome = await persistConfig(nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = nextConfig.profiles[nextConfig.profiles.length - 1]?.id ?? '';
    detectedCandidates.value = detectProfileCandidates(nextConfig, openProcesses.value);
    await refreshProcessIcons();
    syncProfilePlanSelections(powerPlans.value);
    showSaveOutcome('Candidato agregado como perfil.', outcome);
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
    if (nextConfig === config) {
      showNotice('info', 'Ese proceso ya forma parte del perfil.');
      return;
    }
    const outcome = await persistConfig(nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = profileId;
    syncProfilePlanSelections(powerPlans.value);
    showSaveOutcome('Proceso asociado al perfil.', outcome);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function persistSelectedProfile(update: ProfileUpdate): Promise<boolean> {
  if (!selectedGame.value) return false;
  if (typeof update.notify === 'boolean' && !globalNotificationsEnabled.value) return false;

  const profileId = selectedGame.value.id;
  let sourceConfig = currentConfig.value;
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = sourceConfig ?? (await getAppConfig(tauriInvoke));
    sourceConfig = config;
    const nextConfig = updateProfileConfig(config, profileId, update, powerPlans.value);
    const outcome = await persistConfig(nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = profileId;
    syncProfilePlanSelections(powerPlans.value);
    showSaveOutcome('Perfil actualizado.', outcome);
    return true;
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    if (sourceConfig) {
      currentConfig.value = sourceConfig;
      applyConfigProfiles(sourceConfig);
      selectedId.value = profileId;
      syncProfilePlanSelections(powerPlans.value);
    }
    showNotice('error', powerError.value);
    return false;
  } finally {
    powerLoading.value = false;
  }
}

async function deleteProfile(profileId: string) {
  const profile = games.value.find((game) => game.id === profileId);
  if (!profile) return;

  confirmation.value = {
    title: 'Eliminar perfil',
    message: `Se eliminara "${profile.name}" y sus procesos asociados.`,
    confirmLabel: 'Eliminar perfil',
    action: () => performDeleteProfile(profileId),
  };
}

async function performDeleteProfile(profileId: string) {
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = removeProfileFromConfig(config, profileId);
    const outcome = await persistConfig(nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = games.value[0]?.id ?? '';
    syncProfilePlanSelections(powerPlans.value);
    showSaveOutcome('Perfil eliminado.', outcome);
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
    if (nextConfig === config) {
      showNotice('info', 'El proceso ya no estaba asociado.');
      return;
    }
    const outcome = await persistConfig(nextConfig);
    await refreshRecentEvents();
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = profileId;
    syncProfilePlanSelections(powerPlans.value);
    showSaveOutcome('Proceso quitado.', outcome);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  } finally {
    powerLoading.value = false;
  }
}

async function changeAssociatedProcessRole(processName: string, role: AssociatedProcessRole) {
  if (!selectedGame.value) return;

  const profileId = selectedGame.value.id;
  powerError.value = '';
  powerLoading.value = true;
  try {
    const config = currentConfig.value ?? (await getAppConfig(tauriInvoke));
    const nextConfig = updateAssociatedProcessRole(config, profileId, processName, role);
    if (nextConfig === config) return;
    const outcome = await persistConfig(nextConfig);
    currentConfig.value = nextConfig;
    applyConfigProfiles(nextConfig);
    selectedId.value = profileId;
    showSaveOutcome(
      role === 'alternate_trigger'
        ? 'El proceso ahora puede iniciar el perfil.'
        : 'El proceso ahora solo prolonga una sesión iniciada.',
      outcome,
    );
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
  } finally {
    powerLoading.value = false;
  }
}

async function promoteSelectedProfile() {
  if (!selectedGame.value || selectedGame.value.status !== 'active') return;

  const profileName = selectedGame.value.name;
  powerError.value = '';
  powerLoading.value = true;
  try {
    await promoteActiveProfile(tauriInvoke, selectedGame.value.id);
    await refreshAgentSnapshot({ forceLinkedRefresh: true });
    showNotice(
      'success',
      `Traspaso solicitado para ${profileName}; durará mientras el perfil siga activo.`,
    );
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
    showNotice('error', powerError.value);
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

async function refreshProcessIcons() {
  const loadId = ++processIconLoadId;
  const processItems = (processDrawerMode.value === 'candidates' ? [] : openProcesses.value)
    .filter((process) => process.path)
    .slice(0, 80)
    .map((process) => ({ id: processIconMapKey(process), path: process.path ?? '' }));
  const candidateItems =
    processDrawerMode.value === 'candidates'
      ? detectedCandidates.value.slice(0, 80).map((candidate) => ({
          id: candidateIconMapKey(candidate),
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
  if (agentSnapshotInFlight) {
    agentSnapshotPending = true;
    return;
  }

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
    if (agentSnapshotPending) {
      agentSnapshotPending = false;
      void refreshAgentSnapshot();
    }
  }
}

async function subscribeToAgentState() {
  if (!('__TAURI_INTERNALS__' in window)) return;
  try {
    unlistenAgentState = await listen('powershift://agent-state-changed', () => {
      void refreshAgentSnapshot();
    });
  } catch {
    unlistenAgentState = undefined;
  }
}

async function runAgentScanNow() {
  powerError.value = '';
  powerLoading.value = true;
  try {
    await wakeAgent(tauriInvoke);
    showNotice('success', 'Reevaluacion solicitada al agente.');
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

async function confirmPendingAction() {
  const pending = confirmation.value;
  if (!pending) return;
  confirmation.value = null;
  await pending.action();
}

function closeOpenPanel() {
  if (settingsPanelOpen.value) settingsPanelOpen.value = false;
  else if (eventsPanelOpen.value) eventsPanelOpen.value = false;
  else if (processPanelOpen.value) processPanelOpen.value = false;
}

function handleGlobalKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && drawerOpen.value) {
    event.preventDefault();
    closeOpenPanel();
  }
}

watch(
  [processPanelOpen, settingsPanelOpen, eventsPanelOpen],
  async (current, previous) => {
    const isOpen = current.some(Boolean);
    const wasOpen = previous.some(Boolean);
    if (isOpen && !wasOpen) {
      drawerTrigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    }
    await nextTick();
    if (!isOpen && wasOpen) {
      drawerTrigger?.focus();
      drawerTrigger = null;
    }
  },
);

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
  await synchronizeCurrentProfilePriorities();
  await refreshAgentSnapshot();
}

async function synchronizeCurrentProfilePriorities() {
  const config = currentConfig.value;
  if (!config || powerPlans.value.length === 0) return;

  const normalized = normalizeProfilePriorities(config, powerPlans.value);
  if (JSON.stringify(normalized.profiles) === JSON.stringify(config.profiles)) return;

  try {
    const outcome = await persistConfig(normalized);
    currentConfig.value = normalized;
    applyConfigProfiles(normalized);
    showSaveWarnings(outcome);
  } catch (error) {
    powerError.value = error instanceof Error ? error.message : String(error);
  }
}

onMounted(() => {
  window.addEventListener('keydown', handleGlobalKeydown);
  void getVersion()
    .then((version) => {
      appVersion.value = version;
    })
    .catch(() => undefined);
  void subscribeToAgentState();
  void initializeApp();
  agentSnapshotTimer = window.setInterval(() => {
    void refreshAgentSnapshot();
  }, 30_000);
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleGlobalKeydown);
  if (noticeTimer) window.clearTimeout(noticeTimer);
  if (agentSnapshotTimer) window.clearInterval(agentSnapshotTimer);
  unlistenAgentState?.();
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
  <div class="app-frame" :class="{ ready: appReady }" :aria-busy="powerLoading">
    <header class="titlebar" data-tauri-drag-region @pointerdown="startWindowDrag">
      <div class="brand" data-tauri-drag-region>
        <div class="brand-mark">
          <Zap :size="27" stroke-width="2.7" fill="currentColor" />
        </div>
        <span data-tauri-drag-region>PowerShift</span>
      </div>

      <div class="title-status">
        <button
          class="mode-toggle"
          :class="{ enabled: automatic }"
          role="switch"
          :aria-checked="automatic"
          :disabled="powerLoading"
          aria-label="Cambio automatico de planes"
          @click="toggleAutomatic"
        >
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
        <input v-model="query" type="search" placeholder="Buscar juego..." aria-label="Buscar perfil" />
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
            class="game-row"
            :class="{ selected: game.id === selectedId, active: game.status === 'active', disabled: game.status === 'disabled' }"
          >
            <button
              class="game-select"
              :aria-pressed="game.id === selectedId"
              :aria-label="`Seleccionar ${game.name}`"
              @click="selectedId = game.id"
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
            </button>
            <button class="row-action" :disabled="powerLoading" aria-label="Eliminar perfil" @click.stop="deleteProfile(game.id)">
              <Trash2 :size="17" />
            </button>
          </div>
        </div>
      </aside>

      <section class="details-panel">
        <ProfileEditor
          :game="selectedGame"
          :icon="selectedGame ? profileIcons[selectedGame.id] : undefined"
          :busy="powerLoading"
          :power-plan-options="powerPlanOptions"
          :close-delay-options="closeDelayOptions"
          :global-notifications-enabled="globalNotificationsEnabled"
          :can-promote-control="selectedGame?.status === 'active' && controllingGame?.id !== selectedGame.id"
          @add-executable="addExecutableProfile"
          @auto-detect="autoDetectProfiles"
          @update-profile="persistSelectedProfile"
          @update-plan="updateSelectedGame"
          @open-folder="openSelectedExecutableFolder"
          @remove-associated="removeAssociatedProcess"
          @update-associated-role="changeAssociatedProcessRole"
          @associate="openAssociateProcessPanel"
          @test-profile="testSelectedProfile"
          @promote-control="promoteSelectedProfile"
        />
      </section>
    </main>

    <div v-if="drawerOpen" class="drawer-backdrop" aria-hidden="true" @click="closeOpenPanel"></div>

    <ProcessDrawer
      v-if="processPanelOpen"
      :mode="processDrawerMode"
      :processes="openProcesses"
      :candidates="detectedCandidates"
      :icons="processIcons"
      :loading="drawerLoading"
      :busy="powerLoading"
      @close="processPanelOpen = false"
      @add-candidate="addDetectedCandidate"
      @associate="associateOpenProcess"
      @refresh="refreshDrawer"
    />

    <SettingsDrawer
      v-if="settingsPanelOpen && currentConfig"
      :config="currentConfig"
      :agent-task-ready="agentTaskReady"
      :agent-status-text="agentStatusText"
      :agent-status-tone="agentStatusTone"
      :elevated-agent-action-label="elevatedAgentActionLabel"
      :power-loading="powerLoading"
      :agent-setup-loading="agentSetupLoading"
      :app-version="appVersion"
      @close="settingsPanelOpen = false"
      @toggle-automation="toggleAutomatic"
      @update-settings="persistAppSettings"
      @agent-action="handleElevatedAgentAction"
      @open-events="toggleEventsPanel"
      @open-github="openGithubProfile"
    />

    <EventsDrawer
      v-if="eventsPanelOpen"
      :events="recentEvents"
      :loading="powerLoading"
      @close="eventsPanelOpen = false"
      @clear="clearEventHistory"
      @refresh="refreshRecentEvents"
    />

    <footer class="statusbar" aria-live="polite">
      <div class="listener-state" :class="agentStatusTone">
        <Activity :size="18" />
        <span>{{ powerError ? powerError : agentStatusText }}</span>
        <Gauge :size="18" />
        <span class="live-dot"></span>
      </div>
    </footer>

    <div v-if="notice" class="toast" :class="notice.kind" role="status" aria-live="polite">
      <CheckCircle2 :size="17" />
      <span>{{ notice.message }}</span>
    </div>

    <ConfirmDialog
      :open="Boolean(confirmation)"
      :title="confirmation?.title ?? ''"
      :message="confirmation?.message ?? ''"
      :confirm-label="confirmation?.confirmLabel ?? ''"
      @cancel="confirmation = null"
      @confirm="confirmPendingAction"
    />
  </div>
</template>
