import { describe, expect, it } from 'vitest';
import appSource from './App.vue?raw';

describe('application version surface', () => {
  it('reads the packaged Tauri version instead of duplicating a literal', () => {
    expect(appSource).toContain("import { getVersion } from '@tauri-apps/api/app'");
    expect(appSource).toContain('getVersion()');
    expect(appSource).toContain('appVersion.value = version');
    expect(appSource).not.toContain("const APP_VERSION = '");
  });
});
