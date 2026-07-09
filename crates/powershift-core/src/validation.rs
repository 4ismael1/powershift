use crate::{AppConfig, MatchMode, RestoreBehavior};
use std::collections::HashSet;

const MAX_PROFILES: usize = 512;
const MAX_ASSOCIATED_PROCESSES_PER_PROFILE: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub code: ValidationCode,
    pub path: String,
    pub message: String,
}

impl ValidationIssue {
    fn new(code: ValidationCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationCode {
    UnsupportedVersion,
    TooManyProfiles,
    TooManyAssociatedProcesses,
    DuplicateProfileId,
    EmptyProfileId,
    EmptyProfileName,
    EmptyExecutableName,
    InvalidExecutableName,
    EmptyPowerPlanId,
    MissingRestorePowerPlan,
    EmptyProcessMatcher,
    MissingPathForPathMatcher,
    InvalidCloseDelay,
    InvalidPriority,
}

pub fn validate_config(config: &AppConfig) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    if config.version == 0 {
        issues.push(ValidationIssue::new(
            ValidationCode::UnsupportedVersion,
            "version",
            "config version must be at least 1",
        ));
    }

    if config.profiles.len() > MAX_PROFILES {
        issues.push(ValidationIssue::new(
            ValidationCode::TooManyProfiles,
            "profiles",
            format!("config cannot contain more than {MAX_PROFILES} profiles"),
        ));
    }

    let mut ids = HashSet::new();
    for (index, profile) in config.profiles.iter().enumerate() {
        let base = format!("profiles[{index}]");
        let id = profile.id.trim();
        if id.is_empty() {
            issues.push(ValidationIssue::new(
                ValidationCode::EmptyProfileId,
                format!("{base}.id"),
                "profile id cannot be empty",
            ));
        } else if !ids.insert(id.to_ascii_lowercase()) {
            issues.push(ValidationIssue::new(
                ValidationCode::DuplicateProfileId,
                format!("{base}.id"),
                "profile id must be unique",
            ));
        }

        if profile.name.trim().is_empty() {
            issues.push(ValidationIssue::new(
                ValidationCode::EmptyProfileName,
                format!("{base}.name"),
                "profile name cannot be empty",
            ));
        }

        validate_executable_name(
            &profile.main_executable.name,
            &format!("{base}.main_executable.name"),
            &mut issues,
        );

        if profile.power.on_start_plan_id.trim().is_empty() {
            issues.push(ValidationIssue::new(
                ValidationCode::EmptyPowerPlanId,
                format!("{base}.power.on_start_plan_id"),
                "start power plan id cannot be empty",
            ));
        }

        if profile.power.on_close_behavior == RestoreBehavior::SpecificPlan
            && profile
                .power
                .on_close_plan_id
                .as_ref()
                .map(|id| id.trim().is_empty())
                .unwrap_or(true)
        {
            issues.push(ValidationIssue::new(
                ValidationCode::MissingRestorePowerPlan,
                format!("{base}.power.on_close_plan_id"),
                "specific restore behavior requires a restore power plan id",
            ));
        }

        if profile.power.close_delay_seconds > 3600 {
            issues.push(ValidationIssue::new(
                ValidationCode::InvalidCloseDelay,
                format!("{base}.power.close_delay_seconds"),
                "close delay cannot exceed one hour",
            ));
        }

        if profile.power.priority > 100 {
            issues.push(ValidationIssue::new(
                ValidationCode::InvalidPriority,
                format!("{base}.power.priority"),
                "profile priority must be between 0 and 100",
            ));
        }

        if profile.associated_processes.len() > MAX_ASSOCIATED_PROCESSES_PER_PROFILE {
            issues.push(ValidationIssue::new(
                ValidationCode::TooManyAssociatedProcesses,
                format!("{base}.associated_processes"),
                format!(
                    "profile cannot contain more than {MAX_ASSOCIATED_PROCESSES_PER_PROFILE} associated processes"
                ),
            ));
        }

        for (matcher_index, matcher) in profile.associated_processes.iter().enumerate() {
            let matcher_base = format!("{base}.associated_processes[{matcher_index}]");
            if matcher.name.trim().is_empty()
                && matcher
                    .path
                    .as_ref()
                    .map(|path| path.trim().is_empty())
                    .unwrap_or(true)
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::EmptyProcessMatcher,
                    matcher_base.clone(),
                    "process matcher needs a name or path",
                ));
            }

            if matches!(matcher.match_mode, MatchMode::Path | MatchMode::Folder)
                && matcher
                    .path
                    .as_ref()
                    .map(|path| path.trim().is_empty())
                    .unwrap_or(true)
            {
                issues.push(ValidationIssue::new(
                    ValidationCode::MissingPathForPathMatcher,
                    format!("{matcher_base}.path"),
                    "path or folder matcher requires a path",
                ));
            }
        }
    }

    issues
}

