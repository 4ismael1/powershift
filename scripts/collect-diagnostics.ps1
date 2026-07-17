param(
    [string]$OutputDirectory = (Join-Path ([Environment]::GetFolderPath('Desktop')) 'PowerShift-Diagnostics')
)

$ErrorActionPreference = 'Stop'
$timestamp = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmssZ')
$staging = Join-Path $env:TEMP "PowerShift-Diagnostics-$timestamp"
$archive = Join-Path $OutputDirectory "PowerShift-Diagnostics-$timestamp.zip"

New-Item -ItemType Directory -Path $staging -Force | Out-Null
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

try {
    $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $taskName = "PowerShiftAgent-$sid"
    $installDirectory = Join-Path $env:ProgramFiles 'PowerShift'
    $runtimeDirectory = Join-Path $env:ProgramData "PowerShift\users\$sid"

    $operatingSystem = Get-CimInstance Win32_OperatingSystem
    $powerShiftProcesses = @(Get-Process -Name 'powershift*' -ErrorAction SilentlyContinue | ForEach-Object {
        [ordered]@{
            name = $_.ProcessName
            pid = $_.Id
            cpu_seconds = $_.CPU
            working_set_bytes = $_.WorkingSet64
            private_memory_bytes = $_.PrivateMemorySize64
            handles = $_.HandleCount
            started_at = $_.StartTime.ToUniversalTime().ToString('o')
        }
    })
    $executables = @('powershift.exe', 'powershift-agent.exe', 'powershift-tray.exe') | ForEach-Object {
        $path = Join-Path $installDirectory $_
        if (Test-Path -LiteralPath $path) {
            $item = Get-Item -LiteralPath $path
            [ordered]@{
                name = $_
                version = $item.VersionInfo.FileVersion
                sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
                signature = [string](Get-AuthenticodeSignature -LiteralPath $path).Status
            }
        }
    }

    $task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    $taskState = if ($task) {
        [ordered]@{
            name = $task.TaskName
            state = [string]$task.State
            action = $task.Actions[0].Execute
            run_level = [string]$task.Principal.RunLevel
            trigger_enabled = @($task.Triggers | ForEach-Object { $_.Enabled })
        }
    } else {
        $null
    }

    $activePowerPlan = (& powercfg.exe /getactivescheme 2>&1 | Out-String).Trim()
    $metadata = [ordered]@{
        collected_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        operating_system = [ordered]@{
            caption = $operatingSystem.Caption
            version = $operatingSystem.Version
            build = $operatingSystem.BuildNumber
            architecture = $operatingSystem.OSArchitecture
        }
        active_power_plan = $activePowerPlan
        executables = @($executables)
        processes = $powerShiftProcesses
        scheduled_task = $taskState
    }
    $metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $staging 'system.json') -Encoding UTF8

    foreach ($name in 'agent-state.json', 'events.jsonl', 'events.jsonl.1') {
        $source = Join-Path $runtimeDirectory $name
        if (Test-Path -LiteralPath $source) {
            Copy-Item -LiteralPath $source -Destination (Join-Path $staging $name)
        }
    }

    @'
This archive intentionally excludes config.json, agent-control.token, and
power-control-lease.json. Agent state and event logs can contain configured
profile and executable names; review them before sharing the archive.
'@ | Set-Content -LiteralPath (Join-Path $staging 'PRIVACY.txt') -Encoding UTF8

    if (Test-Path -LiteralPath $archive) {
        Remove-Item -LiteralPath $archive -Force
    }
    Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $archive -CompressionLevel Optimal
    Write-Output $archive
} finally {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}
