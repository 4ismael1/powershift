use powershift_core::{
    resolve_active_profiles, ActiveProfile, AgentStatus, AppConfig, ConfigStore, DetectedProcess,
    RestoreBehavior,
};
mod ipc;
mod paths;
mod publisher;
mod scheduler;

pub use ipc::{
    request_agent_reevaluate_via_ipc, request_agent_shutdown_via_ipc, request_agent_status_via_ipc,
    AgentIpcRequest, AgentIpcResponse,
};
pub use paths::AgentPaths;
pub use publisher::{append_event_to_path, publish_state, EventLogEntry};

#[cfg(test)]
use ipc::handle_agent_ipc_request;
use ipc::{load_or_create_control_token, spawn_agent_ipc_server, AgentSharedState};
use powershift_windows::{
    create_agent_wake_event, spawn_process_event_watchers, wait_for_agent_wake, PowerManager,
    PowerManagerBackend, ProcessSnapshotBackend, ProcessWatchMessage, SystemProcessBackend,
};
#[cfg(test)]
use publisher::{
    agent_error_message, publish_error, publish_heartbeat, publish_scan_outcome, scan_event_entry,
    MAX_EVENT_LOG_BYTES,
};
use publisher::{
    publish_error_with_shared, publish_heartbeat_with_shared, publish_scan_outcome_with_shared,
    write_scan_event, AgentPublishMemory,
};
use scheduler::{
    active_profile_id_set, agent_wake_event, is_agent_wake_event, next_wait_with_scheduler,
    AgentScanScheduler, AgentWatchSet,
};
#[cfg(test)]
use scheduler::{
    duration_ms, DEGRADED_PROCESS_SCAN_INTERVAL, PROCESS_EVENT_DEBOUNCE, PUBLIC_WAKE_EVENT_COOLDOWN,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

const AGENT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentScanResult {
    pub matched_profile_id: Option<String>,
    pub matched_profile_name: Option<String>,
    pub target_plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_profile_name: Option<String>,
    #[serde(default)]
    pub active_profiles: Vec<AgentActiveProfile>,
    pub changed_power_plan: bool,
    pub restore_scheduled: bool,
    pub restored_power_plan: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActiveProfile {
    pub profile_id: String,
    pub profile_name: String,
    pub plan_id: String,
    pub priority: u8,
    pub matched_processes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentRuntimeState {
    pub active_profile_ids: Vec<String>,
    pub winning_profile_id: Option<String>,
    pub previous_plan_id: Option<String>,
    pub pending_restore: Option<PendingRestoreState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRestoreState {
    pub due_at_ms: u64,
    pub plan_id: String,
    pub profile_id: String,
    pub profile_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedAgentState {
    pub pid: u32,
    pub status: AgentStatus,
    pub updated_at_ms: u64,
    pub last_scan: Option<AgentScanResult>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AgentWatcherHealth {
    degraded_watchers: BTreeSet<powershift_windows::ProcessWatcherKind>,
}

impl AgentWatcherHealth {
    fn mark_healthy(&mut self, kind: powershift_windows::ProcessWatcherKind) {
        self.degraded_watchers.remove(&kind);
    }

    fn mark_degraded(&mut self, kind: powershift_windows::ProcessWatcherKind) {
        self.degraded_watchers.insert(kind);
    }

    fn is_degraded(&self) -> bool {
        !self.degraded_watchers.is_empty()
    }
}

pub fn run_agent_forever() -> Result<(), String> {
    let paths = AgentPaths::from_app_data();
    let mut runtime = AgentRuntimeState::default();
    let mut publish_memory = AgentPublishMemory::default();
    let mut scheduler = AgentScanScheduler::default();
    let mut watcher_health = AgentWatcherHealth::default();
    let shared_state = AgentSharedState::default();
    let process_backend = SystemProcessBackend;
    let power_backend = PowerManager::new();
    let control_token = load_or_create_control_token(&paths.control_token())?;

    let starting_state = PublishedAgentState {
        pid: std::process::id(),
        status: AgentStatus::Starting,
        updated_at_ms: now_ms(),
        last_scan: None,
        last_error: None,
    };
    shared_state.set(starting_state.clone());
    publish_state(&paths.state, starting_state)?;

    let (sender, receiver) = mpsc::channel();
    spawn_agent_ipc_server(sender.clone(), shared_state.clone(), control_token);
    let _process_watchers = spawn_process_event_watchers(sender.clone());
    let wake_sender = sender.clone();
    std::thread::spawn(move || {
        let wake_handle = match create_agent_wake_event() {
            Ok(handle) => handle,
            Err(error) => {
                let _ = wake_sender.send(ProcessWatchMessage::Error(error.to_string()));
                return;
            }
        };

        loop {
            match wait_for_agent_wake(wake_handle) {
                Ok(()) => {
                    if wake_sender
                        .send(ProcessWatchMessage::Event(agent_wake_event()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = wake_sender.send(ProcessWatchMessage::Error(error.to_string()));
                    break;
                }
            }
        }
    });

    let (scan_result, mut watch_set) =
        run_agent_scan_cycle_with_paths(&paths, &process_backend, &power_backend, &mut runtime);
    publish_scan_outcome_with_shared(
        &paths,
        scan_result,
        &mut publish_memory,
        Some(&shared_state),
    )?;

    loop {
        match receiver.recv_timeout(next_wait_with_scheduler(
            &runtime,
            &scheduler,
            watcher_health.is_degraded(),
        )) {
            Ok(ProcessWatchMessage::Event(event)) => {
                if is_agent_wake_event(&event) {
                    scheduler.record_public_wake(now_ms());
                } else {
                    scheduler.record_process_event(
                        &event,
                        &watch_set,
                        &active_profile_id_set(&runtime),
                        now_ms(),
                    );
                }

                if scheduler.due(now_ms()) {
                    let (scan_result, next_watch_set) = run_agent_scan_cycle_with_paths(
                        &paths,
                        &process_backend,
                        &power_backend,
                        &mut runtime,
                    );
                    publish_scan_outcome_with_shared(
                        &paths,
                        scan_result,
                        &mut publish_memory,
                        Some(&shared_state),
                    )?;
                    scheduler.mark_scan_completed(now_ms());
                    watch_set = next_watch_set;
                }
            }
            Ok(ProcessWatchMessage::Reevaluate) => {
                scheduler.schedule_forced(now_ms());
                let (scan_result, next_watch_set) = run_agent_scan_cycle_with_paths(
                    &paths,
                    &process_backend,
                    &power_backend,
                    &mut runtime,
                );
                publish_scan_outcome_with_shared(
                    &paths,
                    scan_result,
                    &mut publish_memory,
                    Some(&shared_state),
                )?;
                scheduler.mark_scan_completed(now_ms());
                watch_set = next_watch_set;
            }
            Ok(ProcessWatchMessage::Error(error)) => {
                publish_error_with_shared(&paths, &error, &mut publish_memory, Some(&shared_state))?
            }
            Ok(ProcessWatchMessage::WatcherHealthy(kind)) => {
                watcher_health.mark_healthy(kind);
                scheduler.schedule_forced(now_ms());
            }
            Ok(ProcessWatchMessage::WatcherDegraded { kind, error, .. }) => {
                watcher_health.mark_degraded(kind);
                scheduler.schedule_forced(now_ms());
                publish_error_with_shared(
                    &paths,
                    &error,
                    &mut publish_memory,
                    Some(&shared_state),
                )?;
            }
            Ok(ProcessWatchMessage::Shutdown) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                if scheduler.due(now_ms()) {
                    let (scan_result, next_watch_set) = run_agent_scan_cycle_with_paths(
                        &paths,
                        &process_backend,
                        &power_backend,
                        &mut runtime,
                    );
                    publish_scan_outcome_with_shared(
                        &paths,
                        scan_result,
                        &mut publish_memory,
                        Some(&shared_state),
                    )?;
                    scheduler.mark_scan_completed(now_ms());
                    watch_set = next_watch_set;
                } else if restore_due(&runtime) || watcher_health.is_degraded() {
                    let (scan_result, next_watch_set) = run_agent_scan_cycle_with_paths(
                        &paths,
                        &process_backend,
                        &power_backend,
                        &mut runtime,
                    );
                    publish_scan_outcome_with_shared(
                        &paths,
                        scan_result,
                        &mut publish_memory,
                        Some(&shared_state),
                    )?;
                    watch_set = next_watch_set;
                } else {
                    publish_heartbeat_with_shared(&paths, &publish_memory, Some(&shared_state))?;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

pub fn run_scan_once() -> Result<AgentScanResult, String> {
    let paths = AgentPaths::from_app_data();
    let mut runtime = AgentRuntimeState::default();
    run_agent_scan_with_paths(
        &paths,
        &SystemProcessBackend,
        &PowerManager::new(),
        &mut runtime,
    )
}

pub fn run_agent_scan_with_paths<P, W>(
    paths: &AgentPaths,
    process_backend: &P,
    power_backend: &W,
    state: &mut AgentRuntimeState,
) -> Result<AgentScanResult, String>
where
    P: ProcessSnapshotBackend,
    W: PowerManagerBackend,
{
    let config = load_or_create_config(paths.config.clone())?;
    let result =
        evaluate_agent_scan_stateful(&config, process_backend, power_backend, state, now_ms());
    write_scan_event(&paths.events, &result, power_backend);
    result
}

fn run_agent_scan_cycle_with_paths<P, W>(
    paths: &AgentPaths,
    process_backend: &P,
    power_backend: &W,
    state: &mut AgentRuntimeState,
) -> (Result<AgentScanResult, String>, AgentWatchSet)
where
    P: ProcessSnapshotBackend,
    W: PowerManagerBackend,
{
    let config = match load_or_create_config(paths.config.clone()) {
        Ok(config) => config,
        Err(error) => return (Err(error), AgentWatchSet::default()),
    };
    let watch_set = AgentWatchSet::from_config(&config);
    let result =
        evaluate_agent_scan_stateful(&config, process_backend, power_backend, state, now_ms());
    write_scan_event(&paths.events, &result, power_backend);
    (result, watch_set)
}

pub fn evaluate_agent_scan_stateful<P, W>(
    config: &AppConfig,
    process_backend: &P,
    power_backend: &W,
    state: &mut AgentRuntimeState,
    now_ms: u64,
) -> Result<AgentScanResult, String>
where
    P: ProcessSnapshotBackend,
    W: PowerManagerBackend,
{
    let processes = process_backend
        .list_processes()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|process| DetectedProcess {
            pid: process.pid,
            name: process.name,
            path: process.path,
        })
        .collect::<Vec<_>>();
    let active_plan = power_backend
        .active_plan()
        .map_err(|error| error.to_string())?;

    let active_profiles = resolve_active_profiles(config, &processes);

    if let Some(winner) =
        choose_winning_profile(&active_profiles, state.winning_profile_id.as_deref())
    {
        if state.active_profile_ids.is_empty() && state.pending_restore.is_none() {
            state.previous_plan_id = Some(active_plan.id.clone());
        }
        state.active_profile_ids = active_profiles
            .iter()
            .map(|profile| profile.profile_id.clone())
            .collect();
        state.winning_profile_id = Some(winner.profile_id.clone());
        state.pending_restore = None;

        let changed_power_plan = active_plan.id != winner.plan_id;
        if changed_power_plan {
            power_backend
                .set_active_plan(&winner.plan_id)
                .map_err(|error| error.to_string())?;
        }

        return Ok(AgentScanResult {
            matched_profile_id: Some(winner.profile_id.clone()),
            matched_profile_name: Some(winner.name.clone()),
            target_plan_id: Some(winner.plan_id.clone()),
            restore_profile_name: None,
            active_profiles: active_profiles
                .iter()
                .map(AgentActiveProfile::from)
                .collect(),
            changed_power_plan,
            restore_scheduled: false,
            restored_power_plan: false,
        });
    }

    state.active_profile_ids.clear();

    if let Some(restore) = state.pending_restore.clone() {
        if now_ms >= restore.due_at_ms {
            let changed_power_plan = active_plan.id != restore.plan_id;
            if changed_power_plan {
                power_backend
                    .set_active_plan(&restore.plan_id)
                    .map_err(|error| error.to_string())?;
            }
            state.pending_restore = None;
            state.previous_plan_id = None;
            return Ok(AgentScanResult {
                matched_profile_id: None,
                matched_profile_name: None,
                target_plan_id: Some(restore.plan_id),
                restore_profile_name: Some(restore.profile_name),
                active_profiles: Vec::new(),
                changed_power_plan,
                restore_scheduled: false,
                restored_power_plan: true,
            });
        }
    }

    if let Some(profile_id) = state.winning_profile_id.take() {
        if let Some(profile) = config
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
        {
            if let Some(plan_id) = restore_plan_for(profile, state.previous_plan_id.as_deref()) {
                state.pending_restore = Some(PendingRestoreState {
                    due_at_ms: now_ms + u64::from(profile.power.close_delay_seconds) * 1000,
                    plan_id,
                    profile_id: profile.id.clone(),
                    profile_name: profile.name.clone(),
                });
                return Ok(AgentScanResult {
                    matched_profile_id: None,
                    matched_profile_name: None,
                    target_plan_id: None,
                    restore_profile_name: Some(profile.name.clone()),
                    active_profiles: Vec::new(),
                    changed_power_plan: false,
                    restore_scheduled: true,
                    restored_power_plan: false,
                });
            }
            state.previous_plan_id = None;
        }
    }

    Ok(AgentScanResult {
        matched_profile_id: None,
        matched_profile_name: None,
        target_plan_id: None,
        restore_profile_name: None,
        active_profiles: Vec::new(),
        changed_power_plan: false,
        restore_scheduled: false,
        restored_power_plan: false,
    })
}

impl From<&ActiveProfile> for AgentActiveProfile {
    fn from(profile: &ActiveProfile) -> Self {
        let mut seen_process_names = BTreeSet::new();
        let matched_processes = profile
            .matched_processes
            .iter()
            .filter_map(|process| {
                let name = process.name.trim();
                if name.is_empty() {
                    return None;
                }
                if seen_process_names.insert(name.to_ascii_lowercase()) {
                    Some(process.name.clone())
                } else {
                    None
                }
            })
            .collect();

        Self {
            profile_id: profile.profile_id.clone(),
            profile_name: profile.name.clone(),
            plan_id: profile.plan_id.clone(),
            priority: profile.priority,
            matched_processes,
        }
    }
}

fn choose_winning_profile<'a>(
    active_profiles: &'a [ActiveProfile],
    current_winner_id: Option<&str>,
) -> Option<&'a ActiveProfile> {
    let max_priority = active_profiles
        .iter()
        .map(|profile| profile.priority)
        .max()?;

    if let Some(current_winner_id) = current_winner_id {
        if let Some(current) = active_profiles.iter().find(|profile| {
            profile.profile_id == current_winner_id && profile.priority == max_priority
        }) {
            return Some(current);
        }
    }

    active_profiles
        .iter()
        .find(|profile| profile.priority == max_priority)
}

fn restore_plan_for(
    profile: &powershift_core::Profile,
    previous_plan_id: Option<&str>,
) -> Option<String> {
    match profile.power.on_close_behavior {
        RestoreBehavior::PreviousPlan => previous_plan_id.map(ToOwned::to_owned),
        RestoreBehavior::SpecificPlan => profile.power.on_close_plan_id.clone(),
        RestoreBehavior::DoNothing => None,
    }
}

fn next_wait_at(state: &AgentRuntimeState, now_ms: u64) -> Duration {
    let restore_delay = state
        .pending_restore
        .as_ref()
        .map(|restore| Duration::from_millis(restore.due_at_ms.saturating_sub(now_ms)));
    restore_delay
        .map(|delay| delay.min(AGENT_HEARTBEAT_INTERVAL))
        .unwrap_or(AGENT_HEARTBEAT_INTERVAL)
}

fn restore_due(state: &AgentRuntimeState) -> bool {
    restore_due_at(state, now_ms())
}

fn restore_due_at(state: &AgentRuntimeState, now_ms: u64) -> bool {
    state
        .pending_restore
        .as_ref()
        .is_some_and(|restore| restore.due_at_ms <= now_ms)
}

fn load_or_create_config(path: PathBuf) -> Result<AppConfig, String> {
    if path.exists() {
        return ConfigStore::load(path).map_err(|error| error.to_string());
    }

    let config = AppConfig::default();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    ConfigStore::save(path, &config).map_err(|error| error.to_string())?;
    Ok(config)
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use powershift_core::{PowerPlan, ProcessInfo, Profile};
    use powershift_windows::{PowerError, PowerResult, ProcessEvent, ProcessEventKind};
    use std::cell::RefCell;

    struct FakeProcessBackend {
        processes: Vec<ProcessInfo>,
    }

    impl ProcessSnapshotBackend for FakeProcessBackend {
        fn list_processes(&self) -> PowerResult<Vec<ProcessInfo>> {
            Ok(self.processes.clone())
        }
    }

    struct FakePowerBackend {
        active: PowerPlan,
        set_calls: RefCell<Vec<String>>,
    }

    impl PowerManagerBackend for FakePowerBackend {
        fn list_plans(&self) -> PowerResult<Vec<PowerPlan>> {
            Ok(vec![self.active.clone()])
        }

        fn active_plan(&self) -> PowerResult<PowerPlan> {
            Ok(self.active.clone())
        }

        fn set_active_plan(&self, plan_id: &str) -> PowerResult<()> {
            self.set_calls.borrow_mut().push(plan_id.to_string());
            Ok(())
        }
    }

    fn config() -> AppConfig {
        AppConfig {
            profiles: vec![Profile::new("demo", "Demo Game", "demo.exe", "high")],
            ..AppConfig::default()
        }
    }

    fn process(name: &str) -> ProcessInfo {
        process_with_pid(10, name)
    }

    fn process_with_pid(pid: u32, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            path: None,
        }
    }

    fn multi_profile_config() -> AppConfig {
        let mut chrome = Profile::new("chrome", "Chrome", "chrome.exe", "balanced");
        chrome.power.priority = 20;
        let mut game = Profile::new("game", "Game", "game.exe", "high");
        game.power.priority = 90;

        AppConfig {
            profiles: vec![chrome, game],
            ..AppConfig::default()
        }
    }

    fn power(active_id: &str) -> FakePowerBackend {
        FakePowerBackend {
            active: PowerPlan {
                id: active_id.to_string(),
                name: active_id.to_string(),
            },
            set_calls: RefCell::new(Vec::new()),
        }
    }

    fn process_event(name: &str) -> ProcessEvent {
        process_event_with_kind(name, ProcessEventKind::Started)
    }

    fn process_stopped_event(name: &str) -> ProcessEvent {
        process_event_with_kind(name, ProcessEventKind::Stopped)
    }

    fn process_event_with_kind(name: &str, kind: ProcessEventKind) -> ProcessEvent {
        ProcessEvent {
            kind,
            pid: 42,
            name: name.to_string(),
            path: None,
        }
    }

    fn active_ids(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn temp_agent_paths(name: &str) -> AgentPaths {
        let base =
            std::env::temp_dir().join(format!("powershift-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create temp agent dir");
        AgentPaths {
            config: base.join("config.json"),
            events: base.join("events.jsonl"),
            state: base.join("agent-state.json"),
        }
    }

    fn read_published_state(paths: &AgentPaths) -> PublishedAgentState {
        let value = std::fs::read_to_string(&paths.state).expect("read state");
        serde_json::from_str(&value).expect("parse state")
    }

    #[test]
    fn scan_cycle_reuses_loaded_config_for_next_watch_set() {
        let paths = temp_agent_paths("scan-cycle-watch-set");
        ConfigStore::save(&paths.config, &multi_profile_config()).expect("save config");
        let processes = FakeProcessBackend {
            processes: vec![process("game.exe")],
        };
        let power = power("balanced");
        let mut state = AgentRuntimeState::default();

        let (result, watch_set) =
            run_agent_scan_cycle_with_paths(&paths, &processes, &power, &mut state);
        let scan = result.expect("scan result");

        assert_eq!(scan.matched_profile_id.as_deref(), Some("game"));
        assert!(watch_set
            .affected_profiles("game.exe")
            .profiles
            .contains("game"));
        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn published_active_profile_deduplicates_process_names() {
        let active = ActiveProfile {
            profile_id: "chrome".to_string(),
            name: "Chrome".to_string(),
            plan_id: "high".to_string(),
            priority: 70,
            matched_processes: vec![
                DetectedProcess::new(10, "chrome.exe", None::<String>),
                DetectedProcess::new(11, "CHROME.EXE", None::<String>),
                DetectedProcess::new(12, "renderer.exe", None::<String>),
            ],
        };

        let published = AgentActiveProfile::from(&active);

        assert_eq!(
            published.matched_processes,
            vec!["chrome.exe".to_string(), "renderer.exe".to_string()]
        );
    }

    fn active_scan() -> AgentScanResult {
        AgentScanResult {
            matched_profile_id: Some("demo".to_string()),
            matched_profile_name: Some("Demo Game".to_string()),
            target_plan_id: Some("high".to_string()),
            restore_profile_name: None,
            active_profiles: vec![AgentActiveProfile {
                profile_id: "demo".to_string(),
                profile_name: "Demo Game".to_string(),
                plan_id: "high".to_string(),
                priority: 70,
                matched_processes: vec!["demo.exe".to_string()],
            }],
            changed_power_plan: true,
            restore_scheduled: false,
            restored_power_plan: false,
        }
    }

    fn shared_state_with(state: PublishedAgentState) -> AgentSharedState {
        let shared = AgentSharedState::default();
        shared.set(state);
        shared
    }

    const IPC_TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn ipc_status_returns_live_memory_state() {
        let state = PublishedAgentState {
            pid: 44,
            status: AgentStatus::Running,
            updated_at_ms: 123,
            last_scan: Some(active_scan()),
            last_error: None,
        };
        let shared = shared_state_with(state.clone());
        let (sender, _receiver) = mpsc::channel();
        let request = serde_json::to_string(&AgentIpcRequest::GetStatus).expect("request json");

        let response: AgentIpcResponse = serde_json::from_str(&handle_agent_ipc_request(
            &request,
            &shared,
            &sender,
            IPC_TEST_TOKEN,
        ))
        .expect("response json");

        assert!(response.ok);
        assert_eq!(response.state, Some(state));
    }

    #[test]
    fn ipc_reevaluate_sends_forced_wake_message() {
        let shared = shared_state_with(PublishedAgentState {
            pid: 44,
            status: AgentStatus::Running,
            updated_at_ms: 123,
            last_scan: None,
            last_error: None,
        });
        let (sender, receiver) = mpsc::channel();
        let request = serde_json::to_string(&AgentIpcRequest::Reevaluate {
            token: Some(IPC_TEST_TOKEN.to_string()),
        })
        .expect("request json");

        let response: AgentIpcResponse = serde_json::from_str(&handle_agent_ipc_request(
            &request,
            &shared,
            &sender,
            IPC_TEST_TOKEN,
        ))
        .expect("response json");

        assert!(response.ok);
        assert_eq!(
            receiver.recv().expect("wake message"),
            ProcessWatchMessage::Reevaluate
        );
    }

    #[test]
    fn ipc_shutdown_sends_shutdown_message() {
        let shared = AgentSharedState::default();
        let (sender, receiver) = mpsc::channel();
        let request = serde_json::to_string(&AgentIpcRequest::Shutdown {
            token: Some(IPC_TEST_TOKEN.to_string()),
        })
        .expect("request json");

        let response: AgentIpcResponse = serde_json::from_str(&handle_agent_ipc_request(
            &request,
            &shared,
            &sender,
            IPC_TEST_TOKEN,
        ))
        .expect("response json");

        assert!(response.ok);
        assert_eq!(
            receiver.recv().expect("shutdown message"),
            ProcessWatchMessage::Shutdown
        );
    }

    #[test]
    fn ipc_mutations_reject_missing_or_invalid_control_token() {
        let shared = AgentSharedState::default();
        let (sender, receiver) = mpsc::channel();
        let request = serde_json::to_string(&AgentIpcRequest::Shutdown {
            token: Some("wrong".to_string()),
        })
        .expect("request json");

        let response: AgentIpcResponse = serde_json::from_str(&handle_agent_ipc_request(
            &request,
            &shared,
            &sender,
            IPC_TEST_TOKEN,
        ))
        .expect("response json");

        assert!(!response.ok);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn scan_changes_power_plan_when_profile_matches() {
        let processes = FakeProcessBackend {
            processes: vec![process("demo.exe")],
        };
        let power = power("balanced");
        let mut state = AgentRuntimeState::default();

        let result = evaluate_agent_scan_stateful(&config(), &processes, &power, &mut state, 1)
            .expect("scan");

        assert_eq!(result.matched_profile_id.as_deref(), Some("demo"));
        assert_eq!(result.active_profiles.len(), 1);
        assert_eq!(state.active_profile_ids, vec!["demo".to_string()]);
        assert_eq!(state.winning_profile_id.as_deref(), Some("demo"));
        assert!(result.changed_power_plan);
        assert_eq!(power.set_calls.borrow().as_slice(), &["high".to_string()]);
    }

    #[test]
    fn scan_publishes_all_active_profiles_and_chooses_highest_priority_winner() {
        let processes = FakeProcessBackend {
            processes: vec![
                process_with_pid(1, "chrome.exe"),
                process_with_pid(2, "game.exe"),
            ],
        };
        let power = power("balanced");
        let mut state = AgentRuntimeState::default();

        let result = evaluate_agent_scan_stateful(
            &multi_profile_config(),
            &processes,
            &power,
            &mut state,
            1,
        )
        .expect("scan");

        assert_eq!(result.matched_profile_id.as_deref(), Some("game"));
        assert_eq!(result.target_plan_id.as_deref(), Some("high"));
        assert_eq!(
            result
                .active_profiles
                .iter()
                .map(|profile| profile.profile_id.as_str())
                .collect::<Vec<_>>(),
            vec!["chrome", "game"]
        );
        assert_eq!(state.active_profile_ids, vec!["chrome", "game"]);
        assert_eq!(state.winning_profile_id.as_deref(), Some("game"));
        assert_eq!(power.set_calls.borrow().as_slice(), &["high".to_string()]);
    }

    #[test]
    fn lower_priority_profile_does_not_override_current_high_priority_winner() {
        let config = multi_profile_config();
        let mut state = AgentRuntimeState::default();
        let first_power = power("balanced");
        evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: vec![process_with_pid(2, "game.exe")],
            },
            &first_power,
            &mut state,
            1,
        )
        .expect("activate game");

        let second_power = power("high");
        let result = evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: vec![
                    process_with_pid(1, "chrome.exe"),
                    process_with_pid(2, "game.exe"),
                ],
            },
            &second_power,
            &mut state,
            2,
        )
        .expect("scan with both active");

        assert_eq!(result.matched_profile_id.as_deref(), Some("game"));
        assert_eq!(
            result
                .active_profiles
                .iter()
                .map(|profile| profile.profile_id.as_str())
                .collect::<Vec<_>>(),
            vec!["chrome", "game"]
        );
        assert!(second_power.set_calls.borrow().is_empty());
    }

    #[test]
    fn lower_priority_profile_takes_control_when_high_priority_winner_closes() {
        let config = multi_profile_config();
        let mut state = AgentRuntimeState::default();
        evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: vec![
                    process_with_pid(1, "chrome.exe"),
                    process_with_pid(2, "game.exe"),
                ],
            },
            &power("balanced"),
            &mut state,
            1,
        )
        .expect("activate game");

        let power = power("high");
        let result = evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: vec![process_with_pid(1, "chrome.exe")],
            },
            &power,
            &mut state,
            2,
        )
        .expect("fallback to chrome");

        assert_eq!(result.matched_profile_id.as_deref(), Some("chrome"));
        assert_eq!(result.active_profiles.len(), 1);
        assert_eq!(state.active_profile_ids, vec!["chrome".to_string()]);
        assert_eq!(state.winning_profile_id.as_deref(), Some("chrome"));
        assert_eq!(
            power.set_calls.borrow().as_slice(),
            &["balanced".to_string()]
        );
    }

    #[test]
    fn restore_is_scheduled_only_after_all_active_profiles_close() {
        let config = multi_profile_config();
        let mut state = AgentRuntimeState::default();
        evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: vec![
                    process_with_pid(1, "chrome.exe"),
                    process_with_pid(2, "game.exe"),
                ],
            },
            &power("balanced"),
            &mut state,
            1_000,
        )
        .expect("activate both");

        let still_active = evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: vec![process_with_pid(1, "chrome.exe")],
            },
            &power("high"),
            &mut state,
            2_000,
        )
        .expect("fallback while chrome remains active");

        assert!(!still_active.restore_scheduled);
        assert_eq!(still_active.matched_profile_id.as_deref(), Some("chrome"));

        let idle = evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: Vec::new(),
            },
            &power("balanced"),
            &mut state,
            3_000,
        )
        .expect("schedule restore");

        assert!(idle.restore_scheduled);
        assert_eq!(
            state.pending_restore,
            Some(PendingRestoreState {
                due_at_ms: 33_000,
                plan_id: "balanced".to_string(),
                profile_id: "chrome".to_string(),
                profile_name: "Chrome".to_string(),
            })
        );
        assert_eq!(idle.restore_profile_name.as_deref(), Some("Chrome"));
    }

    #[test]
    fn equal_priority_conflicts_keep_the_current_winner_stable() {
        let mut game = Profile::new("game", "Game", "game.exe", "high");
        game.power.priority = 70;
        let mut chrome = Profile::new("chrome", "Chrome", "chrome.exe", "balanced");
        chrome.power.priority = 70;
        let config = AppConfig {
            profiles: vec![game, chrome],
            ..AppConfig::default()
        };
        let mut state = AgentRuntimeState {
            active_profile_ids: vec!["chrome".to_string()],
            winning_profile_id: Some("chrome".to_string()),
            previous_plan_id: Some("balanced".to_string()),
            pending_restore: None,
        };

        let result = evaluate_agent_scan_stateful(
            &config,
            &FakeProcessBackend {
                processes: vec![
                    process_with_pid(1, "game.exe"),
                    process_with_pid(2, "chrome.exe"),
                ],
            },
            &power("balanced"),
            &mut state,
            1,
        )
        .expect("scan");

        assert_eq!(result.matched_profile_id.as_deref(), Some("chrome"));
        assert_eq!(state.winning_profile_id.as_deref(), Some("chrome"));
    }

    #[test]
    fn watch_set_indexes_configured_process_names() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());

        assert!(!watch_set.affected_profiles("chrome.exe").is_empty());
        assert!(!watch_set.affected_profiles("game.exe").is_empty());
        assert!(watch_set.affected_profiles("notepad.exe").is_empty());
    }

    #[test]
    fn watch_set_indexes_file_names_from_path_matchers() {
        let mut profile = Profile::new("path-game", "Path Game", "wrong.exe", "high");
        profile.main_executable.path = Some("D:\\Games\\Real\\real-game.exe".to_string());
        profile.activation.match_mode = powershift_core::MatchMode::Path;
        let watch_set = AgentWatchSet::from_config(&AppConfig {
            profiles: vec![profile],
            ..AppConfig::default()
        });

        assert!(watch_set.affected_profiles("wrong.exe").is_empty());
        assert_eq!(
            watch_set.affected_profiles("real-game.exe").profiles,
            BTreeSet::from(["path-game".to_string()])
        );
    }

    #[test]
    fn watch_set_uses_broad_wake_for_folder_matchers() {
        let mut profile = Profile::new("folder-game", "Folder Game", "launcher.exe", "high");
        profile.activation.require_main_process = false;
        profile
            .associated_processes
            .push(powershift_core::ProcessMatcher {
                name: String::new(),
                path: Some("D:\\Games\\Folder".to_string()),
                match_mode: powershift_core::MatchMode::Folder,
            });
        let watch_set = AgentWatchSet::from_config(&AppConfig {
            profiles: vec![profile],
            ..AppConfig::default()
        });

        let affected = watch_set.affected_profiles("unexpected-helper.exe");
        assert!(affected.broad_only);
        assert_eq!(
            affected.profiles,
            BTreeSet::from(["folder-game".to_string()])
        );
    }

    #[test]
    fn watch_set_is_empty_when_automation_is_disabled() {
        let mut config = multi_profile_config();
        config.automation.enabled = false;

        let watch_set = AgentWatchSet::from_config(&config);

        assert!(watch_set.is_empty());
    }

    #[test]
    fn scheduler_ignores_unconfigured_process_events() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        let scheduled = scheduler.record_process_event(
            &process_event("notepad.exe"),
            &watch_set,
            &BTreeSet::new(),
            1_000,
        );

        assert!(!scheduled);
        assert_eq!(scheduler.next_wait(1_000), None);
    }

    #[test]
    fn scheduler_coalesces_process_event_bursts() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        assert!(scheduler.record_process_event(
            &process_event("game.exe"),
            &watch_set,
            &BTreeSet::new(),
            1_000,
        ));
        assert_eq!(scheduler.next_wait(1_000), Some(PROCESS_EVENT_DEBOUNCE));

        scheduler.record_process_event(
            &process_event("game.exe"),
            &watch_set,
            &BTreeSet::new(),
            1_400,
        );
        assert_eq!(scheduler.next_wait(1_400), Some(PROCESS_EVENT_DEBOUNCE));

        scheduler.record_process_event(
            &process_event("game.exe"),
            &watch_set,
            &BTreeSet::new(),
            2_900,
        );
        assert_eq!(scheduler.next_wait(2_900), Some(Duration::from_millis(100)));
        assert!(scheduler.due(3_000));
    }

    #[test]
    fn scheduler_ignores_started_events_for_already_active_profiles() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        let scheduled = scheduler.record_process_event(
            &process_event("chrome.exe"),
            &watch_set,
            &active_ids(&["chrome"]),
            1_200,
        );

        assert!(!scheduled);
        assert_eq!(scheduler.next_wait(1_200), None);
    }

    #[test]
    fn scheduler_coalesces_stopped_events_for_active_profiles_until_quiet() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        assert!(scheduler.record_process_event(
            &process_stopped_event("chrome.exe"),
            &watch_set,
            &active_ids(&["chrome"]),
            1_000,
        ));
        assert_eq!(scheduler.next_wait(1_000), Some(PROCESS_EVENT_DEBOUNCE));

        scheduler.record_process_event(
            &process_stopped_event("chrome.exe"),
            &watch_set,
            &active_ids(&["chrome"]),
            1_500,
        );
        assert_eq!(scheduler.next_wait(1_500), Some(PROCESS_EVENT_DEBOUNCE));

        scheduler.record_process_event(
            &process_stopped_event("chrome.exe"),
            &watch_set,
            &active_ids(&["chrome"]),
            3_900,
        );
        assert_eq!(scheduler.next_wait(3_900), Some(Duration::from_millis(100)));
        assert!(scheduler.due(4_000));
    }

    #[test]
    fn scheduler_scans_soon_for_inactive_profile_start_events() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        scheduler.record_process_event(
            &process_event("game.exe"),
            &watch_set,
            &active_ids(&["chrome"]),
            1_200,
        );

        assert_eq!(scheduler.next_wait(1_200), Some(PROCESS_EVENT_DEBOUNCE));
    }

    #[test]
    fn scheduler_throttles_broad_folder_wakes() {
        let mut profile = Profile::new("folder-game", "Folder Game", "launcher.exe", "high");
        profile.activation.require_main_process = false;
        profile
            .associated_processes
            .push(powershift_core::ProcessMatcher {
                name: String::new(),
                path: Some("D:\\Games\\Folder".to_string()),
                match_mode: powershift_core::MatchMode::Folder,
            });
        let watch_set = AgentWatchSet::from_config(&AppConfig {
            profiles: vec![profile],
            ..AppConfig::default()
        });
        let mut scheduler = AgentScanScheduler::default();

        assert!(scheduler.record_process_event(
            &process_event("unrelated.exe"),
            &watch_set,
            &BTreeSet::new(),
            1_000,
        ));
        scheduler.mark_scan_completed(1_750);

        assert!(!scheduler.record_process_event(
            &process_event("another.exe"),
            &watch_set,
            &BTreeSet::new(),
            2_000,
        ));
        assert!(scheduler.record_process_event(
            &process_event("later.exe"),
            &watch_set,
            &BTreeSet::new(),
            7_000,
        ));
    }

    #[test]
    fn scheduler_throttles_public_wake_events() {
        let mut scheduler = AgentScanScheduler::default();

        assert!(scheduler.record_public_wake(1_000));
        assert!(scheduler.due(1_000));
        scheduler.mark_scan_completed(1_000);

        assert!(!scheduler.record_public_wake(1_500));
        assert_eq!(scheduler.next_wait(1_500), None);

        assert!(scheduler.record_public_wake(1_000 + duration_ms(PUBLIC_WAKE_EVENT_COOLDOWN)));
    }

    #[test]
    fn scheduler_ignores_stopped_events_for_inactive_profiles() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        let scheduled = scheduler.record_process_event(
            &process_stopped_event("game.exe"),
            &watch_set,
            &active_ids(&["chrome"]),
            1_200,
        );

        assert!(!scheduled);
        assert_eq!(scheduler.next_wait(1_200), None);
    }

    #[test]
    fn scheduler_scans_on_unknown_stop_while_any_profile_is_active() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        let scheduled = scheduler.record_process_event(
            &process_stopped_event("fortnite-helper.exe"),
            &watch_set,
            &active_ids(&["game"]),
            1_200,
        );

        assert!(scheduled);
        assert_eq!(scheduler.next_wait(1_200), Some(PROCESS_EVENT_DEBOUNCE));
    }

    #[test]
    fn scheduler_ignores_unknown_stop_when_no_profile_is_active() {
        let watch_set = AgentWatchSet::from_config(&multi_profile_config());
        let mut scheduler = AgentScanScheduler::default();

        let scheduled = scheduler.record_process_event(
            &process_stopped_event("fortnite-helper.exe"),
            &watch_set,
            &BTreeSet::new(),
            1_200,
        );

        assert!(!scheduled);
        assert_eq!(scheduler.next_wait(1_200), None);
    }

    #[test]
    fn scheduler_forced_wake_is_due_immediately() {
        let mut scheduler = AgentScanScheduler::default();

        scheduler.schedule_forced(2_000);

        assert!(scheduler.due(2_000));
        assert_eq!(scheduler.next_wait(2_000), Some(Duration::ZERO));
    }

    #[test]
    fn schedules_restore_without_polling_delay_until_due() {
        let config = config();
        let active_processes = FakeProcessBackend {
            processes: vec![process("demo.exe")],
        };
        let idle_processes = FakeProcessBackend {
            processes: Vec::new(),
        };
        let power = power("balanced");
        let mut state = AgentRuntimeState::default();

        evaluate_agent_scan_stateful(&config, &active_processes, &power, &mut state, 1_000)
            .expect("activate");
        let result =
            evaluate_agent_scan_stateful(&config, &idle_processes, &power, &mut state, 2_000)
                .expect("schedule");

        assert!(result.restore_scheduled);
        assert_eq!(
            state.pending_restore,
            Some(PendingRestoreState {
                due_at_ms: 32_000,
                plan_id: "balanced".to_string(),
                profile_id: "demo".to_string(),
                profile_name: "Demo Game".to_string(),
            })
        );
        assert_eq!(result.restore_profile_name.as_deref(), Some("Demo Game"));
    }

    #[test]
    fn event_entry_is_created_for_meaningful_changes_only() {
        let active = Ok(AgentScanResult {
            matched_profile_id: Some("demo".to_string()),
            matched_profile_name: Some("Demo".to_string()),
            target_plan_id: Some("high".to_string()),
            restore_profile_name: None,
            active_profiles: vec![AgentActiveProfile {
                profile_id: "demo".to_string(),
                profile_name: "Demo".to_string(),
                plan_id: "high".to_string(),
                priority: 70,
                matched_processes: vec!["demo.exe".to_string()],
            }],
            changed_power_plan: true,
            restore_scheduled: false,
            restored_power_plan: false,
        });
        let idle = Ok(AgentScanResult {
            matched_profile_id: None,
            matched_profile_name: None,
            target_plan_id: None,
            restore_profile_name: None,
            active_profiles: Vec::new(),
            changed_power_plan: false,
            restore_scheduled: false,
            restored_power_plan: false,
        });
        let power = power("high");

        let event = scan_event_entry(&active, &power).expect("activation event");
        assert_eq!(event.kind, "profile_activated");
        assert_eq!(event.message, "Demo activo: high aplicado");
        assert!(scan_event_entry(&idle, &power).is_none());
    }

    #[test]
    fn event_messages_do_not_expose_raw_plan_ids_when_plan_name_is_unknown() {
        let plan_id = "a5fdf429-d150-4353-8425-27bd99357dd8";
        let active = Ok(AgentScanResult {
            matched_profile_id: Some("chrome".to_string()),
            matched_profile_name: Some("Chrome".to_string()),
            target_plan_id: Some(plan_id.to_string()),
            restore_profile_name: None,
            active_profiles: vec![AgentActiveProfile {
                profile_id: "chrome".to_string(),
                profile_name: "Chrome".to_string(),
                plan_id: plan_id.to_string(),
                priority: 70,
                matched_processes: vec!["chrome.exe".to_string()],
            }],
            changed_power_plan: true,
            restore_scheduled: false,
            restored_power_plan: false,
        });
        let power = power("balanced");

        let event = scan_event_entry(&active, &power).expect("activation event");

        assert_eq!(event.message, "Chrome activo: plan aplicado");
        assert!(!event.message.contains(plan_id));
    }

    #[test]
    fn restore_events_include_profile_identity_for_notifications() {
        let restored = Ok(AgentScanResult {
            matched_profile_id: None,
            matched_profile_name: None,
            target_plan_id: Some("balanced".to_string()),
            restore_profile_name: Some("Demo Game".to_string()),
            active_profiles: Vec::new(),
            changed_power_plan: true,
            restore_scheduled: false,
            restored_power_plan: true,
        });
        let power = power("balanced");

        let event = scan_event_entry(&restored, &power).expect("restore event");

        assert_eq!(event.kind, "power_plan_restored");
        assert_eq!(event.profile_name.as_deref(), Some("Demo Game"));
    }

    #[test]
    fn permission_error_message_explains_elevated_agent() {
        assert!(agent_error_message("HRESULT Call failed with: 0x80041003")
            .contains("permisos elevados"));
    }

    #[test]
    fn publish_scan_error_keeps_last_successful_scan() {
        let paths = temp_agent_paths("scan-error-memory");
        let mut memory = AgentPublishMemory::default();
        let scan = active_scan();

        publish_scan_outcome(&paths, Ok(scan.clone()), &mut memory).expect("publish scan");
        publish_scan_outcome(&paths, Err("scan failed".to_string()), &mut memory)
            .expect("publish error");

        let state = read_published_state(&paths);
        assert_eq!(state.status, AgentStatus::Error);
        assert_eq!(state.last_scan, Some(scan));
        assert_eq!(state.last_error.as_deref(), Some("scan failed"));

        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn repeated_identical_scan_does_not_republish_state() {
        let paths = temp_agent_paths("scan-publish-on-change");
        let mut memory = AgentPublishMemory::default();
        let scan = active_scan();

        publish_scan_outcome(&paths, Ok(scan.clone()), &mut memory).expect("first publish");
        let first_state = read_published_state(&paths);
        publish_scan_outcome(&paths, Ok(scan), &mut memory).expect("second publish");
        let second_state = read_published_state(&paths);

        assert_eq!(second_state.updated_at_ms, first_state.updated_at_ms);
        assert_eq!(second_state, first_state);

        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn process_watcher_error_keeps_last_successful_scan() {
        let paths = temp_agent_paths("watcher-error-memory");
        let mut memory = AgentPublishMemory {
            last_scan: Some(active_scan()),
            last_error: None,
        };

        publish_error(&paths, "HRESULT Call failed with: 0x80041003", &mut memory)
            .expect("publish watcher error");

        let state = read_published_state(&paths);
        assert_eq!(state.status, AgentStatus::Error);
        assert_eq!(state.last_scan, Some(active_scan()));
        assert!(state
            .last_error
            .as_deref()
            .expect("last error")
            .contains("permisos elevados"));

        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn heartbeat_updates_live_ipc_state_without_touching_disk() {
        let paths = temp_agent_paths("heartbeat");
        let memory = AgentPublishMemory {
            last_scan: Some(active_scan()),
            last_error: None,
        };
        let shared = AgentSharedState::default();

        publish_heartbeat_with_shared(&paths, &memory, Some(&shared)).expect("publish heartbeat");

        let state = shared.get().expect("shared heartbeat state");
        assert_eq!(state.status, AgentStatus::Running);
        assert_eq!(state.last_scan, Some(active_scan()));
        assert_eq!(state.last_error, None);
        assert!(!paths.state.exists());

        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn publish_state_does_not_leave_temp_file_after_success() {
        let paths = temp_agent_paths("atomic-state");
        let state = PublishedAgentState {
            pid: 10,
            status: AgentStatus::Running,
            updated_at_ms: 1,
            last_scan: Some(active_scan()),
            last_error: None,
        };
        let temp_path = paths
            .state
            .with_extension(format!("tmp-{}", std::process::id()));

        publish_state(&paths.state, state).expect("publish state");

        assert!(paths.state.exists());
        assert!(!temp_path.exists());
        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn publish_state_cleans_stale_temp_files() {
        let paths = temp_agent_paths("atomic-state-cleanup");
        let stale_temp_path = paths.state.with_extension("tmp-12345");
        let unrelated_temp_path = paths.config.with_extension("tmp-12345");
        std::fs::create_dir_all(paths.state.parent().expect("state parent"))
            .expect("create state dir");
        std::fs::write(&stale_temp_path, b"old").expect("seed stale temp");
        std::fs::write(&unrelated_temp_path, b"config temp").expect("seed unrelated temp");

        publish_state(
            &paths.state,
            PublishedAgentState {
                pid: 10,
                status: AgentStatus::Running,
                updated_at_ms: 1,
                last_scan: None,
                last_error: None,
            },
        )
        .expect("publish state");

        assert!(paths.state.exists());
        assert!(!stale_temp_path.exists());
        assert!(unrelated_temp_path.exists());
        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn telemetry_publish_failures_do_not_fail_agent_loop() {
        let paths = temp_agent_paths("telemetry-best-effort");
        let blocked_parent = paths.state.parent().expect("state parent").join("blocked");
        std::fs::create_dir_all(paths.state.parent().expect("state parent"))
            .expect("create state dir");
        std::fs::write(&blocked_parent, b"not a directory").expect("seed blocked path");
        let blocked_paths = AgentPaths {
            config: paths.config.clone(),
            state: blocked_parent.join("agent-state.json"),
            events: blocked_parent.join("events.jsonl"),
        };
        let mut memory = AgentPublishMemory::default();

        publish_scan_outcome(&blocked_paths, Ok(active_scan()), &mut memory)
            .expect("scan publish is best effort");
        publish_error(
            &blocked_paths,
            "HRESULT Call failed with: 0x80041003",
            &mut memory,
        )
        .expect("error publish is best effort");
        publish_heartbeat(&blocked_paths, &memory).expect("heartbeat publish is best effort");

        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn append_event_rotates_large_event_log() {
        let paths = temp_agent_paths("event-rotation");
        std::fs::write(
            &paths.events,
            vec![b'x'; (MAX_EVENT_LOG_BYTES + 1) as usize],
        )
        .expect("seed large log");

        append_event_to_path(
            paths.events.clone(),
            &EventLogEntry::info("profile_activated", "ok"),
        )
        .expect("append event");

        assert!(paths.events.exists());
        assert!(paths.events.with_extension("jsonl.1").exists());
        assert!(
            std::fs::metadata(&paths.events)
                .expect("new log metadata")
                .len()
                < MAX_EVENT_LOG_BYTES
        );
        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn next_wait_uses_heartbeat_without_pending_restore() {
        assert_eq!(
            next_wait_at(&AgentRuntimeState::default(), 1_000),
            AGENT_HEARTBEAT_INTERVAL
        );
    }

    #[test]
    fn next_wait_prefers_due_restore_over_heartbeat() {
        let state = AgentRuntimeState {
            pending_restore: Some(PendingRestoreState {
                due_at_ms: 1_500,
                plan_id: "balanced".to_string(),
                profile_id: "demo".to_string(),
                profile_name: "Demo Game".to_string(),
            }),
            ..AgentRuntimeState::default()
        };

        assert_eq!(next_wait_at(&state, 1_000), Duration::from_millis(500));
        assert!(restore_due_at(&state, 1_500));
    }

    #[test]
    fn next_wait_caps_long_restore_delay_to_heartbeat() {
        let state = AgentRuntimeState {
            pending_restore: Some(PendingRestoreState {
                due_at_ms: 120_000,
                plan_id: "balanced".to_string(),
                profile_id: "demo".to_string(),
                profile_name: "Demo Game".to_string(),
            }),
            ..AgentRuntimeState::default()
        };

        assert_eq!(next_wait_at(&state, 1_000), AGENT_HEARTBEAT_INTERVAL);
        assert!(!restore_due_at(&state, 1_000));
    }

    #[test]
    fn degraded_watcher_caps_wait_to_adaptive_scan_interval() {
        let mut health = AgentWatcherHealth::default();
        health.mark_degraded(powershift_windows::ProcessWatcherKind::Starts);

        assert_eq!(
            next_wait_with_scheduler(
                &AgentRuntimeState::default(),
                &AgentScanScheduler::default(),
                health.is_degraded()
            ),
            DEGRADED_PROCESS_SCAN_INTERVAL
        );
    }

    #[test]
    fn fake_power_error_type_is_available_for_backend_tests() {
        assert_eq!(
            PowerError::Parse("x".to_string()).to_string(),
            "Parse error: x"
        );
    }
}
