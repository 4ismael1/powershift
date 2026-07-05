import type { InvokeFn } from './powerApi';

export type IconMap = Record<string, string>;

const iconCache = new Map<string, Promise<string | null>>();

export async function getExecutableIcon(invokeFn: InvokeFn, executablePath: string): Promise<string | null> {
  return invokeFn<string | null>('get_executable_icon', { executable_path: executablePath });
}

export function clearIconCache(): void {
  iconCache.clear();
}

export async function loadProfileIcons<T extends { id: string; path: string }>(
  invokeFn: InvokeFn,
  profiles: T[],
): Promise<IconMap> {
  const entries = await Promise.all(
    profiles
      .filter((profile) => profile.path.trim().length > 0)
      .map(async (profile) => {
        const icon = await loadExecutableIcon(invokeFn, profile.path);
        return icon ? ([profile.id, icon] as const) : null;
      }),
  );

  return Object.fromEntries(entries.filter((entry): entry is readonly [string, string] => Boolean(entry)));
}

async function loadExecutableIcon(invokeFn: InvokeFn, executablePath: string): Promise<string | null> {
  const key = normalizedPathKey(executablePath);
  const cached = iconCache.get(key);
  if (cached) return cached;

  const request = getExecutableIcon(invokeFn, executablePath).catch(() => null);
  iconCache.set(key, request);

  const icon = await request;
  if (!icon) iconCache.delete(key);
  return icon;
}

function normalizedPathKey(path: string): string {
  return path.trim().replace(/\//g, '\\').toLowerCase();
}
