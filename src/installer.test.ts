import { describe, expect, it } from 'vitest';
import installerHooks from '../src-tauri/nsis/powershift-hooks.nsh?raw';
import tauriConfigRaw from '../src-tauri/tauri.conf.json?raw';

const tauriConfig = JSON.parse(tauriConfigRaw);

function macroBody(name: string): string {
  const match = installerHooks.match(new RegExp(`!macro ${name}([\\s\\S]*?)!macroend`));
  return match?.[1] ?? '';
}

describe('installer background architecture', () => {
  it('bundles the agent and tray next to the Tauri host', () => {
    expect(tauriConfig.bundle.resources).toMatchObject({
      '../target/release/powershift-agent.exe': 'powershift-agent.exe',
      '../target/release/powershift-tray.exe': 'powershift-tray.exe',
    });
  });

  it('uses NSIS hooks in an elevated installer', () => {
    expect(tauriConfig.bundle.windows.nsis).toMatchObject({
      installMode: 'perMachine',
      installerHooks: 'nsis/powershift-hooks.nsh',
    });
  });

  it('syncs background components on install and cleans them on uninstall', () => {
    const postInstall = macroBody('NSIS_HOOK_POSTINSTALL');
    const preUninstall = macroBody('NSIS_HOOK_PREUNINSTALL');

    expect(installerHooks).toContain('NSIS_HOOK_POSTINSTALL');
    expect(installerHooks).toContain('NSIS_HOOK_PREUNINSTALL');
    expect(installerHooks).toContain('PowerShiftReadExistingConfigFlags');
    expect(installerHooks).toContain('"start_with_windows": false');
    expect(installerHooks).toContain('"show_tray_icon": false');
    expect(postInstall).toContain('New-ScheduledTaskAction -Execute $$agentPath');
    expect(postInstall).toContain("Register-ScheduledTask -TaskName 'PowerShiftAgent'");
    expect(postInstall).toContain('-AllowStartIfOnBatteries');
    expect(postInstall).toContain('-DontStopIfGoingOnBatteries');
    expect(postInstall).toContain('-RunLevel Highest');
    expect(postInstall).toContain('-ExecutionTimeLimit ([TimeSpan]::Zero)');
    expect(postInstall).toContain('-RestartCount 3');
    expect(postInstall).toContain('-RestartInterval (New-TimeSpan -Minutes 1)');
    expect(postInstall).not.toContain('New-TimeSpan -Days');
    expect(postInstall).not.toContain('schtasks /Create');
    expect(postInstall).not.toContain('/TR');
    expect(postInstall).toContain('schtasks /Run /TN "PowerShiftAgent"');
    expect(postInstall).toContain('$trigger.Enabled = $$false');
    expect(postInstall).not.toContain('schtasks /End /TN "PowerShiftAgent"');
    expect(postInstall).not.toContain('schtasks /Delete /F /TN "PowerShiftAgent"');
    expect(installerHooks).toContain('powershift-agent.exe');
    expect(installerHooks).toContain('PowerShiftTray');
    expect(installerHooks).toContain('powershift-tray.exe');
    expect(installerHooks).toContain('nsis_tauri_utils::RunAsUser');
    expect(postInstall).toContain('Delete "$APPDATA\\PowerShift\\agent-state.json"');
    expect(postInstall).toContain('Delete "$APPDATA\\PowerShift\\agent-control.token"');
    expect(postInstall).toContain('Delete "$APPDATA\\PowerShift\\events.jsonl"');
    expect(preUninstall).toContain('schtasks /End /TN "PowerShiftAgent"');
    expect(preUninstall).toContain('schtasks /Delete /F /TN "PowerShiftAgent"');
    expect(installerHooks).toContain('DeleteRegValue HKCU');
    expect(preUninstall).toContain('powershift-tray.exe" --quit');
    expect(preUninstall).toContain('Sleep 1200');
    expect(preUninstall).toContain('RMDir /r "$APPDATA\\PowerShift"');
    expect(preUninstall).toContain('RMDir /r "$LOCALAPPDATA\\com.powershift.desktop"');
    expect(preUninstall).toContain('RMDir /r "$PROGRAMDATA\\PowerShift"');
    expect(preUninstall).toContain('Delete "$TEMP\\powershift-exe-icon.png"');
  });
});
