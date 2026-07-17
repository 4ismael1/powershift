use powershift_agent::{AgentPaths, PublishedAgentState};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::Manager;

const AGENT_EXE_NAME: &str = "powershift-agent.exe";
pub const REPAIR_AGENT_TASK_FLAG: &str = "--repair-agent-task";
const TASK_RESTART_COUNT: u8 = 3;
const TASK_RESTART_INTERVAL_MINUTES: u8 = 1;
const AGENT_START_TIMEOUT: Duration = Duration::from_secs(6);
const AGENT_START_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn quiet_command(program: &str) -> Command {
    let mut command = Command::new(program);
    configure_quiet_command(&mut command);
    command
}

#[cfg(windows)]
fn configure_quiet_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_quiet_command(_command: &mut Command) {}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStateResponse {
    #[serde(flatten)]
    pub state: PublishedAgentState,
    pub process_alive: bool,
    pub ipc_connected: bool,
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_agent_state() -> Result<Option<AgentStateResponse>, String> {
    if let Ok(state) = powershift_agent::request_agent_status_via_ipc() {
        return Ok(Some(agent_state_response_from_ipc(state)));
    }

    let path = AgentPaths::from_environment()
        .map_err(|error| error.to_string())?
        .state;
    if !path.exists() {
        return Ok(None);
    }

    let value = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&value)
        .map(|state| Some(agent_state_response(state)))
        .map_err(|error| error.to_string())
}

fn agent_state_response_from_ipc(state: PublishedAgentState) -> AgentStateResponse {
    AgentStateResponse {
        process_alive: true,
        ipc_connected: true,
        state,
    }
}

