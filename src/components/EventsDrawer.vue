<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { RefreshCw, Trash2, X } from '@lucide/vue';
import { formatEventTime, type EventLogEntry } from '@/services/eventsApi';

defineProps<{
  events: EventLogEntry[];
  loading: boolean;
}>();

const emit = defineEmits<{
  close: [];
  clear: [];
  refresh: [];
}>();

const dialog = ref<HTMLElement | null>(null);

onMounted(() => dialog.value?.focus());

function eventKindLabel(kind: string) {
  if (kind === 'profile_activated') return 'Perfil activado';
  if (kind === 'power_plan_restored') return 'Plan restaurado';
  if (kind === 'restore_scheduled') return 'Restauración programada';
  if (kind === 'agent_error') return 'Error del agente';
  return kind.split('_').join(' ');
}

function trapFocus(event: KeyboardEvent) {
  const focusable = Array.from(
    dialog.value?.querySelectorAll<HTMLElement>('button:not(:disabled), [tabindex]:not([tabindex="-1"])') ?? [],
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
    class="events-drawer"
    role="dialog"
    aria-modal="true"
    aria-labelledby="events-drawer-title"
    tabindex="-1"
    @keydown.escape.stop.prevent="emit('close')"
    @keydown.tab="trapFocus"
  >
    <header class="drawer-header">
      <div>
        <strong id="events-drawer-title">Eventos</strong>
        <span>{{ events.length }} recientes</span>
      </div>
      <button class="icon-button" aria-label="Cerrar eventos" @click="emit('close')">
        <X :size="18" />
      </button>
    </header>

    <div class="event-list">
      <div v-if="events.length === 0" class="drawer-empty">
        <strong>Sin eventos</strong>
        <span>El agente registrará cambios de plan, restauraciones y errores aquí.</span>
      </div>
      <div v-for="event in events" :key="`${event.timestamp_ms}-${event.kind}-${event.message}`" class="event-row">
        <span class="event-dot" :class="event.level"></span>
        <span class="event-copy">
          <strong>{{ event.message }}</strong>
          <small>{{ formatEventTime(event.timestamp_ms) }} · {{ eventKindLabel(event.kind) }}</small>
        </span>
      </div>
    </div>

    <footer class="drawer-footer">
      <button class="secondary-action danger" :disabled="loading || events.length === 0" @click="emit('clear')">
        <Trash2 :size="17" />
        <span>Borrar historial</span>
      </button>
      <button class="secondary-action" :disabled="loading" @click="emit('refresh')">
        <RefreshCw :size="17" />
        <span>Refrescar</span>
      </button>
    </footer>
  </section>
</template>
