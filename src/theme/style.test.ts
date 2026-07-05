// @ts-nocheck

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const style = readFileSync(fileURLToPath(new URL('./style.css', import.meta.url)), 'utf8');

function cssRule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return style.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))?.[1] ?? '';
}

describe('PowerShift layout polish', () => {
  it('keeps profile controls visible without a footer scroll trap', () => {
    const profileColumnRule = cssRule('.profile-column,\n.process-column');

    expect(style).toContain('grid-template-rows: auto minmax(0, 1fr);');
    expect(style).toContain('.profile-test-button');
    expect(style).not.toContain('.profile-footer');
    expect(profileColumnRule).toContain('overflow: hidden;');
    expect(profileColumnRule).not.toContain('overflow-y: auto;');
    expect(style).toContain('max-height: 104px;');
  });

  it('keeps command actions inside the compact app width', () => {
    expect(style).toContain('grid-template-columns: minmax(180px, 1fr) max-content max-content max-content;');
    expect(style).toContain('.search-box {\n  min-width: 0;');
  });

  it('keeps the app name visually attached to the logo', () => {
    const brandRule = cssRule('.brand');

    expect(brandRule).toContain('gap: 8px;');
    expect(brandRule).toContain('align-items: center;');
  });

  it('keeps the configured game list readable at the bottom of the scroll area', () => {
    const gameListRule = cssRule('.game-list');

    expect(gameListRule).toContain('padding: 8px 8px 14px;');
    expect(gameListRule).toContain('scrollbar-gutter: stable;');
    expect(gameListRule).toContain('scroll-padding: 8px 0 14px;');
  });

  it('keeps game row metadata away from the delete action', () => {
    const gameRowRule = cssRule('.game-row');
    const rowActionRule = cssRule('.row-action');
    const levelBadgeRule = cssRule('.level-badge');

    expect(gameRowRule).toContain('grid-template-columns: 8px 58px minmax(0, 1fr) max-content 32px;');
    expect(rowActionRule).toContain('justify-self: end;');
    expect(levelBadgeRule).toContain('width: max-content;');
    expect(levelBadgeRule).toContain('max-width: none;');
  });

  it('keeps the settings drawer inside the compact window', () => {
    const settingsDrawerRule = cssRule('.settings-drawer');
    const settingsListRule = cssRule('.settings-list');

    expect(settingsDrawerRule).toContain('max-height: calc(100vh - 92px);');
    expect(settingsDrawerRule).toContain('grid-template-rows: 58px minmax(0, 1fr);');
    expect(settingsListRule).toContain('min-height: 0;');
    expect(settingsListRule).toContain('overflow-y: auto;');
  });

});
