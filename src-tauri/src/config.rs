use powershift_core::{AppConfig, ConfigStore};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Manager};

const TRAY_EXE_NAME: &str = "powershift-tray.exe";
const CONFIG_RECOVERY_NOTICE_FILE: &str = "config-recovery.notice";
const CONFIG_RECOVERY_MESSAGE: &str =
    "La configuración estaba dañada. PowerShift conservó una copia y creó una configuración segura.";
const LEGACY_RUNTIME_FILES: [&str; 4] = [
    "agent-state.json",
    "agent-control.token",
    "events.jsonl",
    "events.jsonl.1",
];

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn default_config_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("PowerShift")
        .join("config.json")
}

pub fn load_or_create_config(path: PathBuf) -> Result<AppConfig, String> {
    cleanup_legacy_runtime_files(path.parent());
    if path.exists() {
        if config_uses_future_schema(&path) {
            return Err(
                "La configuración pertenece a una versión más reciente de PowerShift.".to_string(),
            );
        }
        let config = match ConfigStore::load_recovering(&path) {
            Ok(config) => config,
            Err(error) => recover_unreadable_config(&path, error.to_string())?,
        };
        let input = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let file_version = config_version_from_json(&input);

        if should_persist_loaded_config(file_version, &config) {
            save_config_to_path(path, &config)?;
        }

        return Ok(config);
    }

    let config = AppConfig::default();
    save_config_to_path(path, &config)?;
    Ok(config)
}

fn recover_unreadable_config(path: &Path, load_error: String) -> Result<AppConfig, String> {
    if config_or_backup_uses_future_schema(path) {
        return Err(format!(
            "La configuración pertenece a una versión más reciente de PowerShift. {load_error}"
        ));
    }

    quarantine_corrupt_config(path).map_err(|error| {
        format!("{load_error}. No se pudo conservar la configuración dañada: {error}")
    })?;
    let config = AppConfig::default();
    save_config_to_path(path.to_path_buf(), &config)?;
    write_config_recovery_notice(path);
    Ok(config)
}

fn config_or_backup_uses_future_schema(path: &Path) -> bool {
    [path.to_path_buf(), config_backup_path(path)]
        .iter()
        .any(|candidate| config_uses_future_schema(candidate))
}

fn config_uses_future_schema(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|input| config_version_from_json(&input))
        .is_some_and(|version| version > powershift_core::CURRENT_CONFIG_VERSION)
}

fn config_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".bak");
    PathBuf::from(backup)
}

fn quarantine_corrupt_config(path: &Path) -> Result<(), String> {
    let quarantine = corrupt_config_path(path);
    match std::fs::remove_file(&quarantine) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    std::fs::rename(path, quarantine).map_err(|error| error.to_string())
}

fn corrupt_config_path(path: &Path) -> PathBuf {
    let mut corrupt = path.as_os_str().to_os_string();
    corrupt.push(".corrupt");
    PathBuf::from(corrupt)
}

fn write_config_recovery_notice(config_path: &Path) {
    let Some(parent) = config_path.parent() else {
        return;
    };
    let _ = powershift_core::write_file_atomically(
        parent.join(CONFIG_RECOVERY_NOTICE_FILE),
        CONFIG_RECOVERY_MESSAGE.as_bytes(),
    );
}

fn take_config_recovery_notice(config_path: &Path) -> Option<String> {
    let path = config_path.parent()?.join(CONFIG_RECOVERY_NOTICE_FILE);
    let notice = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(path);
    let notice = notice.trim();
    (!notice.is_empty()).then(|| notice.to_string())
}

fn cleanup_legacy_runtime_files(config_dir: Option<&Path>) {
    let Some(config_dir) = config_dir else {
        return;
    };
    for file_name in LEGACY_RUNTIME_FILES {
        let _ = std::fs::remove_file(config_dir.join(file_name));
    }
}

fn config_version_from_json(input: &str) -> Option<u32> {
    serde_json::from_str::<serde_json::Value>(strip_utf8_bom(input))
        .ok()
        .and_then(|value| value.get("version").and_then(serde_json::Value::as_u64))
        .and_then(|version| u32::try_from(version).ok())
}

