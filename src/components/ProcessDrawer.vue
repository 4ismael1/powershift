<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { Cpu, RefreshCw, Search, X } from '@lucide/vue';
import type { ProfileCandidate } from '@/services/autoDetect';
import { candidateIconMapKey, processIconMapKey, type IconMap } from '@/services/iconApi';
import { filterProcesses, type ProcessInfo } from '@/services/processApi';

export type ProcessDrawerMode = 'processes' | 'candidates' | 'associate';

const props = defineProps<{
  mode: ProcessDrawerMode;
  processes: ProcessInfo[];
  candidates: ProfileCandidate[];
  icons: IconMap;
  loading: boolean;
  busy: boolean;
}>();

const emit = defineEmits<{
  close: [];
  addCandidate: [candidate: ProfileCandidate];
  associate: [process: ProcessInfo];
  refresh: [];
}>();

const dialog = ref<HTMLElement | null>(null);
const query = ref('');

const title = computed(() => {
  if (props.mode === 'candidates') return 'Auto detectar';
  if (props.mode === 'associate') return 'Asociar proceso';
  return 'Procesos abiertos';
});
const count = computed(() =>
  props.mode === 'candidates' ? props.candidates.length : props.processes.length,
);
const visibleProcesses = computed(() => filterProcesses(props.processes, query.value));
const visibleCandidates = computed(() => {
  const value = query.value.trim().toLowerCase();
  if (!value) return props.candidates;
  return props.candidates.filter((candidate) =>
    `${candidate.name} ${candidate.executableName} ${candidate.executablePath} ${candidate.pid}`
      .toLowerCase()
      .includes(value),
  );
});

onMounted(() => dialog.value?.focus());
watch(
  () => props.mode,
  async () => {
    query.value = '';
    await nextTick();
    dialog.value?.focus();
  },
);

function trapFocus(event: KeyboardEvent) {
  const focusable = Array.from(
    dialog.value?.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])',
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
    class="process-drawer"
    role="dialog"
    aria-modal="true"
    aria-labelledby="process-drawer-title"
    tabindex="-1"
    @keydown.escape.stop.prevent="emit('close')"
    @keydown.tab="trapFocus"
  >
    <header class="drawer-header">
      <div>
        <strong id="process-drawer-title">{{ title }}</strong>
        <span>{{ count }} detectados</span>
      </div>
      <button class="icon-button" aria-label="Cerrar procesos abiertos" @click="emit('close')">
        <X :size="18" />
      </button>
    </header>

    <label class="drawer-search">
      <Search :size="18" />
      <input v-model="query" type="search" placeholder="Filtrar..." aria-label="Filtrar procesos" />
    </label>

    <div class="drawer-list">
      <template v-if="mode === 'candidates'">
        <div v-if="loading" class="drawer-empty">
          <strong>Buscando procesos</strong>
          <span>Detectando candidatos abiertos...</span>
        </div>
        <div v-else-if="visibleCandidates.length === 0" class="drawer-empty">
          <strong>Sin candidatos</strong>
          <span>Abre un juego y vuelve a ejecutar la deteccion.</span>
        </div>
        <div v-for="candidate in loading ? [] : visibleCandidates" :key="candidate.id" class="open-process-row candidate-row">
          <span class="drawer-app-icon">
            <img v-if="icons[candidateIconMapKey(candidate)]" :src="icons[candidateIconMapKey(candidate)]" alt="" />
            <Cpu v-else :size="15" />
          </span>
          <span>
            <strong>{{ candidate.name }}</strong>
            <small :title="candidate.executablePath">{{ candidate.executablePath }}</small>
          </span>
          <button class="mini-add" :disabled="busy" @click="emit('addCandidate', candidate)">Agregar</button>
        </div>
      </template>

      <template v-else>
        <div v-if="loading" class="drawer-empty">
          <strong>Leyendo procesos</strong>
          <span>Actualizando lista...</span>
        </div>
        <div v-else-if="visibleProcesses.length === 0" class="drawer-empty">
          <strong>Sin procesos</strong>
          <span>No hay coincidencias para este filtro.</span>
        </div>
        <div v-for="process in loading ? [] : visibleProcesses" :key="`${process.pid}-${process.name}`" class="open-process-row">
          <span class="drawer-app-icon">
            <img v-if="icons[processIconMapKey(process)]" :src="icons[processIconMapKey(process)]" alt="" />
            <Cpu v-else :size="15" />
          </span>
          <span>
            <strong>{{ process.name }}</strong>
            <small :title="process.path ?? `PID ${process.pid}`">{{ process.path ?? `PID ${process.pid}` }}</small>
          </span>
          <button
            v-if="mode === 'associate'"
            class="mini-add"
            :disabled="busy"
            @click="emit('associate', process)"
          >
            Asociar
          </button>
          <small v-else>{{ process.pid }}</small>
        </div>
      </template>
    </div>

    <footer class="drawer-footer">
      <button class="secondary-action" :disabled="busy || loading" @click="emit('refresh')">
        <RefreshCw :size="17" />
        <span>{{ loading ? 'Actualizando...' : 'Refrescar' }}</span>
      </button>
    </footer>
  </section>
</template>
