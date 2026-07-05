use powershift_core::{AppConfig, ConfigStore};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Manager};

const TRAY_EXE_NAME: &str = "powershift-tray.exe";

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
    if path.exists() {
        let input = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let file_version = config_version_from_json(&input);
        let config = ConfigStore::from_json_str(&input).map_err(|error| error.to_string())?;

        if should_persist_loaded_config(file_version, &config) {
            save_config_to_path(path, &config)?;
        }

        return Ok(config);
    }

    let config = AppConfig::default();
    save_config_to_path(path, &config)?;
    Ok(config)
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

    if should_sync_autostart_at_startup(cfg!(debug_assertions)) {
        sync_tray_autostart(app, &config)?;
    }
    if config.agent.show_tray_icon {
        let _ = start_tray_process(app);
    }
    if config.agent.enabled {
        let _ = crate::agent_control::ensure_agent_running();
    }

    Ok(())
}

fn should_sync_autostart_at_startup(debug_build: bool) -> bool {
    !debug_build
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_app_config() -> Result<AppConfig, String> {
    load_or_create_config(default_config_path())
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_app_config(app: AppHandle, config: AppConfig) -> Result<(), String> {
    let previous = load_or_create_config(default_config_path()).ok();
    let startup_changed = previous
        .as_ref()
        .map(|previous| previous.agent.start_with_windows != config.agent.start_with_windows)
        .unwrap_or(true);

    validate_config_for_save(&config)?;
    sync_tray_autostart(&app, &config)?;
    if startup_changed {
        crate::agent_control::sync_agent_startup_task(&app, config.agent.start_with_windows)?;
    }
    save_config_to_path(default_config_path(), &config)?;
    sync_running_tray(&app, &config)?;
    crate::windowing::sync_tray_visibility(&app, config.agent.show_tray_icon)?;
    if should_request_agent_rescan(previous.as_ref(), &config) {
        request_agent_rescan_after_config_save();
    }
    Ok(())
}

fn should_request_agent_rescan(previous: Option<&AppConfig>, next: &AppConfig) -> bool {
    previous
        .map(|previous| previous.agent.enabled || next.agent.enabled)
        .unwrap_or(next.agent.enabled)
}

fn request_agent_rescan_after_config_save() {
    std::thread::spawn(|| {
        let _ = crate::agent_control::wake_agent();
    });
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
    fn config_save_validates_and_syncs_external_state_before_persisting() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("config.rs");
        let source = std::fs::read_to_string(source_path).expect("read source");
        let save_body = source
            .split("pub fn save_app_config")
            .nth(1)
            .and_then(|source| source.split("fn should_request_agent_rescan").next())
            .expect("save_app_config body");

        let validate_index = save_body
            .find("validate_config_for_save(&config)?")
            .expect("validation call");
        let autostart_index = save_body
            .find("sync_tray_autostart(&app, &config)?")
            .expect("autostart sync call");
        let persist_index = save_body
            .find("save_config_to_path(default_config_path(), &config)?")
            .expect("config persist call");

        assert!(validate_index < autostart_index);
        assert!(autostart_index < persist_index);
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
    fn config_save_uses_resilient_agent_wake_path() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("config.rs");
        let source = std::fs::read_to_string(source_path).expect("read source");

        assert!(source.contains("request_agent_rescan_after_config_save"));
        assert!(source.contains("agent_control::wake_agent"));
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
}
