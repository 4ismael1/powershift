import type { InvokeFn } from './powerApi';

export async function openExecutableFolder(invokeFn: InvokeFn, executablePath: string): Promise<void> {
  return invokeFn<void>('open_executable_folder', { executable_path: executablePath });
}

export async function openExternalUrl(invokeFn: InvokeFn, url: string): Promise<void> {
  return invokeFn<void>('open_external_url', { url });
}
