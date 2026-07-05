import { beforeEach, describe, expect, it, vi } from 'vitest';
import { clearIconCache, getExecutableIcon, loadProfileIcons } from './iconApi';
import type { InvokeFn } from './powerApi';

describe('iconApi', () => {
  beforeEach(() => {
    clearIconCache();
  });

  it('calls Tauri command with the executable path', async () => {
    const invoke = vi.fn().mockResolvedValue('data:image/png;base64,AQID') as unknown as InvokeFn;

    const result = await getExecutableIcon(invoke, 'C:\\Games\\Game\\game.exe');

    expect(result).toBe('data:image/png;base64,AQID');
    expect(invoke).toHaveBeenCalledWith('get_executable_icon', {
      executable_path: 'C:\\Games\\Game\\game.exe',
    });
  });

  it('loads icons keyed by profile id and ignores missing paths or failures', async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce('data:image/png;base64,AAAA')
      .mockRejectedValueOnce(new Error('missing icon')) as unknown as InvokeFn;

    const icons = await loadProfileIcons(invoke, [
      { id: 'game', path: 'C:\\Games\\game.exe' },
      { id: 'empty', path: '   ' },
      { id: 'broken', path: 'C:\\Broken\\broken.exe' },
    ]);

    expect(icons).toEqual({ game: 'data:image/png;base64,AAAA' });
  });

  it('deduplicates icon extraction by normalized executable path', async () => {
    const invoke = vi.fn().mockResolvedValue('data:image/png;base64,ICON') as unknown as InvokeFn;

    const icons = await loadProfileIcons(invoke, [
      { id: 'one', path: 'C:\\Games\\Game\\game.exe' },
      { id: 'two', path: 'c:/games/game/GAME.exe' },
    ]);

    expect(icons).toEqual({
      one: 'data:image/png;base64,ICON',
      two: 'data:image/png;base64,ICON',
    });
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it('does not cache failed icon extractions', async () => {
    const invoke = vi
      .fn()
      .mockRejectedValueOnce(new Error('locked'))
      .mockResolvedValueOnce('data:image/png;base64,OK') as unknown as InvokeFn;

    await expect(loadProfileIcons(invoke, [{ id: 'first', path: 'C:\\Games\\game.exe' }])).resolves.toEqual({});
    await expect(loadProfileIcons(invoke, [{ id: 'second', path: 'C:\\Games\\game.exe' }])).resolves.toEqual({
      second: 'data:image/png;base64,OK',
    });
    expect(invoke).toHaveBeenCalledTimes(2);
  });
});
