use crate::{validate_config, write_file_atomically, AppConfig, CURRENT_CONFIG_VERSION};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::Path;

pub struct ConfigStore;

impl ConfigStore {
    pub fn from_json_str(input: &str) -> Result<AppConfig, ConfigError> {
        let config: AppConfig =
            serde_json::from_str(strip_utf8_bom(input)).map_err(ConfigError::Json)?;
        let config = migrate_config(config);
        let issues = validate_config(&config);
        if issues.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError::Validation(issues.len()))
        }
    }

    pub fn to_pretty_json(config: &AppConfig) -> Result<String, ConfigError> {
        let issues = validate_config(config);
        if !issues.is_empty() {
            return Err(ConfigError::Validation(issues.len()));
        }
        serde_json::to_string_pretty(config).map_err(ConfigError::Json)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<AppConfig, ConfigError> {
        let input = fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::from_json_str(&input)
    }

    pub fn save(path: impl AsRef<Path>, config: &AppConfig) -> Result<(), ConfigError> {
        let json = Self::to_pretty_json(config)?;
        write_file_atomically(path.as_ref(), json.as_bytes()).map_err(ConfigError::Io)
    }
}

fn strip_utf8_bom(input: &str) -> &str {
    input.strip_prefix('\u{feff}').unwrap_or(input)
}

fn migrate_config(mut config: AppConfig) -> AppConfig {
    if config.version < 2 {
        config.agent.start_with_windows = true;
        config.agent.start_minimized = true;
        config.agent.show_tray_icon = true;
    }

    if config.version < CURRENT_CONFIG_VERSION {
        config.version = CURRENT_CONFIG_VERSION;
    }

    config
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Json(serde_json::Error),
    Validation(usize),
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "I/O error while reading config: {err}"),
            ConfigError::Json(err) => write!(f, "Invalid config JSON: {err}"),
            ConfigError::Validation(count) => {
                write!(f, "Config failed validation with {count} issue(s)")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, Profile};
    use std::fs;

    fn valid_config() -> AppConfig {
        let mut config = AppConfig::default();
        config
            .profiles
            .push(Profile::new("apex", "Apex Legends", "r5apex.exe", "high"));
        config
    }

    #[test]
    fn serializes_and_deserializes_valid_config() {
        let config = valid_config();

        let json = ConfigStore::to_pretty_json(&config).expect("serialize config");
        let parsed = ConfigStore::from_json_str(&json).expect("parse config");

        assert_eq!(parsed, config);
    }

    #[test]
    fn rejects_invalid_json() {
        let error = ConfigStore::from_json_str("{invalid").expect_err("expected error");

        assert!(matches!(error, ConfigError::Json(_)));
    }

    #[test]
    fn accepts_utf8_bom_prefixed_json() {
        let config = valid_config();
        let json = ConfigStore::to_pretty_json(&config).expect("serialize config");
        let parsed =
            ConfigStore::from_json_str(&format!("\u{feff}{json}")).expect("parse config with bom");

        assert_eq!(parsed, config);
    }

    #[test]
    fn rejects_config_that_fails_validation() {
        let mut config = valid_config();
        config.profiles[0].main_executable.name = "r5apex".to_string();

        let error = ConfigStore::to_pretty_json(&config).expect_err("expected error");

        assert!(matches!(error, ConfigError::Validation(1)));
    }

    #[test]
    fn migrates_legacy_autostart_defaults() {
        let legacy = AppConfig {
            version: 1,
            agent: crate::AgentSettings {
                start_with_windows: false,
                start_minimized: false,
                show_tray_icon: false,
                ..crate::AgentSettings::default()
            },
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&legacy).expect("legacy json");
        let migrated = ConfigStore::from_json_str(&json).expect("migrated config");

        assert_eq!(migrated.version, CURRENT_CONFIG_VERSION);
        assert!(migrated.agent.start_with_windows);
        assert!(migrated.agent.start_minimized);
        assert!(migrated.agent.show_tray_icon);
    }

    #[test]
    fn migrates_missing_global_notification_preference_to_enabled() {
        let json = r#"{
            "version": 2,
            "agent": {
                "enabled": true,
                "start_with_windows": true,
                "start_minimized": true,
                "show_tray_icon": true,
                "single_instance": true
            },
            "automation": {
                "enabled": true,
                "default_restore_behavior": "previous_plan",
                "conflict_strategy": "highest_priority",
                "respect_manual_plan_changes": false,
                "default_close_delay_seconds": 30
            },
            "ui": {
                "theme": "dark",
                "language": "es",
                "close_button_behavior": "hide_window",
                "compact_mode": true
            },
            "profiles": []
        }"#;

        let migrated = ConfigStore::from_json_str(json).expect("migrated config");

        assert_eq!(migrated.version, CURRENT_CONFIG_VERSION);
        assert!(migrated.automation.notifications_enabled);
    }

    #[test]
    fn saves_and_loads_config_file() {
        let config = valid_config();
        let path = std::env::temp_dir().join(format!(
            "powershift-core-config-test-{}.json",
            std::process::id()
        ));

        ConfigStore::save(&path, &config).expect("save config");
        let loaded = ConfigStore::load(&path).expect("load config");
        let _ = fs::remove_file(&path);

        assert_eq!(loaded, config);
    }

    #[test]
    fn save_does_not_leave_temp_file_after_success() {
        let config = valid_config();
        let path = std::env::temp_dir().join(format!(
            "powershift-core-config-atomic-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        ConfigStore::save(&path, &config).expect("save config");

        assert!(path.exists());
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert!(!path
            .parent()
            .expect("parent")
            .read_dir()
            .expect("read dir")
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{stem}.tmp-"))));
        let _ = fs::remove_file(&path);
    }
}
