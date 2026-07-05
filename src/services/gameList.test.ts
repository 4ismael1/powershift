import { describe, expect, it } from 'vitest';
import { gameSortModeLabel, nextGameSortMode, sortGames, type GameSortMode } from './gameList';
import type { UiGameProfile } from './configApi';

function game(name: string, status: UiGameProfile['status'] = 'inactive'): UiGameProfile {
  return {
    id: name.toLowerCase(),
    name,
    exe: `${name}.exe`,
    path: `C:\\Games\\${name}\\${name}.exe`,
    iconText: name.slice(0, 2).toUpperCase(),
    iconClass: 'custom',
    level: 'high',
    status,
    enabled: status !== 'disabled',
    notify: true,
    startPlan: 'high',
    closePlan: 'Restaurar plan anterior',
    closeDelay: '30 s',
    processes: [`${name}.exe`],
    lastEvent: status,
  };
}

describe('gameList', () => {
  it('cycles list sort modes', () => {
    const sequence: GameSortMode[] = ['configured', 'active', 'name'];

    expect(sequence.map(nextGameSortMode)).toEqual(['active', 'name', 'configured']);
  });

  it('labels sort modes for the UI', () => {
    expect(gameSortModeLabel('configured')).toBe('Orden original');
    expect(gameSortModeLabel('active')).toBe('Activos primero');
    expect(gameSortModeLabel('name')).toBe('Nombre');
  });

  it('sorts active games first without losing configured order inside groups', () => {
    const games = [game('Chrome'), game('Apex', 'active'), game('Node', 'disabled')];

    expect(sortGames(games, 'active').map((item) => item.name)).toEqual(['Apex', 'Chrome', 'Node']);
  });

  it('sorts by display name', () => {
    const games = [game('Node'), game('apex'), game('Chrome')];

    expect(sortGames(games, 'name').map((item) => item.name)).toEqual(['apex', 'Chrome', 'Node']);
  });
});
