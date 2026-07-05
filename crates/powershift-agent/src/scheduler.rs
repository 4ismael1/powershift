use crate::{next_wait_at, now_ms, AgentRuntimeState};
use powershift_core::AppConfig;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

pub(crate) const PROCESS_EVENT_DEBOUNCE: Duration = Duration::from_millis(750);
pub(crate) const PROCESS_EVENT_MAX_COALESCE: Duration = Duration::from_secs(2);
pub(crate) const ACTIVE_PROCESS_STOP_MAX_COALESCE: Duration = Duration::from_secs(3);
pub(crate) const DEGRADED_PROCESS_SCAN_INTERVAL: Duration = Duration::from_secs(30);
pub(crate) const FOLDER_BROAD_WAKE_COOLDOWN: Duration = Duration::from_secs(5);
pub(crate) const PUBLIC_WAKE_EVENT_COOLDOWN: Duration = Duration::from_secs(2);

const AGENT_WAKE_PROCESS_NAME: &str = "agent_wake";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentWatchSet {
    process_profiles: BTreeMap<String, BTreeSet<String>>,
    broad_profiles: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AffectedWatchProfiles {
    pub(crate) profiles: BTreeSet<String>,
    pub(crate) broad_only: bool,
}

impl AffectedWatchProfiles {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

impl AgentWatchSet {
    pub(crate) fn from_config(config: &AppConfig) -> Self {
        if !config.agent.enabled || !config.automation.enabled {
            return Self::default();
        }

        let mut process_profiles = BTreeMap::<String, BTreeSet<String>>::new();
        let mut broad_profiles = BTreeSet::<String>::new();
        for profile in config.profiles.iter().filter(|profile| profile.enabled) {
            add_profile_executable(
                &mut process_profiles,
                &mut broad_profiles,
                &profile.main_executable.name,
                profile.main_executable.path.as_deref(),
                profile.activation.match_mode,
                &profile.id,
            );
            for process in &profile.associated_processes {
                add_profile_executable(
                    &mut process_profiles,
                    &mut broad_profiles,
                    &process.name,
                    process.path.as_deref(),
                    process.match_mode,
                    &profile.id,
                );
            }
        }

        Self {
            process_profiles,
            broad_profiles,
        }
    }

    pub(crate) fn affected_profiles(&self, process_name: &str) -> AffectedWatchProfiles {
        let exact_profiles = self
            .process_profiles
            .get(&normalize_process_name(process_name))
            .cloned()
            .unwrap_or_default();
        let mut profiles = self.broad_profiles.clone();
        if !exact_profiles.is_empty() {
            profiles.extend(exact_profiles.iter().cloned());
        }
        AffectedWatchProfiles {
            broad_only: !self.broad_profiles.is_empty() && exact_profiles.is_empty(),
            profiles,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.process_profiles.is_empty() && self.broad_profiles.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentScanScheduler {
    pending: Option<PendingScheduledScan>,
    last_broad_wake_ms: Option<u64>,
    last_public_wake_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingScheduledScan {
    first_seen_ms: u64,
    due_at_ms: u64,
    force: bool,
    broad: bool,
}

impl AgentScanScheduler {
    pub(crate) fn schedule_forced(&mut self, now_ms: u64) {
        self.pending = Some(PendingScheduledScan {
            first_seen_ms: now_ms,
            due_at_ms: now_ms,
            force: true,
            broad: false,
        });
    }

    pub(crate) fn record_public_wake(&mut self, now_ms: u64) -> bool {
        if self.public_wake_on_cooldown(now_ms) {
            return false;
        }

        self.last_public_wake_ms = Some(now_ms);
        self.schedule_forced(now_ms);
        true
    }

    pub(crate) fn record_process_event(
        &mut self,
        event: &powershift_windows::ProcessEvent,
        watch_set: &AgentWatchSet,
        active_profile_ids: &BTreeSet<String>,
        now_ms: u64,
    ) -> bool {
        let affected = watch_set.affected_profiles(&event.name);
        if affected.profiles.is_empty() {
            if event.kind == powershift_windows::ProcessEventKind::Stopped
                && !active_profile_ids.is_empty()
            {
                self.schedule_debounced_scan(PROCESS_EVENT_MAX_COALESCE, false, now_ms);
                return true;
            }
            return false;
        }
        if affected.broad_only && self.broad_wake_on_cooldown(now_ms) {
            return false;
        }

        let affects_inactive_profile = affected
            .profiles
            .iter()
            .any(|profile_id| !active_profile_ids.contains(profile_id));
        let affects_active_profile = affected
            .profiles
            .iter()
            .any(|profile_id| active_profile_ids.contains(profile_id));

        let max_coalesce = match event.kind {
            powershift_windows::ProcessEventKind::Started if !affects_inactive_profile => {
                return false;
            }
            powershift_windows::ProcessEventKind::Stopped if !affects_active_profile => {
                return false;
            }
            powershift_windows::ProcessEventKind::Stopped => ACTIVE_PROCESS_STOP_MAX_COALESCE,
            powershift_windows::ProcessEventKind::Started => PROCESS_EVENT_MAX_COALESCE,
        };
        self.schedule_debounced_scan(max_coalesce, affected.broad_only, now_ms);
        true
    }

    pub(crate) fn due(&self, now_ms: u64) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.force || pending.due_at_ms <= now_ms)
    }

    pub(crate) fn next_wait(&self, now_ms: u64) -> Option<Duration> {
        self.pending
            .as_ref()
            .map(|pending| Duration::from_millis(pending.due_at_ms.saturating_sub(now_ms)))
    }

    pub(crate) fn mark_scan_completed(&mut self, now_ms: u64) {
        if self.pending.as_ref().is_some_and(|pending| pending.broad) {
            self.last_broad_wake_ms = Some(now_ms);
        }
        self.pending = None;
    }

    fn broad_wake_on_cooldown(&self, now_ms: u64) -> bool {
        self.last_broad_wake_ms.is_some_and(|last| {
            now_ms.saturating_sub(last) < duration_ms(FOLDER_BROAD_WAKE_COOLDOWN)
        })
    }

    fn public_wake_on_cooldown(&self, now_ms: u64) -> bool {
        self.last_public_wake_ms.is_some_and(|last| {
            now_ms.saturating_sub(last) < duration_ms(PUBLIC_WAKE_EVENT_COOLDOWN)
        })
    }

    fn schedule_debounced_scan(&mut self, max_coalesce: Duration, broad: bool, now_ms: u64) {
        let first_seen_ms = self
            .pending
            .as_ref()
            .map(|pending| pending.first_seen_ms)
            .unwrap_or(now_ms);
        let mut due_at_ms = now_ms + duration_ms(PROCESS_EVENT_DEBOUNCE);
        due_at_ms = due_at_ms.min(first_seen_ms + duration_ms(max_coalesce));

        self.pending = Some(PendingScheduledScan {
            first_seen_ms,
            due_at_ms,
            force: false,
            broad,
        });
    }
}

pub(crate) fn next_wait_with_scheduler(
    state: &AgentRuntimeState,
    scheduler: &AgentScanScheduler,
    watcher_health_degraded: bool,
) -> Duration {
    let now = now_ms();
    let base = if watcher_health_degraded {
        next_wait_at(state, now).min(DEGRADED_PROCESS_SCAN_INTERVAL)
    } else {
        next_wait_at(state, now)
    };
    scheduler
        .next_wait(now)
        .map(|scheduler_wait| scheduler_wait.min(base))
        .unwrap_or(base)
}

pub(crate) fn active_profile_id_set(state: &AgentRuntimeState) -> BTreeSet<String> {
    state.active_profile_ids.iter().cloned().collect()
}

pub(crate) fn is_agent_wake_event(event: &powershift_windows::ProcessEvent) -> bool {
    event.pid == 0 && normalize_process_name(&event.name) == AGENT_WAKE_PROCESS_NAME
}

pub(crate) fn agent_wake_event() -> powershift_windows::ProcessEvent {
    powershift_windows::ProcessEvent {
        kind: powershift_windows::ProcessEventKind::Started,
        pid: 0,
        name: AGENT_WAKE_PROCESS_NAME.to_string(),
        path: None,
    }
}

pub(crate) fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn add_profile_executable(
    process_profiles: &mut BTreeMap<String, BTreeSet<String>>,
    broad_profiles: &mut BTreeSet<String>,
    process_name: &str,
    process_path: Option<&str>,
    match_mode: powershift_core::MatchMode,
    profile_id: &str,
) {
    if match_mode == powershift_core::MatchMode::Folder {
        broad_profiles.insert(profile_id.to_string());
    }

    for process_name in watch_process_names(process_name, process_path, match_mode) {
        add_profile_process(process_profiles, &process_name, profile_id);
    }
}

fn watch_process_names(
    process_name: &str,
    process_path: Option<&str>,
    match_mode: powershift_core::MatchMode,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if matches!(
        match_mode,
        powershift_core::MatchMode::Name | powershift_core::MatchMode::PathOrName
    ) {
        let name = normalize_process_name(process_name);
        if !name.is_empty() {
            names.insert(name);
        }
    }

    if matches!(
        match_mode,
        powershift_core::MatchMode::Path | powershift_core::MatchMode::PathOrName
    ) {
        if let Some(path_name) = process_path.and_then(file_name_from_path) {
            names.insert(path_name);
        }
    }

    if match_mode == powershift_core::MatchMode::Path && names.is_empty() {
        let name = normalize_process_name(process_name);
        if !name.is_empty() {
            names.insert(name);
        }
    }

    names
}

fn add_profile_process(
    process_profiles: &mut BTreeMap<String, BTreeSet<String>>,
    process_name: &str,
    profile_id: &str,
) {
    let process_name = normalize_process_name(process_name);
    if process_name.is_empty() {
        return;
    }
    process_profiles
        .entry(process_name)
        .or_default()
        .insert(profile_id.to_string());
}

fn file_name_from_path(path: &str) -> Option<String> {
    path.replace('/', "\\")
        .rsplit('\\')
        .next()
        .map(normalize_process_name)
        .filter(|name| !name.is_empty())
}

fn normalize_process_name(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}