fn validate_executable_name(name: &str, path: &str, issues: &mut Vec<ValidationIssue>) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        issues.push(ValidationIssue::new(
            ValidationCode::EmptyExecutableName,
            path,
            "executable name cannot be empty",
        ));
        return;
    }

    if !trimmed.to_ascii_lowercase().ends_with(".exe") {
        issues.push(ValidationIssue::new(
            ValidationCode::InvalidExecutableName,
            path,
            "executable name must end with .exe",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProcessMatcher, Profile};

    fn valid_config() -> AppConfig {
        let mut config = AppConfig::default();
        config
            .profiles
            .push(Profile::new("apex", "Apex Legends", "r5apex.exe", "high"));
        config
    }

    #[test]
    fn accepts_valid_default_profile_config() {
        let config = valid_config();
        assert!(validate_config(&config).is_empty());
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut config = valid_config();
        config.version = 0;

        let issues = validate_config(&config);

        assert!(issues
            .iter()
            .any(|issue| issue.code == ValidationCode::UnsupportedVersion));
    }

    #[test]
    fn rejects_duplicate_profile_ids_case_insensitively() {
        let mut config = valid_config();
        config
            .profiles
            .push(Profile::new("APEX", "Apex Copy", "copy.exe", "high"));

        let issues = validate_config(&config);

        assert_eq!(
            issues
                .iter()
                .filter(|issue| issue.code == ValidationCode::DuplicateProfileId)
                .count(),
            1
        );
    }

    #[test]
    fn rejects_empty_profile_identity_and_executable() {
        let mut config = AppConfig::default();
        config.profiles.push(Profile::new("", "", "", "balanced"));

        let issues = validate_config(&config);
        let codes: Vec<_> = issues.iter().map(|issue| issue.code).collect();

        assert!(codes.contains(&ValidationCode::EmptyProfileId));
        assert!(codes.contains(&ValidationCode::EmptyProfileName));
        assert!(codes.contains(&ValidationCode::EmptyExecutableName));
    }

    #[test]
    fn rejects_executable_without_exe_extension() {
        let mut config = AppConfig::default();
        config
            .profiles
            .push(Profile::new("minecraft", "Minecraft", "javaw", "balanced"));

        let issues = validate_config(&config);

        assert!(issues
            .iter()
            .any(|issue| issue.code == ValidationCode::InvalidExecutableName));
    }

    #[test]
    fn rejects_specific_restore_without_target_plan() {
        let mut config = valid_config();
        config.profiles[0].power.on_close_behavior = RestoreBehavior::SpecificPlan;
        config.profiles[0].power.on_close_plan_id = None;

        let issues = validate_config(&config);

        assert!(issues
            .iter()
            .any(|issue| issue.code == ValidationCode::MissingRestorePowerPlan));
    }

    #[test]
    fn rejects_path_matcher_without_path() {
        let mut config = valid_config();
        config.profiles[0]
            .associated_processes
            .push(ProcessMatcher {
                name: "launcher.exe".to_string(),
                path: None,
                match_mode: MatchMode::Path,
            });

        let issues = validate_config(&config);

        assert!(issues
            .iter()
            .any(|issue| issue.code == ValidationCode::MissingPathForPathMatcher));
    }

    #[test]
    fn rejects_close_delay_above_one_hour() {
        let mut config = valid_config();
        config.profiles[0].power.close_delay_seconds = 3601;

        let issues = validate_config(&config);

        assert!(issues
            .iter()
            .any(|issue| issue.code == ValidationCode::InvalidCloseDelay));
    }

    #[test]
    fn rejects_unbounded_profile_and_matcher_collections() {
        let mut too_many_profiles = AppConfig::default();
        for index in 0..=MAX_PROFILES {
            too_many_profiles.profiles.push(Profile::new(
                format!("profile-{index}"),
                format!("Profile {index}"),
                format!("game-{index}.exe"),
                "balanced",
            ));
        }
        assert!(validate_config(&too_many_profiles)
            .iter()
            .any(|issue| issue.code == ValidationCode::TooManyProfiles));

        let mut too_many_matchers = valid_config();
        too_many_matchers.profiles[0].associated_processes = (0
            ..=MAX_ASSOCIATED_PROCESSES_PER_PROFILE)
            .map(|index| ProcessMatcher::by_name(format!("helper-{index}.exe")))
            .collect();
        assert!(validate_config(&too_many_matchers)
            .iter()
            .any(|issue| issue.code == ValidationCode::TooManyAssociatedProcesses));
    }
}
