Var PowerShiftConfigHandle
Var PowerShiftConfigLine
Var PowerShiftSearchIndex
Var PowerShiftStartupEnabled
Var PowerShiftTrayEnabled

Function PowerShiftReadExistingConfigFlags
  StrCpy $PowerShiftStartupEnabled 1
  StrCpy $PowerShiftTrayEnabled 1

  ${IfNot} ${FileExists} "$APPDATA\PowerShift\config.json"
    Return
  ${EndIf}

  ClearErrors
  FileOpen $PowerShiftConfigHandle "$APPDATA\PowerShift\config.json" r
  IfErrors powershift_config_done

  powershift_config_loop:
    ClearErrors
    FileRead $PowerShiftConfigHandle $PowerShiftConfigLine
    IfErrors powershift_config_close

    ${StrLoc} $PowerShiftSearchIndex $PowerShiftConfigLine '"start_with_windows": false' ">"
    StrCmp $PowerShiftSearchIndex "" powershift_no_startup_match
      StrCpy $PowerShiftStartupEnabled 0
    powershift_no_startup_match:

    ${StrLoc} $PowerShiftSearchIndex $PowerShiftConfigLine '"show_tray_icon": false' ">"
    StrCmp $PowerShiftSearchIndex "" powershift_no_tray_match
      StrCpy $PowerShiftTrayEnabled 0
    powershift_no_tray_match:

    Goto powershift_config_loop

  powershift_config_close:
    FileClose $PowerShiftConfigHandle

  powershift_config_done:
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  ; An in-place update must release executable locks and return Windows to the
  ; pre-PowerShift plan before NSIS replaces the installed binaries.
  ${If} ${FileExists} "$INSTDIR\powershift-tray.exe"
    ExecWait '"$INSTDIR\powershift-tray.exe" --quit'
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\powershift-agent.exe"
    ExecWait '"$INSTDIR\powershift-agent.exe" --shutdown-ipc'
    Sleep 1200
    ExecWait '"$INSTDIR\powershift-agent.exe" --release-power-control'
  ${EndIf}
  nsExec::ExecToLog `powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "Get-ScheduledTask -TaskName 'PowerShiftAgent-*' -ErrorAction SilentlyContinue | Stop-ScheduledTask -ErrorAction SilentlyContinue"`
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; PowerShift installs the elevated agent as a tiny background task.
  ; The tray remains a normal user process so opening the UI never requires elevation.
  Call PowerShiftReadExistingConfigFlags

  ; Runtime state now lives in a high-integrity ProgramData directory. Remove
  ; legacy elevated output from the user-writable config directory.
  Delete "$APPDATA\PowerShift\agent-state.json"
  Delete "$APPDATA\PowerShift\agent-control.token"
  Delete "$APPDATA\PowerShift\events.jsonl"
  Delete "$APPDATA\PowerShift\events.jsonl.1"

  nsExec::ExecToLog `powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "$$ErrorActionPreference = 'Stop'; $$agentPath = '$INSTDIR\powershift-agent.exe'; $$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent(); $$sid = $$identity.User.Value; $$taskName = 'PowerShiftAgent-' + $$sid; $$action = New-ScheduledTaskAction -Execute $$agentPath; $$trigger = New-ScheduledTaskTrigger -AtLogOn; $$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1); $$principal = New-ScheduledTaskPrincipal -UserId $$identity.Name -LogonType Interactive -RunLevel Highest; Register-ScheduledTask -TaskName $$taskName -Action $$action -Trigger $$trigger -Settings $$settings -Principal $$principal -Force | Out-Null"`
  Pop $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "PowerShift no pudo registrar el agente elevado. La instalación no puede continuar."
    Abort
  ${EndIf}

  ; Migrate the global preview task after the SID-scoped replacement exists.
  nsExec::ExecToLog 'powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "Stop-ScheduledTask -TaskName PowerShiftAgent -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName PowerShiftAgent -Confirm:$$false -ErrorAction SilentlyContinue"'
  Pop $0

  ${If} $PowerShiftStartupEnabled = 1
    nsExec::ExecToLog `powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "$$ErrorActionPreference = 'Stop'; $$taskName = 'PowerShiftAgent-' + [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; Start-ScheduledTask -TaskName $$taskName"`
    Pop $0
    ${If} $0 != 0
      MessageBox MB_ICONSTOP|MB_OK "PowerShift instaló el agente, pero Windows no pudo iniciarlo."
      Abort
    ${EndIf}
  ${Else}
    nsExec::ExecToLog `powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "$$taskName = 'PowerShiftAgent-' + [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $$task = Get-ScheduledTask -TaskName $$taskName -ErrorAction SilentlyContinue; if ($$task) { foreach ($$trigger in $$task.Triggers) { $$trigger.Enabled = $$false }; Set-ScheduledTask -TaskName $$taskName -Trigger $$task.Triggers | Out-Null }"`
    Pop $0
  ${EndIf}

  ${If} $PowerShiftStartupEnabled = 1
  ${AndIf} $PowerShiftTrayEnabled = 1
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "PowerShiftTray" "$\"$INSTDIR\powershift-tray.exe$\""
    nsis_tauri_utils::RunAsUser "$INSTDIR\powershift-tray.exe" ""
  ${Else}
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "PowerShiftTray"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\powershift-tray.exe"
    ExecWait '"$INSTDIR\powershift-tray.exe" --quit'
    Sleep 1200
  ${EndIf}

  ; Restore the plan before removing the elevated task. The fallback is
  ; idempotent and also covers an unresponsive agent process.
  ${If} ${FileExists} "$INSTDIR\powershift-agent.exe"
    ExecWait '"$INSTDIR\powershift-agent.exe" --shutdown-ipc'
    Sleep 1200
    ExecWait '"$INSTDIR\powershift-agent.exe" --release-power-control'
  ${EndIf}

  nsExec::ExecToLog `powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "$$tasks = @(Get-ScheduledTask -TaskName 'PowerShiftAgent-*' -ErrorAction SilentlyContinue); $$legacy = Get-ScheduledTask -TaskName 'PowerShiftAgent' -ErrorAction SilentlyContinue; if ($$legacy) { $$tasks += $$legacy }; foreach ($$task in $$tasks) { Stop-ScheduledTask -TaskName $$task.TaskName -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName $$task.TaskName -Confirm:$$false -ErrorAction SilentlyContinue }"`
  Pop $0

  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "PowerShiftTray"

  RMDir /r "$APPDATA\PowerShift"
  RMDir /r "$LOCALAPPDATA\com.powershift.desktop"
  RMDir /r "$LOCALAPPDATA\PowerShift"
  RMDir /r "$PROGRAMDATA\PowerShift"
  Delete "$TEMP\powershift-exe-icon.png"
!macroend
