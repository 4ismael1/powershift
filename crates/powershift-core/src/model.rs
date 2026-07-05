use serde::{Deserialize, Serialize};

pub const CURRENT_CONFIG_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub agent: AgentSettings,
    pub automation: AutomationSettings,
    pub ui: UiSettings,
    pub profiles: Vec<Profile>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION,
            agent: AgentSettings::default(),
            automation: AutomationSettings::default(),
            ui: UiSettings::default(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSettings {
    pub enabled: bool,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub show_tray_icon: bool,
    pub single_instance: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            start_with_windows: true,
            start_minimized: true,
            show_tray_icon: true,
            single_instance: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationSettings {
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    pub default_restore_behavior: RestoreBehavior,
    pub conflict_strategy: ConflictStrategy,
    pub respect_manual_plan_changes: bool,
    pub default_close_delay_seconds: u32,
}

impl Default for AutomationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            notifications_enabled: true,
            default_restore_behavior: RestoreBehavior::PreviousPlan,
            conflict_strategy: ConflictStrategy::HighestPriority,
            respect_manual_plan_changes: false,
            default_close_delay_seconds: 30,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSettings {
    pub theme: ThemePreference,
    pub language: String,
    pub close_button_behavior: CloseButtonBehavior,
    pub compact_mode: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::Dark,
            language: "es".to_string(),
            close_button_behavior: CloseButtonBehavior::HideWindow,
            compact_mode: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub main_executable: ExecutableRef,
    pub associated_processes: Vec<ProcessMatcher>,
    pub activation: ActivationSettings,
    pub power: ProfilePowerSettings,
    pub notifications: NotificationSettings,
    pub ui: ProfileUiSettings,
}

impl Profile {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        exe_name: impl Into<String>,
        on_start_plan_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            enabled: true,
            main_executable: ExecutableRef {
                name: exe_name.into(),
                path: None,
            },
            associated_processes: Vec::new(),
            activation: ActivationSettings::default(),
            power: ProfilePowerSettings {
                on_start_plan_id: on_start_plan_id.into(),
                ..ProfilePowerSettings::default()
            },
            notifications: NotificationSettings::default(),
            ui: ProfileUiSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutableRef {
    pub name: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessMatcher {
    pub name: String,
    pub path: Option<String>,
    pub match_mode: MatchMode,
}

impl ProcessMatcher {
    pub fn by_name(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: None,
            match_mode: MatchMode::Name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationSettings {
    pub match_mode: MatchMode,
    pub require_main_process: bool,
}

impl Default for ActivationSettings {
    fn default() -> Self {
        Self {
            match_mode: MatchMode::PathOrName,
            require_main_process: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfilePowerSettings {
    pub on_start_plan_id: String,
    pub on_close_behavior: RestoreBehavior,
    pub on_close_plan_id: Option<String>,
    pub close_delay_seconds: u32,
    pub priority: u8,
}

impl Default for ProfilePowerSettings {
    fn default() -> Self {
        Self {
            on_start_plan_id: String::new(),
            on_close_behavior: RestoreBehavior::PreviousPlan,
            on_close_plan_id: None,
            close_delay_seconds: 30,
            priority: 70,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettings {
    pub on_activate: bool,
    pub on_restore: bool,
    pub on_error: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            on_activate: true,
            on_restore: true,
            on_error: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProfileUiSettings {
    pub icon_cache_key: Option<String>,
    pub accent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerPlan {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub agent_status: AgentStatus,
    pub automation_enabled: bool,
    pub current_power_plan: Option<PowerPlan>,
    pub active_profiles: Vec<RuntimeActiveProfile>,
    pub pending_restores: Vec<PendingRestore>,
    pub last_event: Option<AgentEvent>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            agent_status: AgentStatus::Stopped,
            automation_enabled: true,
            current_power_plan: None,
            active_profiles: Vec::new(),
            pending_restores: Vec::new(),
            last_event: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeActiveProfile {
    pub profile_id: String,
    pub name: String,
    pub detected_processes: Vec<String>,
    pub activated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRestore {
    pub profile_id: String,
    pub restore_at: String,
    pub target_plan_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub level: EventLevel,
    pub kind: AgentEventKind,
    pub message: String,
    pub profile_id: Option<String>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    Name,
    Path,
    PathOrName,
    Folder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreBehavior {
    PreviousPlan,
    SpecificPlan,
    DoNothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    HighestPriority,
    LastActivated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseButtonBehavior {
    HideWindow,
    ExitApp,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Running,
    Paused,
    Error,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Active,
    Inactive,
    Disabled,
    WaitingRestore,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventKind {
    AgentStarted,
    AgentStopped,
    ConfigReloaded,
    ProcessStarted,
    ProcessStopped,
    ProfileActivated,
    ProfileDeactivated,
    PowerPlanChanged,
    RestoreScheduled,
    RestoreCancelled,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_automation_ready_and_empty() {
        let config = AppConfig::default();

        assert_eq!(config.version, CURRENT_CONFIG_VERSION);
        assert!(config.agent.enabled);
        assert!(config.agent.start_with_windows);
        assert!(config.agent.start_minimized);
        assert!(config.agent.show_tray_icon);
        assert!(config.automation.enabled);
        assert!(config.automation.notifications_enabled);
        assert!(config.profiles.is_empty());
        assert_eq!(config.ui.language, "es");
    }

    #[test]
    fn profile_constructor_sets_required_defaults() {
        let profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");

        assert_eq!(profile.id, "apex");
        assert_eq!(profile.name, "Apex Legends");
        assert!(profile.enabled);
        assert_eq!(profile.main_executable.name, "r5apex.exe");
        assert_eq!(profile.power.on_start_plan_id, "high");
        assert_eq!(
            profile.power.on_close_behavior,
            RestoreBehavior::PreviousPlan
        );
        assert_eq!(profile.power.close_delay_seconds, 30);
        assert!(profile.activation.require_main_process);
    }

    #[test]
    fn serde_uses_snake_case_for_enums() {
        let value = serde_json::to_value(RestoreBehavior::PreviousPlan).expect("serialize enum");

        assert_eq!(value, serde_json::json!("previous_plan"));
    }
}
