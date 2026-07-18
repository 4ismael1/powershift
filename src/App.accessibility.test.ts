import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const source = readFileSync(fileURLToPath(new URL('./App.vue', import.meta.url)), 'utf8');
const confirmDialogSource = readFileSync(
  fileURLToPath(new URL('./components/ConfirmDialog.vue', import.meta.url)),
  'utf8',
);
const settingsDrawerSource = readFileSync(
  fileURLToPath(new URL('./components/SettingsDrawer.vue', import.meta.url)),
  'utf8',
);
const eventsDrawerSource = readFileSync(
  fileURLToPath(new URL('./components/EventsDrawer.vue', import.meta.url)),
  'utf8',
);
const processDrawerSource = readFileSync(
  fileURLToPath(new URL('./components/ProcessDrawer.vue', import.meta.url)),
  'utf8',
);
const profileEditorSource = readFileSync(
  fileURLToPath(new URL('./components/ProfileEditor.vue', import.meta.url)),
  'utf8',
);
const htmlSource = readFileSync(fileURLToPath(new URL('../index.html', import.meta.url)), 'utf8');
const drawerSources = `${source}\n${profileEditorSource}\n${settingsDrawerSource}\n${eventsDrawerSource}\n${processDrawerSource}`;

describe('PowerShift accessibility contracts', () => {
  it('exposes every toggle with switch semantics and checked state', () => {
    const switchRoles = drawerSources.match(/role="switch"/g) ?? [];
    const checkedStates = drawerSources.match(/:aria-checked=/g) ?? [];

    expect(switchRoles.length).toBeGreaterThanOrEqual(7);
    expect(checkedStates).toHaveLength(switchRoles.length);
  });

  it('treats drawers as modal dialogs with labels and keyboard containment', () => {
    expect(drawerSources.match(/role="dialog"/g)).toHaveLength(3);
    expect(drawerSources.match(/aria-modal="true"/g)).toHaveLength(3);
    expect(drawerSources.match(/@keydown\.tab=/g)).toHaveLength(3);
    expect(source).toContain("event.key === 'Escape'");
    expect(source).toContain('class="drawer-backdrop"');
    expect(settingsDrawerSource).toContain('@keydown.escape.stop.prevent');
    expect(eventsDrawerSource).toContain('@keydown.escape.stop.prevent');
    expect(processDrawerSource).toContain('@keydown.escape.stop.prevent');
    expect(processDrawerSource).toContain('aria-label="Filtrar procesos"');
  });

  it('keeps profile selection and deletion as sibling controls', () => {
    const rowStart = source.indexOf('class="game-row"');
    const rowEnd = source.indexOf('</div>', rowStart);
    const row = source.slice(rowStart, rowEnd);

    expect(row).toContain('class="game-select"');
    expect(row).toContain('class="row-action"');
    expect(row).not.toContain('role="button"');
  });

  it('announces asynchronous status without interrupting the user', () => {
    expect(source).toContain('class="statusbar" aria-live="polite"');
    expect(source).toContain('role="status" aria-live="polite"');
    expect(source).toContain(':aria-busy="powerLoading"');
  });

  it('uses the reusable non-blocking confirmation dialog', () => {
    expect(source).toContain("import ConfirmDialog from '@/components/ConfirmDialog.vue'");
    expect(source).toContain('<ConfirmDialog');
    expect(source).not.toContain('window.confirm');
    expect(confirmDialogSource).toContain('role="alertdialog"');
    expect(confirmDialogSource).toContain('aria-modal="true"');
    expect(confirmDialogSource).toContain('@keydown.escape.stop.prevent');
  });

  it('uses explicit close behavior labels and the shared icon library', () => {
    expect(settingsDrawerSource).toContain('Cerrar ventana; mantener agente');
    expect(settingsDrawerSource).toContain('Salir por completo');
    expect(settingsDrawerSource).toContain('GitFork');
    expect(settingsDrawerSource).not.toContain('<svg');
  });

  it('keeps startup styling compatible with the production CSP', () => {
    expect(htmlSource).toContain('href="/src/theme/style.css"');
    expect(htmlSource).not.toContain('<style>');
    expect(htmlSource).not.toMatch(/style=/);
  });
});
