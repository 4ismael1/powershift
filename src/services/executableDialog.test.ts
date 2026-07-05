import { describe, expect, it, vi } from 'vitest';
import { pickExecutable, type OpenDialogFn } from './executableDialog';

describe('executableDialog', () => {
  it('opens a native dialog restricted to executable files', async () => {
    const openDialog = vi.fn().mockResolvedValue('C:\\Games\\Demo\\demo.exe') as unknown as OpenDialogFn;

    const result = await pickExecutable(openDialog);

    expect(openDialog).toHaveBeenCalledWith({
      multiple: false,
      filters: [{ name: 'Ejecutables', extensions: ['exe'] }],
    });
    expect(result).toBe('C:\\Games\\Demo\\demo.exe');
  });

  it('returns null when the user cancels', async () => {
    const openDialog = vi.fn().mockResolvedValue(null) as unknown as OpenDialogFn;

    await expect(pickExecutable(openDialog)).resolves.toBeNull();
  });

  it('uses the first selected path if a dialog implementation returns an array', async () => {
    const openDialog = vi.fn().mockResolvedValue(['C:\\Games\\A\\a.exe']) as unknown as OpenDialogFn;

    await expect(pickExecutable(openDialog)).resolves.toBe('C:\\Games\\A\\a.exe');
  });
});
