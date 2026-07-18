// @vitest-environment happy-dom

import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { RESTORE_NOTHING_OPTION, type UiGameProfile } from '@/services/configApi';
import ProfileEditor from './ProfileEditor.vue';

function game(): UiGameProfile {
  return {
    id: 'fortnite',
    name: 'Fortnite',
    exe: 'FortniteClient-Win64-Shipping.exe',
    path: 'C:\\Games\\Fortnite\\FortniteClient-Win64-Shipping.exe',
    iconText: 'FO',
    iconClass: 'custom',
    level: 'high',
    status: 'active',
    enabled: true,
    notify: true,
    startPlan: 'high',
    closePlan: RESTORE_NOTHING_OPTION,
    closeDelay: '30 s',
    associatedProcesses: [{ name: 'chrome.exe', role: 'companion' }],
    lastEvent: 'Activo',
  };
}

function mountEditor(canPromoteControl = true) {
  return mount(ProfileEditor, {
    props: {
      game: game(),
      busy: false,
      powerPlanOptions: [
        { id: 'high', name: 'Alto rendimiento' },
        { id: 'balanced', name: 'Equilibrado' },
      ],
      closeDelayOptions: ['0 s', '30 s'],
      globalNotificationsEnabled: true,
      canPromoteControl,
    },
  });
}

describe('ProfileEditor', () => {
  it('renders the restore-do-nothing option and emits plan edits', async () => {
    const wrapper = mountEditor();
    const selects = wrapper.findAll('select');

    expect(selects[1].find(`option[value="${RESTORE_NOTHING_OPTION}"]`).exists()).toBe(true);
    await selects[0].setValue('balanced');

    expect(wrapper.emitted('updatePlan')).toEqual([['startPlan', 'balanced']]);
  });

  it('explains and toggles an associated process role', async () => {
    const wrapper = mountEditor();
    const roleButton = wrapper.get('button[aria-label="Cambiar función de chrome.exe"]');

    expect(wrapper.text()).toContain('Un compañero solo la prolonga');
    expect(roleButton.text()).toContain('Compañero');
    await roleButton.trigger('click');

    expect(wrapper.emitted('updateAssociatedRole')).toEqual([
      ['chrome.exe', 'alternate_trigger'],
    ]);
  });

  it('offers an explicit temporary handoff only when another profile controls', async () => {
    const wrapper = mountEditor();
    const handoff = wrapper.get('button[title*="traspaso dura"]');

    expect(handoff.text()).toContain('Tomar control ahora');
    await handoff.trigger('click');
    expect(wrapper.emitted('promoteControl')).toHaveLength(1);

    await wrapper.setProps({ canPromoteControl: false });
    expect(wrapper.find('button[title*="traspaso dura"]').exists()).toBe(false);
  });
});
