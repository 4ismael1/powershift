import { describe, expect, it } from 'vitest';
import agentBuildScript from '../crates/powershift-agent/build.rs?raw';
import agentManifest from '../crates/powershift-agent/Cargo.toml?raw';
import packageJsonRaw from '../package.json?raw';
import trayBuildScript from '../crates/powershift-tray/build.rs?raw';
import installerHooks from '../src-tauri/nsis/powershift-hooks.nsh?raw';
import tauriConfigRaw from '../src-tauri/tauri.conf.json?raw';

const tauriConfig = JSON.parse(tauriConfigRaw);
const packageJson = JSON.parse(packageJsonRaw);

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
      languages: ['Spanish'],
    });
  });

  it('ships stable publisher and version metadata in every executable', () => {
    expect(tauriConfig.version).toBe(packageJson.version);
    expect(tauriConfig.bundle.publisher).toBe('4ismael1');
    expect(tauriConfig.bundle.copyright).toContain('4ismael1');
    expect(agentManifest).toContain('embed-resource = "3.0.9"');
    expect(agentBuildScript).toContain('PowerShift background agent');
    expect(agentBuildScript).toContain('powershift-agent.exe');
    expect(trayBuildScript).toContain('PowerShift notification area companion');
    expect(trayBuildScript).toContain('powershift-tray.exe');
  });

  it('syncs background components on install and cleans them on uninstall', () => {
    const preInstall = macroBody('NSIS_HOOK_PREINSTALL');
    const postInstall = macroBody('NSIS_HOOK_POSTINSTALL');
    const preUninstall = macroBody('NSIS_HOOK_PREUNINSTALL');

    expect(installerHooks).toContain('NSIS_HOOK_PREINSTALL');
    expect(installerHooks).toContain('NSIS_HOOK_POSTINSTALL');
    expect(installerHooks).toContain('NSIS_HOOK_PREUNINSTALL');
    expect(installerHooks).toContain('PowerShiftReadExistingConfigFlags');
    expect(installerHooks).toContain('"start_with_windows": false');
    expect(installerHooks).toContain('"show_tray_icon": false');
    expect(postInstall).toContain('New-ScheduledTaskAction -Execute $$agentPath');
    expect(postInstall).toContain("$$taskName = 'PowerShiftAgent-' + $$sid");
    expect(postInstall).toContain('Register-ScheduledTask -TaskName $$taskName');
    expect(postInstall).toContain("$$ErrorActionPreference = 'Stop'");
    expect(postInstall).toContain('PowerShift no pudo registrar el agente elevado');
    expect(postInstall).toContain('PowerShift instaló el agente, pero Windows no pudo iniciarlo');
    expect(postInstall).toContain('Abort');
    expect(postInstall).toContain('-AllowStartIfOnBatteries');
    expect(postInstall).toContain('-DontStopIfGoingOnBatteries');
    expect(postInstall).toContain('-RunLevel Highest');
    expect(postInstall).toContain('-ExecutionTimeLimit ([TimeSpan]::Zero)');
    expect(postInstall).toContain('-RestartCount 3');
    expect(postInstall).toContain('-RestartInterval (New-TimeSpan -Minutes 1)');
    expect(postInstall).not.toContain('New-TimeSpan -Days');
    expect(postInstall).not.toContain('schtasks /Create');
    expect(postInstall).not.toContain('/TR');
    expect(postInstall).toContain('Start-ScheduledTask -TaskName $$taskName');
    expect(postInstall).toContain('$trigger.Enabled = $$false');
    expect(postInstall).toContain('Unregister-ScheduledTask -TaskName PowerShiftAgent');
    expect(postInstall).not.toContain('schtasks /End');
    expect(postInstall).not.toContain('schtasks /Delete');
    expect(preInstall).toContain('powershift-tray.exe" --quit');
    expect(preInstall).toContain('powershift-agent.exe" --shutdown-ipc');
    expect(preInstall).toContain('powershift-agent.exe" --release-power-control');
    expect(preInstall).toContain("Get-ScheduledTask -TaskName 'PowerShiftAgent-*'");
    expect(preInstall).toContain('Stop-ScheduledTask');
    expect(preInstall).not.toContain('Unregister-ScheduledTask');
    expect(installerHooks).toContain('powershift-agent.exe');
    expect(installerHooks).toContain('PowerShiftTray');
    expect(installerHooks).toContain('powershift-tray.exe');
    expect(installerHooks).toContain('nsis_tauri_utils::RunAsUser');
    expect(postInstall).toContain('Delete "$APPDATA\\PowerShift\\agent-state.json"');
    expect(postInstall).toContain('Delete "$APPDATA\\PowerShift\\agent-control.token"');
    expect(postInstall).toContain('Delete "$APPDATA\\PowerShift\\events.jsonl"');
    expect(preUninstall).toContain("Get-ScheduledTask -TaskName 'PowerShiftAgent-*'");
    expect(preUninstall).toContain("Get-ScheduledTask -TaskName 'PowerShiftAgent'");
    expect(preUninstall).toContain('Unregister-ScheduledTask -TaskName $$task.TaskName');
    expect(installerHooks).toContain('DeleteRegValue HKCU');
    expect(preUninstall).toContain('powershift-tray.exe" --quit');
    expect(preUninstall).toContain('Sleep 1200');
    expect(preUninstall).toContain('RMDir /r "$APPDATA\\PowerShift"');
    expect(preUninstall).toContain('RMDir /r "$LOCALAPPDATA\\com.powershift.desktop"');
    expect(preUninstall).toContain('RMDir /r "$PROGRAMDATA\\PowerShift"');
    expect(preUninstall).toContain('Delete "$TEMP\\powershift-exe-icon.png"');
  });
});
