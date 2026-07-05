import type { UiGameProfile } from './configApi';

export type GameSortMode = 'configured' | 'active' | 'name';

export function nextGameSortMode(mode: GameSortMode): GameSortMode {
  if (mode === 'configured') return 'active';
  if (mode === 'active') return 'name';
  return 'configured';
}

export function gameSortModeLabel(mode: GameSortMode): string {
  if (mode === 'active') return 'Activos primero';
  if (mode === 'name') return 'Nombre';
  return 'Orden original';
}

export function sortGames(games: UiGameProfile[], mode: GameSortMode): UiGameProfile[] {
  const indexed = games.map((game, index) => ({ game, index }));
  if (mode === 'active') {
    return indexed
      .sort((left, right) => {
        const status = statusRank(left.game) - statusRank(right.game);
        if (status !== 0) return status;
        return left.index - right.index;
      })
      .map((item) => item.game);
  }

  if (mode === 'name') {
    return indexed
      .sort((left, right) => {
        const name = left.game.name.localeCompare(right.game.name, undefined, { sensitivity: 'base' });
        if (name !== 0) return name;
        return left.index - right.index;
      })
      .map((item) => item.game);
  }

  return games;
}

function statusRank(game: UiGameProfile): number {
  if (game.status === 'active') return 0;
  if (game.status === 'inactive') return 1;
  return 2;
}