fn agent_state_response(state: PublishedAgentState) -> AgentStateResponse {
    AgentStateResponse {
        process_alive: powershift_windows::process_id_is_running(state.pid),
        ipc_connected: false,
        state,
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn wake_agent() -> Result<(), String> {
    if powershift_agent::request_agent_reevaluate_via_ipc().is_ok() {
        return Ok(());
    }

    match powershift_windows::signal_agent_wake() {
        Ok(()) => Ok(()),
        Err(first_error) => wake_agent_after_start_attempt(&first_error.to_string()),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn install_agent_task(app: tauri::AppHandle) -> Result<(), String> {
    let agent_path = agent_exe_path(Some(&app))?;

    if register_agent_task(&agent_path).is_ok() {
        return ensure_agent_running();
    }

    run_elevated_task_installer()
}

pub fn repair_agent_task_elevated_cli() -> Result<(), String> {
    let agent_path = agent_exe_path(None)?;
    register_agent_task(&agent_path)?;
    ensure_agent_running()
}

pub fn sync_agent_startup_task(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if powershift_agent::request_agent_set_startup_via_ipc(enabled).is_ok() {
        return Ok(());
    }

    if enabled {
        install_agent_task(app.clone())
    } else {
        disable_agent_startup_trigger()
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn start_agent_task() -> Result<(), String> {
    ensure_agent_running()
}

pub fn ensure_agent_running() -> Result<(), String> {
    match powershift_agent::request_agent_status_via_ipc() {
        Ok(state) if !agent_state_requires_elevated_restart(&state) => return Ok(()),
        Ok(state) => return promote_agent_to_elevated_task(state.pid),
        Err(_) => {}
    }

    run_agent_task()
}

pub fn stop_agent_task() -> Result<(), String> {
    if powershift_agent::request_agent_shutdown_via_ipc().is_ok() {
        return Ok(());
    }
    if !agent_task_exists()? {
        return Ok(());
    }

    let task_name = powershift_windows::agent_task_name().map_err(|error| error.to_string())?;
    let status = quiet_command("schtasks")
        .args(stop_agent_task_args(&task_name))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("No se pudo detener la tarea PowerShiftAgent.".to_string())
    }
}

fn promote_agent_to_elevated_task(agent_pid: u32) -> Result<(), String> {
    if !agent_task_exists()? {
        return Err("El agente responde, pero no esta elevado y la tarea PowerShiftAgent no esta instalada.".to_string());
    }

    if powershift_agent::request_agent_shutdown_via_ipc().is_err() {
        stop_agent_process_by_pid(agent_pid);
    }
    std::thread::sleep(Duration::from_millis(700));

    run_agent_task()
}

fn stop_agent_process_by_pid(pid: u32) {
    if pid == 0 || pid == std::process::id() {
        return;
    }

    let pid = pid.to_string();
    let _ = quiet_command("taskkill")
        .args(stop_agent_process_args(pid.as_str()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn stop_agent_process_args(pid: &str) -> [&str; 4] {
    ["/PID", pid, "/T", "/F"]
}

fn agent_state_requires_elevated_restart(state: &PublishedAgentState) -> bool {
    state
        .last_error
        .as_deref()
        .is_some_and(agent_error_requires_elevated_agent)
}

fn agent_error_requires_elevated_agent(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("wmi requiere permisos elevados")
        || normalized.contains("0x80041003")
        || (normalized.contains("wmi") && normalized.contains("access denied"))
        || (normalized.contains("wmi") && normalized.contains("acceso denegado"))
}

#[allow(dead_code)]
pub fn restart_agent_task() -> Result<(), String> {
    let task_name = powershift_windows::agent_task_name().map_err(|error| error.to_string())?;
    let _ = quiet_command("schtasks")
        .args(stop_agent_task_args(&task_name))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    run_agent_task()
}

fn run_agent_task() -> Result<(), String> {
    let task_name = powershift_windows::agent_task_name().map_err(|error| error.to_string())?;
    let status = quiet_command("schtasks")
        .args(run_agent_task_args(&task_name))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;

    if status.success() {
        wait_for_agent_ipc(AGENT_START_TIMEOUT)
    } else {
        Err("No se pudo iniciar la tarea PowerShiftAgent.".to_string())
    }
}

fn wait_for_agent_ipc(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = String::new();

    while Instant::now() < deadline {
        match powershift_agent::request_agent_status_via_ipc() {
            Ok(state) if !agent_state_requires_elevated_restart(&state) => return Ok(()),
            Ok(state) => {
                last_error = state.last_error.unwrap_or_else(|| {
                    "El agente arranco, pero requiere reparacion elevada.".to_string()
                });
            }
            Err(error) => last_error = error,
        }
        std::thread::sleep(AGENT_START_POLL_INTERVAL);
    }

    Err(format!(
        "Windows intento iniciar PowerShiftAgent, pero el agente no respondio. {last_error}"
    ))
}

fn run_agent_task_args(task_name: &str) -> [&str; 3] {
    ["/Run", "/TN", task_name]
}

fn stop_agent_task_args(task_name: &str) -> [&str; 3] {
    ["/End", "/TN", task_name]
}

#[tauri::command(rename_all = "snake_case")]
pub fn agent_task_installed(app: tauri::AppHandle) -> Result<bool, String> {
    let agent_path = agent_exe_path(Some(&app))?;
    agent_task_matches_expected_registration(&agent_path)
}

fn agent_task_exists() -> Result<bool, String> {
    let task_name = powershift_windows::agent_task_name().map_err(|error| error.to_string())?;
    let status = quiet_command("schtasks")
        .args(["/Query", "/TN", &task_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    Ok(status.success())
}

fn disable_agent_startup_trigger() -> Result<(), String> {
    if !agent_task_exists()? {
        return Ok(());
    }

    let task_name = powershift_windows::agent_task_name().map_err(|error| error.to_string())?;
    let command = disable_agent_startup_trigger_powershell(&task_name);
    let status = quiet_command("powershell")
        .args(elevated_powershell_args(&command))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err("No se pudo desactivar el inicio con Windows del agente.".to_string())
    }
}

fn agent_exe_path(app: Option<&tauri::AppHandle>) -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let resource_dir = app.and_then(|handle| handle.path().resource_dir().ok());
    agent_exe_path_from(&current, resource_dir.as_deref())
}

fn agent_exe_path_from(current: &Path, resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    for candidate in agent_exe_candidates(current, resource_dir) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(missing_agent_message(current, resource_dir))
}

fn agent_exe_candidates(current: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let directory = current
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    candidates.push(directory.join(AGENT_EXE_NAME));

    if let Some(resource_dir) = resource_dir {
        candidates.push(resource_dir.join(AGENT_EXE_NAME));
    }

    candidates
}

fn missing_agent_message(current_exe: &Path, resource_dir: Option<&Path>) -> String {
    let locations = agent_exe_candidates(current_exe, resource_dir)
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "No se encontro {AGENT_EXE_NAME}. Buscado en: {locations}. Ejecuta npm run build:agent:debug en desarrollo o npm run build:agent:release para release.",
    )
}

fn register_agent_task(agent_path: &Path) -> Result<(), String> {
    let user_sid =
        powershift_windows::current_user_sid_string().map_err(|error| error.to_string())?;
    let task_name = powershift_windows::agent_task_name_for_sid(&user_sid);
    let command = scheduled_task_registration_powershell(agent_path, &task_name, &user_sid);
    let status = quiet_command("powershell")
        .args(elevated_powershell_args(&command))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err("No se pudo registrar la tarea PowerShiftAgent sin elevacion.".to_string())
    }
}

fn run_elevated_task_installer() -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let exit_code = powershift_windows::run_elevated_and_wait(&current_exe, REPAIR_AGENT_TASK_FLAG)
        .map_err(|error| format!("Windows no autorizo la reparacion de PowerShift: {error}"))?;

    if exit_code == 0 {
        wait_for_agent_ipc(AGENT_START_TIMEOUT)
    } else {
        Err(format!(
            "La reparacion elevada de PowerShift termino con el codigo {exit_code}."
        ))
    }
}

fn scheduled_task_registration_powershell(
    agent_path: &Path,
    task_name: &str,
    user_sid: &str,
) -> String {
    let escaped_path = agent_path.display().to_string().replace('\'', "''");
    format!(
        "$agentPath = '{escaped_path}'; \
         $action = New-ScheduledTaskAction -Execute $agentPath; \
         $trigger = New-ScheduledTaskTrigger -AtLogOn; \
         $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount {TASK_RESTART_COUNT} -RestartInterval (New-TimeSpan -Minutes {TASK_RESTART_INTERVAL_MINUTES}); \
         $userSid = '{user_sid}'; \
         $user = ([System.Security.Principal.SecurityIdentifier]$userSid).Translate([System.Security.Principal.NTAccount]).Value; \
         $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest; \
         Register-ScheduledTask -TaskName '{task_name}' -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null"
    )
}

fn agent_task_matches_expected_registration(agent_path: &Path) -> Result<bool, String> {
    let task_name = powershift_windows::agent_task_name().map_err(|error| error.to_string())?;
    let command = task_registration_probe_powershell(&task_name, agent_path);
    let status = quiet_command("powershell")
        .args(elevated_powershell_args(&command))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    Ok(status.success())
}

fn task_registration_probe_powershell(task_name: &str, agent_path: &Path) -> String {
    let escaped_path = agent_path.display().to_string().replace('\'', "''");
    format!(
        "$task = Get-ScheduledTask -TaskName '{task_name}' -ErrorAction Stop; \
         $expected = [IO.Path]::GetFullPath('{escaped_path}'); \
         if (@($task.Actions).Count -ne 1) {{ exit 2 }}; \
         $actual = [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($task.Actions[0].Execute.Trim('`\"'))); \
         if ($actual -ine $expected -or $task.Principal.RunLevel -ne 'Highest') {{ exit 2 }}; \
         if ($task.Settings.DisallowStartIfOnBatteries -or $task.Settings.StopIfGoingOnBatteries -or -not $task.Settings.StartWhenAvailable -or $task.Settings.ExecutionTimeLimit -ne 'PT0S' -or $task.Settings.RestartCount -lt {TASK_RESTART_COUNT}) {{ exit 2 }}; \
         exit 0"
    )
}

fn disable_agent_startup_trigger_powershell(task_name: &str) -> String {
    format!(
        "$task = Get-ScheduledTask -TaskName '{task_name}' -ErrorAction Stop; \
     foreach ($trigger in $task.Triggers) {{ $trigger.Enabled = $false }}; \
     Set-ScheduledTask -TaskName '{task_name}' -Trigger $task.Triggers | Out-Null"
    )
}

fn elevated_powershell_args(command: &str) -> [&str; 7] {
    [
        "-NoProfile",
        "-WindowStyle",
        "Hidden",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        command,
    ]
}

fn wake_agent_after_start_attempt(first_error: &str) -> Result<(), String> {
    if !agent_task_exists()? {
        return Err(agent_wake_missing_task_message(first_error));
    }

    if let Err(start_error) = ensure_agent_running() {
        return Err(agent_wake_start_failed_message(first_error, &start_error));
    }

    std::thread::sleep(Duration::from_millis(700));
    powershift_windows::signal_agent_wake().map_err(|retry_error| {
        agent_wake_retry_failed_message(first_error, &retry_error.to_string())
    })
}

fn agent_wake_missing_task_message(first_error: &str) -> String {
    format!(
        "No se pudo despertar el agente porque la tarea elevada no esta instalada. Detalle: {first_error}"
    )
}

fn agent_wake_start_failed_message(first_error: &str, start_error: &str) -> String {
    format!(
        "No se pudo despertar el agente. Primer intento: {first_error}. Al iniciar la tarea: {start_error}"
    )
}

fn agent_wake_retry_failed_message(first_error: &str, retry_error: &str) -> String {
    format!(
        "El agente se intento iniciar, pero el evento wake sigue inaccesible. Primer intento: {first_error}. Reintento: {retry_error}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_USER_SID: &str = "S-1-5-21-1000";
    const TEST_TASK_NAME: &str = "PowerShiftAgent-S-1-5-21-1000";

    #[test]
    fn scheduled_task_uses_highest_privileges_and_agent_path() {
        let script = scheduled_task_registration_powershell(
            &PathBuf::from("C:\\PowerShift\\powershift-agent.exe"),
            TEST_TASK_NAME,
            TEST_USER_SID,
        );

        assert!(script.contains("New-ScheduledTaskTrigger -AtLogOn"));
        assert!(script.contains("New-ScheduledTaskPrincipal"));
        assert!(script.contains("-RunLevel Highest"));
        assert!(script.contains("C:\\PowerShift\\powershift-agent.exe"));
        assert!(script.contains("-ExecutionTimeLimit ([TimeSpan]::Zero)"));
        assert!(script.contains("-RestartCount 3"));
        assert!(script.contains("-RestartInterval (New-TimeSpan -Minutes 1)"));
        assert!(script.contains(TEST_TASK_NAME));
        assert!(script.contains(TEST_USER_SID));
        assert!(script.contains("SecurityIdentifier"));
        assert!(!script.contains("New-TimeSpan -Days"));
    }

    #[test]
    fn scheduled_task_can_start_on_battery_and_repair_without_forced_restart() {
        let script = scheduled_task_registration_powershell(
            &PathBuf::from("C:\\PowerShift\\powershift-agent.exe"),
            TEST_TASK_NAME,
            TEST_USER_SID,
        );
        let source = include_str!("agent_control.rs");
        let repair = source
            .split("pub fn repair_agent_task_elevated_cli")
            .nth(1)
            .and_then(|source| source.split("pub fn sync_agent_startup_task").next())
            .expect("repair function");

        assert!(script.contains("-AllowStartIfOnBatteries"));
        assert!(script.contains("-DontStopIfGoingOnBatteries"));
        assert!(script.contains("-StartWhenAvailable"));
        assert!(script.contains("-MultipleInstances IgnoreNew"));
        assert!(repair.contains("register_agent_task"));
        assert!(repair.contains("ensure_agent_running"));
        assert!(!repair.contains("restart_agent_task"));
    }

    #[test]
    fn normal_start_runs_task_without_ending_existing_agent() {
        assert_eq!(
            run_agent_task_args(TEST_TASK_NAME),
            ["/Run", "/TN", TEST_TASK_NAME]
        );
        assert_eq!(
            stop_agent_task_args(TEST_TASK_NAME),
            ["/End", "/TN", TEST_TASK_NAME]
        );
    }

    #[test]
    fn task_start_is_not_success_until_agent_ipc_responds() {
        let source = include_str!("agent_control.rs");
        let run_agent_task = source
            .split("fn run_agent_task()")
            .nth(1)
            .expect("run_agent_task body");

        assert!(run_agent_task.contains("wait_for_agent_ipc"));
        assert!(
            source.contains("Windows intento iniciar PowerShiftAgent, pero el agente no respondio")
        );
    }

    #[test]
    fn non_elevated_agent_fallback_targets_only_reported_pid() {
        assert_eq!(
            stop_agent_process_args("1234"),
            ["/PID", "1234", "/T", "/F"]
        );
    }

    #[test]
    fn task_registration_probe_checks_path_elevation_and_runtime_settings() {
        let probe = task_registration_probe_powershell(
            TEST_TASK_NAME,
            Path::new("C:\\Program Files\\PowerShift\\powershift-agent.exe"),
        );

        assert!(probe.contains("C:\\Program Files\\PowerShift\\powershift-agent.exe"));
        assert!(probe.contains("Actions[0].Execute"));
        assert!(probe.contains("Principal.RunLevel -ne 'Highest'"));
        assert!(probe.contains("DisallowStartIfOnBatteries"));
        assert!(probe.contains("StopIfGoingOnBatteries"));
        assert!(probe.contains("StartWhenAvailable"));
        assert!(probe.contains("ExecutionTimeLimit"));
        assert!(probe.contains("RestartCount"));
        assert!(probe.contains("PT0S"));
        assert!(probe.contains("exit 2"));
    }

    #[test]
    fn disabling_startup_disables_triggers_without_stopping_agent() {
        let script = disable_agent_startup_trigger_powershell(TEST_TASK_NAME);

        assert!(script.contains("$trigger.Enabled = $false"));
        assert!(script.contains("Set-ScheduledTask"));
        assert!(!script.contains("Stop-ScheduledTask"));
        assert!(!script.contains("Unregister-ScheduledTask"));
        assert!(!script.contains("/Delete"));
    }

    #[test]
    fn agent_state_response_includes_process_liveness() {
        let response = agent_state_response(PublishedAgentState {
            pid: std::process::id(),
            status: powershift_core::AgentStatus::Running,
            updated_at_ms: 1,
            last_scan: None,
            last_error: None,
            process_tracking: Default::default(),
            wmi_watchers: Default::default(),
        });

        assert!(response.process_alive);
        assert!(!response.ipc_connected);
    }

    #[test]
    fn agent_state_response_from_ipc_marks_live_transport() {
        let response = agent_state_response_from_ipc(PublishedAgentState {
            pid: 1,
            status: powershift_core::AgentStatus::Running,
            updated_at_ms: 1,
            last_scan: None,
            last_error: None,
            process_tracking: Default::default(),
            wmi_watchers: Default::default(),
        });

        assert!(response.process_alive);
        assert!(response.ipc_connected);
    }

    #[test]
    fn elevated_restart_is_required_for_wmi_permission_errors() {
        let state = PublishedAgentState {
            pid: 1,
            status: powershift_core::AgentStatus::Error,
            updated_at_ms: 1,
            last_scan: None,
            last_error: Some(
                "WMI requiere permisos elevados para eventos de proceso. Instala o inicia el agente elevado."
                    .to_string(),
            ),
            process_tracking: Default::default(),
            wmi_watchers: Default::default(),
        };

        assert!(agent_state_requires_elevated_restart(&state));
        assert!(agent_error_requires_elevated_agent(
            "HRESULT Call failed with: 0x80041003"
        ));
        assert!(agent_error_requires_elevated_agent(
            "WMI access denied while subscribing to process events"
        ));
    }

    #[test]
    fn elevated_restart_is_not_required_for_regular_agent_errors() {
        let state = PublishedAgentState {
            pid: 1,
            status: powershift_core::AgentStatus::Error,
            updated_at_ms: 1,
            last_scan: None,
            last_error: Some("No se pudo leer config.json".to_string()),
            process_tracking: Default::default(),
            wmi_watchers: Default::default(),
        };

        assert!(!agent_state_requires_elevated_restart(&state));
    }

    #[test]
    fn missing_agent_message_names_the_build_scripts() {
        let message = missing_agent_message(
            &PathBuf::from("C:\\PowerShift\\powershift.exe"),
            Some(Path::new("C:\\PowerShift\\resources")),
        );

        assert!(message.contains("build:agent:debug"));
        assert!(message.contains("build:agent:release"));
        assert!(message.contains("C:\\PowerShift\\powershift-agent.exe"));
        assert!(message.contains("C:\\PowerShift\\resources\\powershift-agent.exe"));
    }

    #[test]
    fn agent_path_prefers_executable_directory() {
        let base = temp_agent_dir("exe-dir");
        let host = base.join("PowerShift").join("powershift.exe");
        let resource_dir = base.join("resources");
        let agent = host.parent().expect("host parent").join(AGENT_EXE_NAME);
        std::fs::create_dir_all(host.parent().expect("host parent")).expect("create host dir");
        std::fs::create_dir_all(&resource_dir).expect("create resources dir");
        std::fs::write(&agent, []).expect("write agent");
        std::fs::write(resource_dir.join(AGENT_EXE_NAME), []).expect("write resource agent");

        let resolved = agent_exe_path_from(&host, Some(&resource_dir)).expect("resolve agent");

        assert_eq!(resolved, agent);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn agent_path_uses_resource_directory_when_not_next_to_exe() {
        let base = temp_agent_dir("resource-dir");
        let host = base.join("PowerShift").join("powershift.exe");
        let resource_dir = base.join("resources");
        let agent = resource_dir.join(AGENT_EXE_NAME);
        std::fs::create_dir_all(host.parent().expect("host parent")).expect("create host dir");
        std::fs::create_dir_all(&resource_dir).expect("create resources dir");
        std::fs::write(&agent, []).expect("write resource agent");

        let resolved = agent_exe_path_from(&host, Some(&resource_dir)).expect("resolve agent");

        assert_eq!(resolved, agent);
        let _ = std::fs::remove_dir_all(base);
    }

    fn temp_agent_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "powershift-agent-control-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn elevated_installer_relaunches_the_signed_powershift_host() {
        let source = include_str!("agent_control.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert_eq!(REPAIR_AGENT_TASK_FLAG, "--repair-agent-task");
        assert!(production.contains("run_elevated_and_wait"));
        assert!(!production.contains("Start-Process -FilePath powershell.exe"));
    }

    #[test]
    fn startup_setting_prefers_the_running_elevated_agent() {
        let source = include_str!("agent_control.rs");
        let sync = source
            .split("pub fn sync_agent_startup_task")
            .nth(1)
            .and_then(|source| source.split("#[tauri::command").next())
            .expect("startup sync function");

        assert!(sync.contains("request_agent_set_startup_via_ipc"));
        assert!(sync.find("request_agent_set_startup_via_ipc") < sync.find("install_agent_task"));
    }

    #[test]
    fn elevated_powershell_args_hide_the_host_window() {
        let args = elevated_powershell_args("Write-Output ok");

        assert_eq!(args[0], "-NoProfile");
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-WindowStyle", "Hidden"]));
        assert_eq!(args.last(), Some(&"Write-Output ok"));
    }

    #[test]
    fn wake_error_messages_explain_missing_task_start_failure_and_retry_failure() {
        assert!(agent_wake_missing_task_message("OpenEvent failed").contains("no esta instalada"));
        assert!(
            agent_wake_start_failed_message("OpenEvent failed", "schtasks failed")
                .contains("Al iniciar la tarea")
        );
        assert!(
            agent_wake_retry_failed_message("OpenEvent failed", "Access denied")
                .contains("sigue inaccesible")
        );
    }
}