fn strip_utf8_bom(input: &str) -> &str {
    input.strip_prefix('\u{feff}').unwrap_or(input)
}

fn should_persist_loaded_config(file_version: Option<u32>, config: &AppConfig) -> bool {
    file_version.unwrap_or_default() < config.version
}

pub fn save_config_to_path(path: PathBuf, config: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    ConfigStore::save(path, config).map_err(|error| error.to_string())
}

fn validate_config_for_save(config: &AppConfig) -> Result<(), String> {
    ConfigStore::to_pretty_json(config)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn sync_startup_shell_settings(app: &AppHandle) -> Result<(), String> {
    let config = load_or_create_config(default_config_path())?;

    let mut warnings = Vec::new();

    if should_sync_autostart_at_startup(cfg!(debug_assertions)) {
        collect_sync_warning(
            &mut warnings,
            "inicio de la bandeja",
            sync_tray_autostart(app, &config),
        );
    }
    if config.agent.show_tray_icon {
        collect_sync_warning(&mut warnings, "proceso de bandeja", start_tray_process(app));
    }
    if config.agent.enabled {
        collect_sync_warning(
            &mut warnings,
            "agente",
            crate::agent_control::ensure_agent_running(),
        );
    }

    sync_warnings_result(warnings)
}

fn should_sync_autostart_at_startup(debug_build: bool) -> bool {
    !debug_build
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_app_config() -> Result<AppConfig, String> {
    load_or_create_config(default_config_path())
}

#[tauri::command(rename_all = "snake_case")]
pub fn take_config_recovery_warning() -> Result<Option<String>, String> {
    Ok(take_config_recovery_notice(&default_config_path()))
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_app_config(app: AppHandle, config: AppConfig) -> Result<ConfigSaveOutcome, String> {
    let previous = load_or_create_config(default_config_path()).ok();
    let startup_changed = previous
        .as_ref()
        .map(|previous| previous.agent.start_with_windows != config.agent.start_with_windows)
        .unwrap_or(true);

    persist_config_then_sync(default_config_path(), &config, |warnings| {
        collect_sync_warning(
            warnings,
            "inicio de la bandeja",
            sync_tray_autostart(&app, &config),
        );
        if startup_changed {
            collect_sync_warning(
                warnings,
                "inicio del agente",
                crate::agent_control::sync_agent_startup_task(
                    &app,
                    config.agent.start_with_windows,
                ),
            );
        }
        collect_sync_warning(
            warnings,
            "proceso de bandeja",
            sync_running_tray(&app, &config),
        );
        if should_request_agent_rescan(previous.as_ref(), &config) {
            collect_agent_rescan_warning(warnings, crate::agent_control::wake_agent);
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigSaveOutcome {
    pub warnings: Vec<String>,
}

fn collect_sync_warning(warnings: &mut Vec<String>, component: &str, result: Result<(), String>) {
    if let Err(error) = result {
        warnings.push(format!("No se pudo sincronizar {component}: {error}"));
    }
}

fn persist_config_then_sync(
    path: PathBuf,
    config: &AppConfig,
    sync: impl FnOnce(&mut Vec<String>),
) -> Result<ConfigSaveOutcome, String> {
    validate_config_for_save(config)?;
    save_config_to_path(path, config)?;

    let mut warnings = Vec::new();
    sync(&mut warnings);
    Ok(ConfigSaveOutcome { warnings })
}

fn sync_warnings_result(warnings: Vec<String>) -> Result<(), String> {
    if warnings.is_empty() {
        Ok(())
    } else {
        Err(warnings.join("; "))
    }
}

fn should_request_agent_rescan(previous: Option<&AppConfig>, next: &AppConfig) -> bool {
    let Some(previous) = previous else {
        return next.agent.enabled && next.automation.enabled;
    };

    if previous.agent.enabled != next.agent.enabled
        || previous.automation.enabled != next.automation.enabled
    {
        return true;
    }

    next.agent.enabled && next.automation.enabled && previous.profiles != next.profiles
}

fn collect_agent_rescan_warning(
    warnings: &mut Vec<String>,
    wake_agent: impl FnOnce() -> Result<(), String>,
) {
    collect_sync_warning(warnings, "recarga del agente", wake_agent());
}

fn sync_tray_autostart(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let enabled = config.agent.start_with_windows && config.agent.show_tray_icon;
    let tray_path = if enabled {
        tray_exe_path(Some(app))?
    } else {
        PathBuf::new()
    };
    powershift_windows::set_autostart_for_executable(
        powershift_windows::TRAY_AUTOSTART_VALUE_NAME,
        enabled,
        &tray_path,
        tray_autostart_args(config),
    )
    .map_err(|error| error.to_string())
}

fn tray_autostart_args(config: &AppConfig) -> &[&'static str] {
    if config.agent.start_minimized {
        &[]
    } else {
        &["--open-ui"]
    }
}

fn sync_running_tray(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    if config.agent.show_tray_icon {
        start_tray_process(app)?;
    } else {
        let _ = powershift_windows::signal_ipc_event(powershift_windows::TRAY_QUIT_EVENT_NAME);
    }
    Ok(())
}

fn start_tray_process(app: &AppHandle) -> Result<(), String> {
    let tray_path = tray_exe_path(Some(app))?;
    let mut command = Command::new(tray_path);
    configure_quiet_command(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn tray_exe_path(app: Option<&AppHandle>) -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let resource_dir = app.and_then(|handle| handle.path().resource_dir().ok());
    tray_exe_path_from(&current, resource_dir.as_deref())
}

fn tray_exe_path_from(current: &Path, resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    for candidate in tray_exe_candidates(current, resource_dir) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(missing_tray_message(current, resource_dir))
}

fn tray_exe_candidates(current: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let directory = current
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    candidates.push(directory.join(TRAY_EXE_NAME));

    if let Some(resource_dir) = resource_dir {
        candidates.push(resource_dir.join(TRAY_EXE_NAME));
    }

    candidates
}

fn missing_tray_message(current_exe: &Path, resource_dir: Option<&Path>) -> String {
    let locations = tray_exe_candidates(current_exe, resource_dir)
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "No se encontro {TRAY_EXE_NAME}. Buscado en: {locations}. Ejecuta npm run build:tray:debug en desarrollo o npm run build:tray:release para release.",
    )
}

#[cfg(windows)]
fn configure_quiet_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_quiet_command(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use powershift_core::Profile;

    fn temp_config_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "powershift-tauri-config-{name}-{}.json",
            std::process::id()
        ))
    }

    #[test]
    fn load_or_create_config_creates_default_file_when_missing() {
        let path = temp_config_path("create");
        let _ = std::fs::remove_file(&path);

        let config = load_or_create_config(path.clone()).expect("load default config");

        assert_eq!(config, AppConfig::default());
        assert!(path.exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn config_load_removes_only_legacy_runtime_files() {
        let directory = temp_config_path("legacy-runtime")
            .parent()
            .expect("temp parent")
            .join(format!("powershift-legacy-runtime-{}", std::process::id()));
        let config_path = directory.join("config.json");
        std::fs::create_dir_all(&directory).expect("create temp directory");
        std::fs::write(
            &config_path,
            ConfigStore::to_pretty_json(&AppConfig::default()).unwrap(),
        )
        .expect("seed config");
        for file_name in LEGACY_RUNTIME_FILES {
            std::fs::write(directory.join(file_name), b"legacy").expect("seed legacy file");
        }

        load_or_create_config(config_path.clone()).expect("load config");

        assert!(config_path.exists());
        for file_name in LEGACY_RUNTIME_FILES {
            assert!(!directory.join(file_name).exists());
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn load_or_create_config_persists_migrated_config() {
        let path = temp_config_path("migrate");
        let _ = std::fs::remove_file(&path);
        let legacy = AppConfig {
            version: 1,
            agent: powershift_core::AgentSettings {
                start_with_windows: false,
                start_minimized: false,
                show_tray_icon: false,
                ..powershift_core::AgentSettings::default()
            },
            ..AppConfig::default()
        };
        let json = ConfigStore::to_pretty_json(&legacy).expect("legacy json");
        std::fs::write(&path, json).expect("write legacy");

        let migrated = load_or_create_config(path.clone()).expect("load migrated");
        let persisted = ConfigStore::load(&path).expect("reload persisted");

        assert_eq!(migrated, persisted);
        assert_eq!(persisted.version, powershift_core::CURRENT_CONFIG_VERSION);
        assert!(persisted.agent.start_with_windows);
        assert!(persisted.agent.start_minimized);
        assert!(persisted.agent.show_tray_icon);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_config_to_path_persists_valid_config() {
        let path = temp_config_path("save");
        let _ = std::fs::remove_file(&path);
        let mut config = AppConfig::default();
        config.profiles.push(Profile::new(
            "notepad",
            "Notepad",
            "notepad.exe",
            "balanced",
        ));

        save_config_to_path(path.clone(), &config).expect("save config");
        let loaded = load_or_create_config(path.clone()).expect("load config");

        assert_eq!(loaded, config);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_config_to_path_rejects_invalid_config() {
        let path = temp_config_path("invalid");
        let _ = std::fs::remove_file(&path);
        let mut config = AppConfig::default();
        config.profiles.push(Profile::new("bad", "Bad", "bad", ""));

        let error =
            save_config_to_path(path.clone(), &config).expect_err("expected validation error");

        assert!(error.contains("validation"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn corrupt_config_without_backup_is_quarantined_and_recovered() {
        let path = temp_config_path("corrupt-recovery");
        let parent = path.parent().expect("temp parent");
        let corrupt = corrupt_config_path(&path);
        let notice = parent.join(CONFIG_RECOVERY_NOTICE_FILE);
        let backup = config_backup_path(&path);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&corrupt);
        let _ = std::fs::remove_file(&notice);
        let _ = std::fs::remove_file(&backup);
        std::fs::write(&path, b"{invalid").expect("seed corrupt config");

        let recovered = load_or_create_config(path.clone()).expect("recover config");

        assert_eq!(recovered, AppConfig::default());
        assert_eq!(
            ConfigStore::load(&path).expect("valid replacement"),
            recovered
        );
        assert_eq!(
            std::fs::read_to_string(&corrupt).expect("quarantine"),
            "{invalid"
        );
        assert_eq!(
            take_config_recovery_notice(&path).as_deref(),
            Some(CONFIG_RECOVERY_MESSAGE)
        );
        assert!(take_config_recovery_notice(&path).is_none());

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(corrupt);
        let _ = std::fs::remove_file(backup);
    }

    #[test]
    fn future_config_is_never_overwritten_by_recovery() {
        let path = temp_config_path("future-schema");
        let corrupt = corrupt_config_path(&path);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&corrupt);
        let future = format!(
            "{{\"version\":{},\"profiles\":[]}}",
            powershift_core::CURRENT_CONFIG_VERSION + 1
        );
        std::fs::write(&path, &future).expect("seed future config");

        let error = load_or_create_config(path.clone()).expect_err("reject future config");

        assert!(error.contains("versión más reciente"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("preserved future config"),
            future
        );
        assert!(!corrupt.exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn config_save_persists_before_syncing_external_state() {
        let path = temp_config_path("persist-before-sync");
        let _ = std::fs::remove_file(&path);
        let config = AppConfig::default();
        let mut observed_persisted_config = false;

        let outcome = persist_config_then_sync(path.clone(), &config, |warnings| {
            observed_persisted_config = ConfigStore::load(&path)
                .map(|persisted| persisted == config)
                .unwrap_or(false);
            collect_sync_warning(warnings, "prueba", Err("fallo externo".to_string()));
        })
        .expect("persist config");

        assert!(observed_persisted_config);
        assert_eq!(outcome.warnings.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn external_sync_failures_are_reported_as_non_destructive_warnings() {
        let mut warnings = Vec::new();

        collect_sync_warning(
            &mut warnings,
            "agente",
            Err("tarea no disponible".to_string()),
        );
        collect_sync_warning(&mut warnings, "bandeja", Ok(()));

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("agente"));
        assert!(warnings[0].contains("tarea no disponible"));
    }

    #[test]
    fn startup_sync_reports_all_failures_after_running_every_step() {
        let result = sync_warnings_result(vec![
            "bandeja fallida".to_string(),
            "agente fallido".to_string(),
        ])
        .expect_err("combined startup warning");

        assert!(result.contains("bandeja fallida"));
        assert!(result.contains("agente fallido"));
    }

    #[test]
    fn startup_autostart_sync_is_release_only() {
        assert!(should_sync_autostart_at_startup(false));
        assert!(!should_sync_autostart_at_startup(true));
    }

    #[test]
    fn tray_path_prefers_executable_directory() {
        let base = temp_config_path("tray-path")
            .parent()
            .expect("temp parent")
            .join(format!("powershift-tray-path-{}", std::process::id()));
        let host = base.join("PowerShift").join("powershift.exe");
        let tray = host.parent().expect("host parent").join(TRAY_EXE_NAME);
        std::fs::create_dir_all(host.parent().expect("host parent")).expect("create host dir");
        std::fs::write(&tray, []).expect("write tray");

        let resolved = tray_exe_path_from(&host, None).expect("resolve tray");

        assert_eq!(resolved, tray);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn missing_tray_message_names_build_scripts() {
        let message = missing_tray_message(
            &PathBuf::from("C:\\PowerShift\\powershift.exe"),
            Some(Path::new("C:\\PowerShift\\resources")),
        );

        assert!(message.contains("build:tray:debug"));
        assert!(message.contains("build:tray:release"));
        assert!(message.contains("powershift-tray.exe"));
    }

    #[test]
    fn tray_autostart_opens_ui_only_when_not_minimized() {
        let mut config = AppConfig::default();
        config.agent.start_minimized = true;
        assert!(tray_autostart_args(&config).is_empty());

        config.agent.start_minimized = false;
        assert_eq!(tray_autostart_args(&config), &["--open-ui"]);
    }

    #[test]
    fn loaded_config_is_persisted_only_when_version_increases() {
        let config = AppConfig::default();

        assert!(should_persist_loaded_config(Some(1), &config));
        assert!(!should_persist_loaded_config(Some(config.version), &config));
    }

    #[test]
    fn config_version_reader_accepts_utf8_bom() {
        assert_eq!(config_version_from_json("\u{feff}{\"version\":2}"), Some(2));
    }

    #[test]
    fn config_save_reports_agent_rescan_failures() {
        let mut warnings = Vec::new();

        collect_agent_rescan_warning(&mut warnings, || Err("agente no disponible".to_string()));

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("recarga del agente"));
        assert!(warnings[0].contains("agente no disponible"));
    }

    #[test]
    fn startup_ensures_agent_without_forcing_restart() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("config.rs");
        let source = std::fs::read_to_string(source_path).expect("read source");
        let production_source = source.split("#[cfg(test)]").next().unwrap_or(&source);

        assert!(production_source.contains("agent_control::ensure_agent_running"));
        assert!(!production_source.contains("let _ = crate::agent_control::start_agent_task();"));
    }

    #[test]
    fn config_save_requests_rescan_when_agent_enabled_state_changes() {
        let mut previous = AppConfig::default();
        let mut next = AppConfig::default();

        previous.agent.enabled = true;
        next.agent.enabled = false;
        assert!(should_request_agent_rescan(Some(&previous), &next));

        previous.agent.enabled = false;
        next.agent.enabled = true;
        assert!(should_request_agent_rescan(Some(&previous), &next));

        previous.agent.enabled = false;
        next.agent.enabled = false;
        assert!(!should_request_agent_rescan(Some(&previous), &next));
    }

    #[test]
    fn config_save_rescans_only_for_runtime_relevant_changes() {
        let previous = AppConfig::default();

        let mut profile_change = previous.clone();
        profile_change.profiles.push(Profile::new(
            "notepad",
            "Notepad",
            "notepad.exe",
            "balanced",
        ));
        assert!(should_request_agent_rescan(
            Some(&previous),
            &profile_change
        ));

        let mut automation_change = previous.clone();
        automation_change.automation.enabled = false;
        assert!(should_request_agent_rescan(
            Some(&previous),
            &automation_change
        ));

        let mut notification_change = previous.clone();
        notification_change.automation.notifications_enabled = false;
        assert!(!should_request_agent_rescan(
            Some(&previous),
            &notification_change
        ));

        let mut close_button_change = previous.clone();
        close_button_change.ui.close_button_behavior =
            powershift_core::CloseButtonBehavior::ExitApp;
        assert!(!should_request_agent_rescan(
            Some(&previous),
            &close_button_change
        ));
    }
}
