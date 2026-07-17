<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { GitFork, List, Play, RefreshCw, X } from '@lucide/vue';
import type { AgentStateTone } from '@/services/agentApi';
import type { AppConfig, AppSettingsUpdate } from '@/services/configApi';

defineProps<{
  config: AppConfig;
  agentTaskReady: boolean;
  agentStatusText: string;
  agentStatusTone: AgentStateTone;
  elevatedAgentActionLabel: string;
  powerLoading: boolean;
  agentSetupLoading: boolean;
  appVersion: string;
}>();

const emit = defineEmits<{
  close: [];
  toggleAutomation: [];
  updateSettings: [update: AppSettingsUpdate];
  agentAction: [];
  openEvents: [];
  openGithub: [];
}>();

const dialog = ref<HTMLElement | null>(null);

onMounted(() => dialog.value?.focus());

function trapFocus(event: KeyboardEvent) {
  const focusable = Array.from(
    dialog.value?.querySelectorAll<HTMLElement>(
      'button:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])',
    ) ?? [],
  ).filter((element) => element.offsetParent !== null);
  if (focusable.length === 0) return;

  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}
</script>

<template>
  <section
    ref="dialog"
    class="settings-drawer"
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-drawer-title"
    tabindex="-1"
    @keydown.escape.stop.prevent="emit('close')"
    @keydown.tab="trapFocus"
  >
    <header class="drawer-header">
      <div>
        <strong id="settings-drawer-title">Configuración</strong>
        <span>Preferencias generales</span>
      </div>
      <button class="icon-button" aria-label="Cerrar configuración" @click="emit('close')">
        <X :size="18" />
      </button>
    </header>

    <div class="settings-list">
      <div class="setting-line">
        <span>
          <strong>Automatización</strong>
          <small>Cambiar planes automáticamente</small>
        </span>
        <button
          class="switch"
          :class="{ on: config.automation.enabled }"
          role="switch"
          :aria-checked="config.automation.enabled"
          :disabled="powerLoading"
          aria-label="Automatización"
          @click="emit('toggleAutomation')"
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
          :class="{ on: config.automation.notifications_enabled }"
          role="switch"
          :aria-checked="config.automation.notifications_enabled"
          :disabled="powerLoading"
          aria-label="Notificaciones generales"
          @click="emit('updateSettings', { notificationsEnabled: !config.automation.notifications_enabled })"
        >
          <span></span>
        </button>
      </div>

      <div class="setting-line">
        <span>
          <strong>Iniciar con Windows</strong>
          <small>Arranca el agente y la bandeja al iniciar sesión</small>
        </span>
        <button
          class="switch"
          :class="{ on: config.agent.start_with_windows }"
          role="switch"
          :aria-checked="config.agent.start_with_windows"
          :disabled="powerLoading"
          aria-label="Iniciar con Windows"
          @click="emit('updateSettings', { startWithWindows: !config.agent.start_with_windows })"
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
          :class="{ on: config.agent.start_minimized && config.agent.start_with_windows }"
          :disabled="!config.agent.start_with_windows || powerLoading"
          role="switch"
          :aria-checked="config.agent.start_minimized && config.agent.start_with_windows"
          aria-label="Iniciar en segundo plano"
          @click="emit('updateSettings', { startMinimized: !config.agent.start_minimized })"
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
          :class="{ on: config.agent.show_tray_icon }"
          role="switch"
          :aria-checked="config.agent.show_tray_icon"
          :disabled="powerLoading"
          aria-label="Icono en bandeja"
          @click="emit('updateSettings', { showTrayIcon: !config.agent.show_tray_icon })"
        >
          <span></span>
        </button>
      </div>

      <div class="setting-line">
        <span>
          <strong>Agente elevado</strong>
          <small>{{ agentTaskReady ? agentStatusText : 'Requerido para eventos WMI de procesos' }}</small>
        </span>
        <button
          class="secondary-action compact"
          :disabled="powerLoading || agentSetupLoading"
          @click="emit('agentAction')"
        >
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
        <button class="secondary-action compact" :disabled="powerLoading" @click="emit('openEvents')">
          <List :size="16" />
          <span>Abrir</span>
        </button>
      </div>

      <label class="select-field">
        <span>Al pulsar el botón X</span>
        <select
          :value="config.ui.close_button_behavior"
          :disabled="powerLoading"
          @change="emit('updateSettings', { closeButtonBehavior: ($event.target as HTMLSelectElement).value })"
        >
          <option value="hide_window">Cerrar ventana; mantener agente</option>
          <option value="exit_app">Salir por completo</option>
        </select>
      </label>

      <div class="settings-about">
        <span>PowerShift{{ appVersion ? ` v${appVersion}` : '' }}</span>
        <button class="secondary-action compact" @click="emit('openGithub')">
          <GitFork :size="16" />
          <span>GitHub</span>
        </button>
      </div>
    </div>
  </section>
</template>
