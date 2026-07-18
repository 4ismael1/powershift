<script setup lang="ts">
import { Cpu, FilePlus2, Folder, Play, Plus, Sparkles, Trash2, Zap } from '@lucide/vue';
import {
  RESTORE_NOTHING_OPTION,
  RESTORE_PREVIOUS_OPTION,
  type AssociatedProcessRole,
  type ProfileUpdate,
  type UiGameProfile,
} from '@/services/configApi';
import type { PowerPlan } from '@/services/powerApi';

defineProps<{
  game: UiGameProfile | null;
  icon?: string;
  busy: boolean;
  powerPlanOptions: PowerPlan[];
  closeDelayOptions: string[];
  globalNotificationsEnabled: boolean;
  canPromoteControl: boolean;
}>();

const emit = defineEmits<{
  addExecutable: [];
  autoDetect: [];
  updateProfile: [update: ProfileUpdate];
  updatePlan: [field: 'startPlan' | 'closePlan' | 'closeDelay', value: string];
  openFolder: [];
  removeAssociated: [processName: string];
  updateAssociatedRole: [processName: string, role: AssociatedProcessRole];
  associate: [];
  testProfile: [];
  promoteControl: [];
}>();

function nextRole(role: AssociatedProcessRole): AssociatedProcessRole {
  return role === 'companion' ? 'alternate_trigger' : 'companion';
}

function roleLabel(role: AssociatedProcessRole): string {
  return role === 'alternate_trigger' ? 'Disparador' : 'Compañero';
}
</script>

<template>
  <div v-if="!game" class="details-empty-state">
    <div class="brand-mark">
      <Zap :size="32" stroke-width="2.7" fill="currentColor" />
    </div>
    <strong>No hay perfil seleccionado</strong>
    <span>Agrega un ejecutable o detecta una aplicación abierta.</span>
    <div class="empty-actions">
      <button class="primary-action compact" :disabled="busy" @click="emit('addExecutable')">
        <FilePlus2 :size="17" />
        <span>Agregar exe</span>
      </button>
      <button class="secondary-action compact" :disabled="busy" @click="emit('autoDetect')">
        <Sparkles :size="17" />
        <span>Auto detectar</span>
      </button>
    </div>
  </div>

  <template v-else>
    <div class="profile-header">
      <span class="selected-art" :class="{ [game.iconClass]: !icon }">
        <img v-if="icon" :src="icon" alt="" />
        <template v-else>{{ game.iconText }}</template>
      </span>
      <div class="identity-fields">
        <label>
          <span>Nombre del juego</span>
          <input
            :value="game.name"
            :disabled="busy"
            @change="emit('updateProfile', { name: ($event.target as HTMLInputElement).value })"
          />
        </label>
        <label>
          <span>Ejecutable</span>
          <div class="path-field">
            <input :value="game.path" :title="game.path" readonly />
            <button
              class="square-tool"
              :disabled="busy"
              aria-label="Abrir carpeta del ejecutable"
              @click="emit('openFolder')"
            >
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
            :class="{ on: game.enabled }"
            role="switch"
            :aria-checked="game.enabled"
            :disabled="busy"
            aria-label="Activar perfil"
            @click="emit('updateProfile', { enabled: !game.enabled })"
          >
            <span></span>
          </button>
        </div>
        <label class="select-field">
          <span>Plan al iniciar</span>
          <select
            :value="game.startPlan"
            :disabled="busy"
            @change="emit('updatePlan', 'startPlan', ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="plan in powerPlanOptions" :key="`start-${plan.id}`" :value="plan.id">
              {{ plan.name }}
            </option>
          </select>
        </label>
        <label class="select-field">
          <span>Al cerrar</span>
          <select
            :value="game.closePlan"
            :disabled="busy"
            @change="emit('updatePlan', 'closePlan', ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="plan in powerPlanOptions" :key="`close-${plan.id}`" :value="plan.id">
              {{ plan.name }}
            </option>
            <option :value="RESTORE_PREVIOUS_OPTION">Restaurar plan anterior</option>
            <option :value="RESTORE_NOTHING_OPTION">No cambiar el plan</option>
          </select>
        </label>
        <label class="select-field">
          <span>Retardo al cerrar</span>
          <select
            :value="game.closeDelay"
            :disabled="busy"
            @change="emit('updatePlan', 'closeDelay', ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="delay in closeDelayOptions" :key="delay" :value="delay">
              {{ delay === '0 s' ? 'Sin retardo' : delay }}
            </option>
          </select>
        </label>
      </div>

      <div class="process-column">
        <div class="setting-row compact">
          <span>Mostrar notificación</span>
          <button
            class="switch"
            :class="{ on: game.notify && globalNotificationsEnabled }"
            :disabled="!globalNotificationsEnabled || busy"
            role="switch"
            :aria-checked="game.notify && globalNotificationsEnabled"
            aria-label="Mostrar notificación"
            @click="emit('updateProfile', { notify: !game.notify })"
          >
            <span></span>
          </button>
        </div>
        <div class="process-list">
          <div class="process-title">
            <span>Procesos del perfil</span>
            <small>
              El principal inicia la sesión. Un compañero solo la prolonga; un disparador también puede iniciarla.
            </small>
          </div>
          <div class="process-row primary-process-row">
            <Cpu :size="15" />
            <span>{{ game.exe }}</span>
            <small class="process-role">Principal</small>
          </div>
          <div
            v-for="process in game.associatedProcesses"
            :key="process.name"
            class="process-row associated-process-row"
          >
            <Cpu :size="15" />
            <span>{{ process.name }}</span>
            <button
              class="process-role-toggle"
              :disabled="busy"
              :aria-label="`Cambiar función de ${process.name}`"
              :title="process.role === 'companion' ? 'No inicia el perfil por sí solo' : 'Puede iniciar el perfil sin el principal'"
              @click="emit('updateAssociatedRole', process.name, nextRole(process.role))"
            >
              {{ roleLabel(process.role) }}
            </button>
            <button
              class="process-remove"
              :disabled="busy"
              :aria-label="`Quitar ${process.name}`"
              @click="emit('removeAssociated', process.name)"
            >
              <Trash2 :size="15" />
            </button>
          </div>
        </div>
        <button class="add-process" :disabled="busy" @click="emit('associate')">
          <Plus :size="18" />
          <span>Agregar proceso</span>
        </button>
        <button
          v-if="canPromoteControl"
          class="secondary-action control-handoff-button"
          :disabled="busy"
          title="El traspaso dura hasta que este perfil deje de estar activo"
          @click="emit('promoteControl')"
        >
          Tomar control ahora
        </button>
        <button class="test-button profile-test-button" :disabled="busy" @click="emit('testProfile')">
          <Play :size="15" fill="currentColor" />
          <span>{{ busy ? 'Aplicando...' : 'Probar perfil' }}</span>
        </button>
      </div>
    </div>
  </template>
</template>
