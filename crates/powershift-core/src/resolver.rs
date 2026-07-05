use crate::{AppConfig, MatchMode, ProcessMatcher, Profile};

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
    if !config.agent.enabled || !config.automation.enabled {
        return Vec::new();
    }

    config
        .profiles
        .iter()
        .filter_map(|profile| active_profile_for(profile, processes))
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

fn active_profile_for(profile: &Profile, processes: &[DetectedProcess]) -> Option<ActiveProfile> {
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

    if profile.activation.require_main_process && main_matches.is_empty() {
        return None;
    }

    let associated_matches = profile.associated_processes.iter().flat_map(|matcher| {
        processes
            .iter()
            .filter(move |process| process_matches_matcher(process, matcher))
            .cloned()
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
            expected_path
                .map(|path| paths_match(process.path.as_deref(), path))
                .unwrap_or(false)
                || names_match(&process.name, expected_name)
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
