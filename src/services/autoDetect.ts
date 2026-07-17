import type { AppConfig } from './configApi';
import type { ProcessInfo } from './processApi';

export interface ProfileCandidate {
  id: string;
  name: string;
  executablePath: string;
  executableName: string;
  pid: number;
  reason: string;
  score: number;
}

const blockedNames = new Set([
  'applicationframehost.exe',
  'cmd.exe',
  'conhost.exe',
  'codex.exe',
  'dwm.exe',
  'explorer.exe',
  'powershell.exe',
  'powershift.exe',
  'powershift-agent.exe',
  'powershift-tray.exe',
  'runtimebroker.exe',
  'searchhost.exe',
  'shellexperiencehost.exe',
  'startmenuexperiencehost.exe',
  'taskhostw.exe',
  'textinputhost.exe',
  'windowsterminal.exe',
]);

export function detectProfileCandidates(config: AppConfig, processes: ProcessInfo[]): ProfileCandidate[] {
  const configured = configuredExecutableKeys(config);
  const byPath = new Map<string, ProfileCandidate>();

  for (const process of processes) {
    const executablePath = process.path?.trim();
    const executableName = process.name.trim();
    if (!executablePath || !executableName.toLowerCase().endsWith('.exe')) continue;
    if (blockedNames.has(executableName.toLowerCase())) continue;
    if (isSystemPath(executablePath)) continue;
    if (configured.has(executablePath.toLowerCase()) || configured.has(executableName.toLowerCase())) continue;

    const score = candidateScore(executablePath);
    const candidate: ProfileCandidate = {
      id: `${executableName.toLowerCase()}-${process.pid}`,
      name: executableName.replace(/\.exe$/i, ''),
      executablePath,
      executableName,
      pid: process.pid,
      reason: score >= 80 ? 'Ruta de juego detectada' : 'Proceso ejecutable abierto',
      score,
    };

    const key = executablePath.toLowerCase();
    const previous = byPath.get(key);
    if (!previous || candidate.score > previous.score) {
      byPath.set(key, candidate);
    }
  }

  return Array.from(byPath.values()).sort((left, right) =>
    right.score - left.score || left.name.localeCompare(right.name),
  );
}

function configuredExecutableKeys(config: AppConfig): Set<string> {
  const values = new Set<string>();
  for (const profile of config.profiles) {
    values.add(profile.main_executable.name.toLowerCase());
    if (profile.main_executable.path) values.add(profile.main_executable.path.toLowerCase());
  }
  return values;
}

function isSystemPath(path: string): boolean {
  const value = path.toLowerCase().replace(/\//g, '\\');
  return value.includes('\\windows\\') || value.includes('\\windowsapps\\');
}

function candidateScore(path: string): number {
  const value = path.toLowerCase().replace(/\//g, '\\');
  if (value.includes('\\steamapps\\common\\')) return 100;
  if (value.includes('\\epic games\\')) return 95;
  if (value.includes('\\xboxgames\\')) return 92;
  if (value.includes('\\riot games\\')) return 90;
  if (value.includes('\\games\\')) return 85;
  if (value.includes('\\program files\\')) return 65;
  return 50;
}
