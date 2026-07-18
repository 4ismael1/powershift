use crate::{AppConfig, AssociatedProcessRole, MatchMode, ProcessMatcher, Profile};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedProcess {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
}

impl DetectedProcess {
    pub fn new(pid: u32, name: impl Into<String>, path: Option<impl Into<String>>) -> Self {
        Self {
            pid,
            name: name.into(),
            path: path.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveProfile {
    pub profile_id: String,
    pub name: String,
    pub plan_id: String,
    pub priority: u8,
    pub matched_processes: Vec<DetectedProcess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerDecision {
    pub profile_id: String,
    pub profile_name: String,
    pub plan_id: String,
    pub priority: u8,
}

pub fn resolve_active_profiles(
    config: &AppConfig,
    processes: &[DetectedProcess],
) -> Vec<ActiveProfile> {
    resolve_active_profiles_with_previous(config, processes, &[])
}

/// Resolves active profiles while preserving sessions started by a main
/// executable or alternate trigger. A companion can extend a profile that was
/// active in the preceding evaluation, but cannot cold-start it.
pub fn resolve_active_profiles_with_previous(
    config: &AppConfig,
    processes: &[DetectedProcess],
    previously_active_profile_ids: &[String],
) -> Vec<ActiveProfile> {
    if !config.agent.enabled || !config.automation.enabled {
        return Vec::new();
    }

    let previously_active = previously_active_profile_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    config
        .profiles
        .iter()
        .filter_map(|profile| {
            active_profile_for(
                profile,
                processes,
                previously_active.contains(profile.id.as_str()),
            )
        })
        .collect()
}

pub fn choose_power_plan(
    config: &AppConfig,
    processes: &[DetectedProcess],
) -> Option<PowerDecision> {
    resolve_active_profiles(config, processes)
        .into_iter()
        .max_by_key(|profile| profile.priority)
        .map(|profile| PowerDecision {
            profile_id: profile.profile_id,
            profile_name: profile.name,
            plan_id: profile.plan_id,
            priority: profile.priority,
        })
}

/// Returns whether a process can contribute to any enabled profile. This is
/// intentionally less strict than activation: associated processes must be
/// retained too, so they can participate once the main executable appears.
pub fn process_matches_enabled_profile(config: &AppConfig, process: &DetectedProcess) -> bool {
    if !config.agent.enabled || !config.automation.enabled {
        return false;
    }

    config
        .profiles
        .iter()
        .filter(|profile| profile.enabled)
        .any(|profile| {
            process_matches_executable(
                process,
                &profile.main_executable.name,
                profile.main_executable.path.as_deref(),
                profile.activation.match_mode,
            ) || profile
                .associated_processes
                .iter()
                .any(|matcher| process_matches_matcher(process, matcher))
        })
}

fn active_profile_for(
    profile: &Profile,
    processes: &[DetectedProcess],
    was_previously_active: bool,
) -> Option<ActiveProfile> {
    if !profile.enabled {
        return None;
    }

    let main_matches: Vec<_> = processes
        .iter()
        .filter(|process| {
            process_matches_executable(
                process,
                &profile.main_executable.name,
                profile.main_executable.path.as_deref(),
                profile.activation.match_mode,
            )
        })
        .cloned()
        .collect();
    let main_match_found = !main_matches.is_empty();

    let mut companion_match_found = false;
    let mut alternate_trigger_match_found = false;
    let associated_matches = profile.associated_processes.iter().flat_map(|matcher| {
        let matches = processes
            .iter()
            .filter(move |process| process_matches_matcher(process, matcher))
            .cloned()
            .collect::<Vec<_>>();
        if !matches.is_empty() {
            match matcher.role {
                AssociatedProcessRole::Companion => companion_match_found = true,
                AssociatedProcessRole::AlternateTrigger => alternate_trigger_match_found = true,
            }
        }
        matches
    });

    let mut matched_processes = main_matches;
    for process in associated_matches {
        if !matched_processes
            .iter()
            .any(|known| known.pid == process.pid)
        {
            matched_processes.push(process);
        }
    }

    if matched_processes.is_empty() {
        return None;
    }

    let can_cold_start = main_match_found
        || alternate_trigger_match_found
        || (!profile.activation.require_main_process && companion_match_found);
    let can_extend_session = was_previously_active && companion_match_found;
    if !can_cold_start && !can_extend_session {
        return None;
    }

    Some(ActiveProfile {
        profile_id: profile.id.clone(),
        name: profile.name.clone(),
        plan_id: profile.power.on_start_plan_id.clone(),
        priority: profile.power.priority,
        matched_processes,
    })
}

fn process_matches_matcher(process: &DetectedProcess, matcher: &ProcessMatcher) -> bool {
    process_matches_executable(
        process,
        &matcher.name,
        matcher.path.as_deref(),
        matcher.match_mode,
    )
}

fn process_matches_executable(
    process: &DetectedProcess,
    expected_name: &str,
    expected_path: Option<&str>,
    match_mode: MatchMode,
) -> bool {
    match match_mode {
        MatchMode::Name => names_match(&process.name, expected_name),
        MatchMode::Path => expected_path
            .map(|path| paths_match(process.path.as_deref(), path))
            .unwrap_or(false),
        MatchMode::PathOrName => {
            let actual_path = process
                .path
                .as_deref()
                .filter(|path| !path.trim().is_empty());
            let expected_path = expected_path.filter(|path| !path.trim().is_empty());
            match (actual_path, expected_path) {
                (Some(actual_path), Some(expected_path)) => {
                    paths_match(Some(actual_path), expected_path)
                }
                _ => names_match(&process.name, expected_name),
            }
        }
        MatchMode::Folder => expected_path
            .map(|folder| process_is_in_folder(process.path.as_deref(), folder))
            .unwrap_or(false),
    }
}

fn names_match(actual: &str, expected: &str) -> bool {
    normalize_name(actual) == normalize_name(expected)
}

fn paths_match(actual: Option<&str>, expected: &str) -> bool {
    actual
        .map(|actual| normalize_path(actual) == normalize_path(expected))
        .unwrap_or(false)
}

fn process_is_in_folder(actual: Option<&str>, folder: &str) -> bool {
    actual
        .map(|actual| {
            let actual = normalize_path(actual);
            let mut folder = normalize_path(folder);
            if !folder.ends_with('\\') {
                folder.push('\\');
            }
            actual.starts_with(&folder)
        })
        .unwrap_or(false)
}

fn normalize_name(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}

fn normalize_path(input: &str) -> String {
    input.trim().replace('/', "\\").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutableRef, Profile};

    fn config_with_profiles(profiles: Vec<Profile>) -> AppConfig {
        AppConfig {
            profiles,
            ..AppConfig::default()
        }
    }

    fn process(pid: u32, name: &str, path: Option<&str>) -> DetectedProcess {
        DetectedProcess::new(pid, name, path)
    }

    #[test]
    fn resolves_profile_by_process_name_case_insensitively() {
        let config = config_with_profiles(vec![Profile::new(
            "apex",
            "Apex Legends",
            "r5apex.exe",
            "high",
        )]);
        let processes = vec![process(10, "R5APEX.EXE", None)];

        let active = resolve_active_profiles(&config, &processes);

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].profile_id, "apex");
        assert_eq!(active[0].matched_processes[0].pid, 10);
    }

    #[test]
    fn resolves_profile_by_exact_path_when_available() {
        let mut profile = Profile::new("apex", "Apex Legends", "wrong-name.exe", "high");
        profile.main_executable = ExecutableRef {
            name: "wrong-name.exe".to_string(),
            path: Some("C:\\Games\\Apex\\r5apex.exe".to_string()),
        };
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(10, "r5apex.exe", Some("c:/games/apex/r5apex.exe"))];

        let active = resolve_active_profiles(&config, &processes);

        assert_eq!(active.len(), 1);
    }

