import { open } from '@tauri-apps/plugin-dialog';

export type OpenDialogFn = typeof open;

export async function pickExecutable(openDialog: OpenDialogFn = open): Promise<string | null> {
  const selected = await openDialog({
    multiple: false,
    filters: [{ name: 'Ejecutables', extensions: ['exe'] }],
  });

  if (Array.isArray(selected)) {
    return selected[0] ?? null;
  }

  return typeof selected === 'string' ? selected : null;
}
