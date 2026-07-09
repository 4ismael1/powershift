use crate::{next_restore_wait_at, now_ms, AgentRuntimeState};
use powershift_core::AppConfig;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

pub(crate) const DEGRADED_PROCESS_SCAN_INTERVAL: Duration = Duration::from_secs(30);
pub(crate) const PUBLIC_WAKE_EVENT_COOLDOWN: Duration = Duration::from_secs(2);

const TARGETED_INSPECTION_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(150), Duration::from_secs(1)];

const AGENT_WAKE_PROCESS_NAME: &str = "agent_wake";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentWatchSet {
    process_profiles: BTreeMap<String, BTreeSet<String>>,
    broad_profiles: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AffectedWatchProfiles {
    pub(crate) profiles: BTreeSet<String>,
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
        AffectedWatchProfiles { profiles }
    }

    /// Fast name prefilter before asking Windows for metadata about a single
    /// PID. Folder matchers intentionally return true for every name because
    /// only the executable path can decide that match.
    pub(crate) fn should_observe(&self, process_name: &str) -> bool {
        !self.affected_profiles(process_name).profiles.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.process_profiles.is_empty() && self.broad_profiles.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentScanScheduler {
    pending: Option<PendingScheduledScan>,
    last_public_wake_ms: Option<u64>,
}

/// A bounded retry queue for a WMI start event whose process handle is not
/// ready yet. It inspects only that PID; it never enumerates all processes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TargetedInspectionQueue {
    pending: BTreeMap<u32, PendingTargetedInspection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DueTargetedInspection {
    pub(crate) pid: u32,
    pub(crate) name: String,
    attempt: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingTargetedInspection {
    name: String,
    attempt: usize,
    due_at_ms: u64,
}

impl TargetedInspectionQueue {
    pub(crate) fn schedule_initial(&mut self, pid: u32, name: impl Into<String>, now_ms: u64) {
        if pid == 0 {
            return;
        }
        self.pending.insert(
            pid,
            PendingTargetedInspection {
                name: name.into(),
                attempt: 0,
                due_at_ms: now_ms + TARGETED_INSPECTION_RETRY_DELAYS[0].as_millis() as u64,
            },
        );
    }

    pub(crate) fn take_due(&mut self, now_ms: u64) -> Vec<DueTargetedInspection> {
        let due_pids = self
            .pending
            .iter()
            .filter_map(|(pid, pending)| (pending.due_at_ms <= now_ms).then_some(*pid))
            .collect::<Vec<_>>();
        due_pids
            .into_iter()
            .filter_map(|pid| {
                self.pending
                    .remove(&pid)
                    .map(|pending| DueTargetedInspection {
                        pid,
                        name: pending.name,
                        attempt: pending.attempt,
                    })
            })
            .collect()
    }

    pub(crate) fn reschedule_after_failure(
        &mut self,
        inspection: DueTargetedInspection,
        now_ms: u64,
    ) {
        let next_attempt = inspection.attempt + 1;
        let Some(delay) = TARGETED_INSPECTION_RETRY_DELAYS.get(next_attempt) else {
            return;
        };
        self.pending.insert(
            inspection.pid,
            PendingTargetedInspection {
                name: inspection.name,
                attempt: next_attempt,
                due_at_ms: now_ms + delay.as_millis() as u64,
            },
        );
    }

    pub(crate) fn remove(&mut self, pid: u32) {
        self.pending.remove(&pid);
    }

    pub(crate) fn next_wait(&self, now_ms: u64) -> Option<Duration> {
        self.pending
            .values()
            .map(|pending| Duration::from_millis(pending.due_at_ms.saturating_sub(now_ms)))
            .min()
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingScheduledScan {
    due_at_ms: u64,
    force: bool,
}

impl AgentScanScheduler {
    pub(crate) fn schedule_forced(&mut self, now_ms: u64) {
        self.pending = Some(PendingScheduledScan {
            due_at_ms: now_ms,
            force: true,
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

    pub(crate) fn mark_scan_completed(&mut self, _now_ms: u64) {
        self.pending = None;
    }

    fn public_wake_on_cooldown(&self, now_ms: u64) -> bool {
        self.last_public_wake_ms.is_some_and(|last| {
            now_ms.saturating_sub(last) < PUBLIC_WAKE_EVENT_COOLDOWN.as_millis() as u64
        })
    }
}

pub(crate) fn next_wait_with_scheduler(
    state: &AgentRuntimeState,
    scheduler: &AgentScanScheduler,
    watcher_health_degraded: bool,
) -> Option<Duration> {
    let now = now_ms();
    let restore_wait = next_restore_wait_at(state, now);
    let base = if watcher_health_degraded {
        Some(
            restore_wait
                .unwrap_or(DEGRADED_PROCESS_SCAN_INTERVAL)
                .min(DEGRADED_PROCESS_SCAN_INTERVAL),
        )
    } else {
        restore_wait
    };
    minimum_wait(base, scheduler.next_wait(now))
}

pub(crate) fn minimum_wait(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(wait), None) | (None, Some(wait)) => Some(wait),
        (None, None) => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targeted_inspection_retries_only_the_original_pid_and_is_bounded() {
        let mut queue = TargetedInspectionQueue::default();
        queue.schedule_initial(42, "game.exe", 1_000);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.next_wait(1_000), Some(Duration::from_millis(150)));
        assert!(queue.take_due(1_149).is_empty());

        let first = queue.take_due(1_150);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].pid, 42);
        assert_eq!(first[0].name, "game.exe");
        queue.reschedule_after_failure(first.into_iter().next().expect("first retry"), 1_150);

        assert_eq!(queue.next_wait(1_150), Some(Duration::from_secs(1)));
        let second = queue.take_due(2_150);
        assert_eq!(second.len(), 1);
        queue.reschedule_after_failure(second.into_iter().next().expect("second retry"), 2_150);

        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn stop_event_can_cancel_a_targeted_inspection_before_it_runs() {
        let mut queue = TargetedInspectionQueue::default();
        queue.schedule_initial(42, "game.exe", 1_000);

        queue.remove(42);

        assert!(queue.take_due(10_000).is_empty());
    }

    #[test]
    fn optional_waits_block_indefinitely_when_no_deadline_exists() {
        assert_eq!(minimum_wait(None, None), None);
        assert_eq!(
            minimum_wait(Some(Duration::from_secs(5)), None),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            minimum_wait(Some(Duration::from_secs(5)), Some(Duration::from_secs(2))),
            Some(Duration::from_secs(2))
        );
    }
}