    #[test]
    fn path_or_name_rejects_matching_name_when_known_path_differs() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile.main_executable.path = Some("C:\\Games\\Apex\\r5apex.exe".to_string());
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(10, "r5apex.exe", Some("C:\\Unrelated\\r5apex.exe"))];

        let active = resolve_active_profiles(&config, &processes);

        assert!(active.is_empty());
    }

    #[test]
    fn path_or_name_falls_back_to_name_when_process_path_is_unavailable() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile.main_executable.path = Some("C:\\Games\\Apex\\r5apex.exe".to_string());
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(10, "R5APEX.EXE", None)];

        let active = resolve_active_profiles(&config, &processes);

        assert_eq!(active.len(), 1);
    }

    #[test]
    fn path_or_name_treats_blank_paths_as_unavailable() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile.main_executable.path = Some("   ".to_string());
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(
            10,
            "r5apex.exe",
            Some("C:\\Games\\Apex\\r5apex.exe"),
        )];

        let active = resolve_active_profiles(&config, &processes);

        assert_eq!(active.len(), 1);
    }

    #[test]
    fn ignores_disabled_profiles() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile.enabled = false;
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(10, "r5apex.exe", None)];

        let active = resolve_active_profiles(&config, &processes);

        assert!(active.is_empty());
    }

    #[test]
    fn ignores_profiles_when_global_automation_is_disabled() {
        let mut config = config_with_profiles(vec![Profile::new(
            "apex",
            "Apex Legends",
            "r5apex.exe",
            "high",
        )]);
        config.automation.enabled = false;
        let processes = vec![process(10, "r5apex.exe", None)];

        let active = resolve_active_profiles(&config, &processes);

        assert!(active.is_empty());
    }

    #[test]
    fn includes_associated_processes_after_main_process_matches() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile
            .associated_processes
            .push(ProcessMatcher::by_name("EasyAntiCheat.exe"));
        let config = config_with_profiles(vec![profile]);
        let processes = vec![
            process(10, "r5apex.exe", None),
            process(11, "easyanticheat.exe", None),
        ];

        let active = resolve_active_profiles(&config, &processes);

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].matched_processes.len(), 2);
    }

    #[test]
    fn requires_main_process_by_default_even_if_associated_process_matches() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile
            .associated_processes
            .push(ProcessMatcher::by_name("EasyAntiCheat.exe"));
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(11, "EasyAntiCheat.exe", None)];

        let active = resolve_active_profiles(&config, &processes);

        assert!(active.is_empty());
    }

    #[test]
    fn companion_extends_a_session_after_the_main_process_closes() {
        let mut profile = Profile::new("fortnite", "Fortnite", "fortnite.exe", "high");
        profile
            .associated_processes
            .push(ProcessMatcher::by_name("chrome.exe"));
        let config = config_with_profiles(vec![profile]);

        let started = resolve_active_profiles_with_previous(
            &config,
            &[
                process(10, "fortnite.exe", None),
                process(11, "chrome.exe", None),
            ],
            &[],
        );
        assert_eq!(started.len(), 1);

        let continued = resolve_active_profiles_with_previous(
            &config,
            &[process(11, "chrome.exe", None)],
            &["fortnite".to_string()],
        );
        assert_eq!(continued.len(), 1);
        assert_eq!(continued[0].matched_processes[0].pid, 11);

        let stopped =
            resolve_active_profiles_with_previous(&config, &[], &["fortnite".to_string()]);
        assert!(stopped.is_empty());
    }

    #[test]
    fn alternate_trigger_can_cold_start_a_profile() {
        let mut profile = Profile::new("fortnite", "Fortnite", "fortnite.exe", "high");
        let mut chrome = ProcessMatcher::by_name("chrome.exe");
        chrome.role = AssociatedProcessRole::AlternateTrigger;
        profile.associated_processes.push(chrome);
        let config = config_with_profiles(vec![profile]);

        let active =
            resolve_active_profiles_with_previous(&config, &[process(11, "chrome.exe", None)], &[]);

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].profile_id, "fortnite");
    }

    #[test]
    fn can_activate_without_main_process_when_configured() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile.activation.require_main_process = false;
        profile
            .associated_processes
            .push(ProcessMatcher::by_name("EasyAntiCheat.exe"));
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(11, "EasyAntiCheat.exe", None)];

        let active = resolve_active_profiles(&config, &processes);

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].matched_processes[0].pid, 11);
    }

    #[test]
    fn supports_folder_matchers_for_associated_processes() {
        let mut profile = Profile::new("launcher", "Game Folder", "game.exe", "high");
        profile.activation.require_main_process = false;
        profile.associated_processes.push(ProcessMatcher {
            name: String::new(),
            path: Some("D:\\Games\\Example".to_string()),
            match_mode: MatchMode::Folder,
            role: AssociatedProcessRole::Companion,
        });
        let config = config_with_profiles(vec![profile]);
        let processes = vec![process(
            20,
            "helper.exe",
            Some("D:/Games/Example/bin/helper.exe"),
        )];

        let active = resolve_active_profiles(&config, &processes);

        assert_eq!(active.len(), 1);
    }

    #[test]
    fn identifies_processes_that_can_contribute_before_a_profile_is_active() {
        let mut profile = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        profile
            .associated_processes
            .push(ProcessMatcher::by_name("EasyAntiCheat.exe"));
        let config = config_with_profiles(vec![profile]);

        assert!(process_matches_enabled_profile(
            &config,
            &process(11, "EasyAntiCheat.exe", None),
        ));
        assert!(!process_matches_enabled_profile(
            &config,
            &process(12, "notepad.exe", None),
        ));
    }

    #[test]
    fn does_not_watch_processes_when_automation_is_disabled() {
        let mut config = config_with_profiles(vec![Profile::new(
            "apex",
            "Apex Legends",
            "r5apex.exe",
            "high",
        )]);
        config.automation.enabled = false;

        assert!(!process_matches_enabled_profile(
            &config,
            &process(10, "r5apex.exe", None),
        ));
    }

    #[test]
    fn chooses_highest_priority_power_plan() {
        let mut balanced = Profile::new("minecraft", "Minecraft", "javaw.exe", "balanced");
        balanced.power.priority = 30;
        let mut high = Profile::new("apex", "Apex Legends", "r5apex.exe", "high");
        high.power.priority = 80;
        let config = config_with_profiles(vec![balanced, high]);
        let processes = vec![
            process(1, "javaw.exe", None),
            process(2, "r5apex.exe", None),
        ];

        let decision = choose_power_plan(&config, &processes).expect("expected decision");

        assert_eq!(decision.profile_id, "apex");
        assert_eq!(decision.plan_id, "high");
        assert_eq!(decision.priority, 80);
    }

    #[test]
    fn returns_none_when_no_profile_matches() {
        let config = config_with_profiles(vec![Profile::new(
            "apex",
            "Apex Legends",
            "r5apex.exe",
            "high",
        )]);
        let processes = vec![process(1, "notepad.exe", None)];

        assert!(choose_power_plan(&config, &processes).is_none());
    }
}
