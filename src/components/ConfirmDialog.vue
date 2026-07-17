<script setup lang="ts">
import { nextTick, ref, watch } from 'vue';
import { Trash2, X } from '@lucide/vue';

const props = defineProps<{
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [];
}>();

const dialog = ref<HTMLElement | null>(null);
let returnFocus: HTMLElement | null = null;

watch(
  () => props.open,
  async (open) => {
    if (open) {
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      await nextTick();
      dialog.value?.focus();
    } else {
      await nextTick();
      returnFocus?.focus();
      returnFocus = null;
    }
  },
);

function trapFocus(event: KeyboardEvent) {
  const focusable = Array.from(
    dialog.value?.querySelectorAll<HTMLElement>('button:not(:disabled)') ?? [],
  );
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
  <div v-if="open" class="confirm-backdrop" @click.self="emit('cancel')">
    <section
      ref="dialog"
      class="confirm-dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="confirm-title"
      aria-describedby="confirm-message"
      tabindex="-1"
      @keydown.escape.stop.prevent="emit('cancel')"
      @keydown.tab="trapFocus"
    >
      <header class="confirm-header">
        <strong id="confirm-title">{{ title }}</strong>
        <button class="icon-button" aria-label="Cancelar" @click="emit('cancel')">
          <X :size="18" />
        </button>
      </header>
      <p id="confirm-message">{{ message }}</p>
      <footer class="confirm-actions">
        <button class="secondary-action compact" @click="emit('cancel')">Cancelar</button>
        <button class="danger-action" @click="emit('confirm')">
          <Trash2 :size="16" />
          <span>{{ confirmLabel }}</span>
        </button>
      </footer>
    </section>
  </div>
</template>
