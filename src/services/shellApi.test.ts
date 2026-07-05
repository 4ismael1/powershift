import { describe, expect, it, vi } from 'vitest';
import { openExecutableFolder } from './shellApi';
import type { InvokeFn } from './powerApi';

describe('shellApi', () => {
  it('calls Tauri command to reveal an executable path', async () => {
    const mockInvoke = vi.fn().mockResolvedValue(undefined);
    const invokeFn = mockInvoke as unknown as InvokeFn;

    await expect(openExecutableFolder(invokeFn, 'C:\\Games\\Apex\\r5apex.exe')).resolves.toBeUndefined();

    expect(mockInvoke).toHaveBeenCalledWith('open_executable_folder', {
      executable_path: 'C:\\Games\\Apex\\r5apex.exe',
    });
  });
});
