use powershift_core::{AgentStatus, AppConfig, ConfigStore};
mod engine;
mod ipc;
mod model;
mod paths;
mod process_registry;
mod process_runtime;
mod publisher;
mod scheduler;

pub use engine::evaluate_agent_scan_stateful;
pub use ipc::{
    request_agent_clear_events_via_ipc, request_agent_reevaluate_via_ipc,
    request_agent_shutdown_via_ipc, request_agent_status_via_ipc, AgentIpcRequest,
    AgentIpcResponse,
};
pub use model::{
    AgentActiveProfile, AgentRuntimeState, AgentScanResult, PendingRestoreState,
    ProcessTrackingStatus, PublishedAgentState, WmiWatcherChannelStatus, WmiWatcherState,
    WmiWatcherStatus,
};
pub use paths::AgentPaths;
pub use publisher::{append_event_to_path, publish_state, EventLogEntry};

use engine::evaluate_agent_processes_stateful;
#[cfg(test)]
use ipc::handle_agent_ipc_request;
use ipc::{load_or_create_control_token, spawn_agent_ipc_server, AgentSharedState};
use powershift_windows::{
    create_agent_wake_event, inspect_process, spawn_process_event_watchers, wait_for_agent_wake,
    PowerManager, PowerManagerBackend, ProcessSnapshotBackend, ProcessWatchMessage,
    SystemProcessBackend,
};
use process_registry::{ProcessExitWatchSet, ProcessRegistry};
#[cfg(test)]
use process_runtime::apply_observed_stop;
use process_runtime::{apply_observed_start, apply_process_event, tracked_process_from_snapshot};
#[cfg(test)]
use publisher::{
    agent_error_message, publish_error, publish_heartbeat, publish_scan_outcome, scan_event_entry,
    MAX_EVENT_LOG_BYTES,
};
use publisher::{
    publish_error_with_shared, publish_heartbeat_with_shared, publish_scan_outcome_with_shared,
    write_scan_event, AgentPublishMemory,
};
#[cfg(test)]
use scheduler::DEGRADED_PROCESS_SCAN_INTERVAL;
use scheduler::{
    agent_wake_event, is_agent_wake_event, next_wait_with_scheduler, AgentScanScheduler,
    AgentWatchSet, TargetedInspectionQueue,
};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

const AGENT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

fn synchronize_watcher_health(
    watcher_health: &WmiWatcherStatus,
    shared_state: &AgentSharedState,
    publish_memory: &mut AgentPublishMemory,
) {
    publish_memory.wmi_watchers = watcher_health.clone();
    if let Some(mut state) = shared_state.get() {
        if state.wmi_watchers != *watcher_health {
            state.wmi_watchers = watcher_health.clone();
            shared_state.set(state);
            powershift_windows::signal_agent_state_updated();
        }
    }
}

fn synchronize_exit_watches(
    exit_watches: &mut ProcessExitWatchSet,
    registry: &ProcessRegistry,
    pending_inspections: &TargetedInspectionQueue,
    sender: &mpsc::Sender<ProcessWatchMessage>,
    shared_state: &AgentSharedState,
    publish_memory: &mut AgentPublishMemory,
) {
    exit_watches.synchronize(registry, sender);
    let tracking = ProcessTrackingStatus {
        tracked_instances: registry.instances().count().try_into().unwrap_or(u32::MAX),
        registered_exit_waits: exit_watches
            .registered_count()
            .try_into()
            .unwrap_or(u32::MAX),
        unavailable_exit_waits: exit_watches
            .unavailable_count()
            .try_into()
            .unwrap_or(u32::MAX),
        pending_targeted_inspections: pending_inspections.len().try_into().unwrap_or(u32::MAX),
    };
    publish_memory.process_tracking = tracking.clone();
    if let Some(mut state) = shared_state.get() {
        if state.process_tracking != tracking {
            state.process_tracking = tracking;
            shared_state.set(state);
        }
    }
}

pub fn run_agent_forever() -> Result<(), String> {
    let paths = AgentPaths::from_environment().map_err(|error| error.to_string())?;
    paths
        .prepare_runtime_directory()
        .map_err(|error| error.to_string())?;
    let mut runtime = AgentRuntimeState::default();
    let mut publish_memory = AgentPublishMemory::default();
    let mut scheduler = AgentScanScheduler::default();
    let mut watcher_health = WmiWatcherStatus::default();
    let mut process_registry = ProcessRegistry::default();
    let mut exit_watches = ProcessExitWatchSet::default();
    let mut pending_inspections = TargetedInspectionQueue::default();
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
        process_tracking: ProcessTrackingStatus::default(),
        wmi_watchers: WmiWatcherStatus::default(),
    };
    shared_state.set(starting_state.clone());
    publish_state(&paths.state, starting_state)?;

    let (sender, receiver) = mpsc::channel();
    spawn_agent_ipc_server(
        sender.clone(),
        shared_state.clone(),
        control_token,
        paths.events.clone(),
    );
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

    let mut config = AppConfig::default();
    let mut watch_set = AgentWatchSet::default();
    let scan_result = refresh_config_and_reconcile(
        &paths,
        &process_backend,
        &power_backend,
        &mut runtime,
        &mut config,
        &mut watch_set,
        &mut process_registry,
    );
    synchronize_exit_watches(
        &mut exit_watches,
        &process_registry,
        &pending_inspections,
        &sender,
        &shared_state,
        &mut publish_memory,
    );
    publish_scan_outcome_with_shared(
        &paths,
        scan_result,
        &mut publish_memory,
        Some(&shared_state),
    )?;

    loop {
        let receive_timeout =
            next_wait_with_scheduler(&runtime, &scheduler, watcher_health.is_degraded());
        let receive_timeout = pending_inspections
            .next_wait(now_ms())
            .map(|pending_wait| pending_wait.min(receive_timeout))
            .unwrap_or(receive_timeout);
        match receiver.recv_timeout(receive_timeout) {
            Ok(ProcessWatchMessage::Event(event)) => {
                if is_agent_wake_event(&event) {
                    scheduler.record_public_wake(now_ms());
                } else {
                    let was_stop = event.kind == powershift_windows::ProcessEventKind::Stopped;
                    if event.kind == powershift_windows::ProcessEventKind::Stopped {
                        pending_inspections.remove(event.pid);
                    }
                    let application =
                        apply_process_event(&event, &config, &watch_set, &mut process_registry);
                    if let Some((pid, name)) = application.deferred_inspection {
                        pending_inspections.schedule_initial(pid, name, now_ms());
                    }
                    if !application.changed {
                        if was_stop || pending_inspections.len() > 0 {
                            synchronize_exit_watches(
                                &mut exit_watches,
                                &process_registry,
                                &pending_inspections,
                                &sender,
                                &shared_state,
                                &mut publish_memory,
                            );
                        }
                        continue;
                    }
                    synchronize_exit_watches(
                        &mut exit_watches,
                        &process_registry,
                        &pending_inspections,
                        &sender,
                        &shared_state,
                        &mut publish_memory,
                    );
                    let scan_result = evaluate_registry_with_paths(
                        &paths,
                        &power_backend,
                        &mut runtime,
                        &config,
                        &process_registry,
                    );
                    publish_scan_outcome_with_shared(
                        &paths,
                        scan_result,
                        &mut publish_memory,
                        Some(&shared_state),
                    )?;
                }
            }
            Ok(ProcessWatchMessage::TrackedProcessExited(instance)) => {
                if process_registry.remove_exact(&instance) {
                    synchronize_exit_watches(
                        &mut exit_watches,
                        &process_registry,
                        &pending_inspections,
                        &sender,
                        &shared_state,
                        &mut publish_memory,
                    );
                    let scan_result = evaluate_registry_with_paths(
                        &paths,
                        &power_backend,
                        &mut runtime,
                        &config,
                        &process_registry,
                    );
                    publish_scan_outcome_with_shared(
                        &paths,
                        scan_result,
                        &mut publish_memory,
                        Some(&shared_state),
                    )?;
                }
            }
            Ok(ProcessWatchMessage::Reevaluate) => {
                scheduler.schedule_forced(now_ms());
            }
            Ok(ProcessWatchMessage::Error(error)) => {
                publish_error_with_shared(&paths, &error, &mut publish_memory, Some(&shared_state))?
            }
            Ok(ProcessWatchMessage::WatcherHealthy(kind)) => {
                watcher_health.mark_healthy(kind, now_ms());
                synchronize_watcher_health(&watcher_health, &shared_state, &mut publish_memory);
                scheduler.schedule_forced(now_ms());
            }
            Ok(ProcessWatchMessage::WatcherDegraded {
                kind,
                error,
                retry_in_ms,
            }) => {
                watcher_health.mark_degraded(kind, error.clone(), retry_in_ms, now_ms());
                synchronize_watcher_health(&watcher_health, &shared_state, &mut publish_memory);
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
                let now = now_ms();
                let due_inspections = pending_inspections.take_due(now);
                if !due_inspections.is_empty() {
                    let mut changed = false;
                    for inspection in due_inspections {
                        match inspect_process(inspection.pid, &inspection.name) {
                            Some(observed) => {
                                changed |= apply_observed_start(
                                    &config,
                                    &watch_set,
                                    &mut process_registry,
                                    observed,
                                );
                            }
                            None => pending_inspections.reschedule_after_failure(inspection, now),
                        }
                    }
                    synchronize_exit_watches(
                        &mut exit_watches,
                        &process_registry,
                        &pending_inspections,
                        &sender,
                        &shared_state,
                        &mut publish_memory,
                    );
                    if changed {
                        let scan_result = evaluate_registry_with_paths(
                            &paths,
                            &power_backend,
                            &mut runtime,
                            &config,
                            &process_registry,
                        );
                        publish_scan_outcome_with_shared(
                            &paths,
                            scan_result,
                            &mut publish_memory,
                            Some(&shared_state),
                        )?;
                    }
                }
                if scheduler.due(now) {
                    let scan_result = refresh_config_and_reconcile(
                        &paths,
                        &process_backend,
                        &power_backend,
                        &mut runtime,
                        &mut config,
                        &mut watch_set,
                        &mut process_registry,
                    );
                    exit_watches.clear_unavailable();
                    synchronize_exit_watches(
                        &mut exit_watches,
                        &process_registry,
                        &pending_inspections,
                        &sender,
                        &shared_state,
                        &mut publish_memory,
                    );
                    publish_scan_outcome_with_shared(
                        &paths,
                        scan_result,
                        &mut publish_memory,
                        Some(&shared_state),
                    )?;
                    scheduler.mark_scan_completed(now_ms());
                } else if restore_due(&runtime) || watcher_health.is_degraded() {
                    let scan_result = refresh_config_and_reconcile(
                        &paths,
                        &process_backend,
                        &power_backend,
                        &mut runtime,
                        &mut config,
                        &mut watch_set,
                        &mut process_registry,
                    );
                    exit_watches.clear_unavailable();
                    synchronize_exit_watches(
                        &mut exit_watches,
                        &process_registry,
                        &pending_inspections,
                        &sender,
                        &shared_state,
                        &mut publish_memory,
                    );
                    publish_scan_outcome_with_shared(
                        &paths,
                        scan_result,
                        &mut publish_memory,
                        Some(&shared_state),
                    )?;
                } else {
                    publish_heartbeat_with_shared(&paths, &publish_memory, Some(&shared_state))?;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

pub fn run_scan_once() -> Result<AgentScanResult, String> {
    let paths = AgentPaths::from_environment().map_err(|error| error.to_string())?;
    paths
        .prepare_runtime_directory()
        .map_err(|error| error.to_string())?;
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
    let config = load_agent_config(&paths.config)?;
    let result =
        evaluate_agent_scan_stateful(&config, process_backend, power_backend, state, now_ms());
    write_scan_event(&paths.events, &result, power_backend);
    result
}

#[cfg(test)]
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
    let config = match load_agent_config(&paths.config) {
        Ok(config) => config,
        Err(error) => return (Err(error), AgentWatchSet::default()),
    };
    let watch_set = AgentWatchSet::from_config(&config);
    let result =
        evaluate_agent_scan_stateful(&config, process_backend, power_backend, state, now_ms());
    write_scan_event(&paths.events, &result, power_backend);
    (result, watch_set)
}

fn refresh_config_and_reconcile<P, W>(
    paths: &AgentPaths,
    process_backend: &P,
    power_backend: &W,
    state: &mut AgentRuntimeState,
    config: &mut AppConfig,
    watch_set: &mut AgentWatchSet,
    registry: &mut ProcessRegistry,
) -> Result<AgentScanResult, String>
where
    P: ProcessSnapshotBackend,
    W: PowerManagerBackend,
{
    let next_config = load_agent_config(&paths.config)?;
    let next_watch_set = AgentWatchSet::from_config(&next_config);
    *config = next_config;
    *watch_set = next_watch_set;
    let result = reconcile_process_registry(
        config,
        watch_set,
        process_backend,
        power_backend,
        state,
        registry,
    );
    write_scan_event(&paths.events, &result, power_backend);
    result
}

fn reconcile_process_registry<P, W>(
    config: &AppConfig,
    watch_set: &AgentWatchSet,
    process_backend: &P,
    power_backend: &W,
    state: &mut AgentRuntimeState,
    registry: &mut ProcessRegistry,
) -> Result<AgentScanResult, String>
where
    P: ProcessSnapshotBackend,
    W: PowerManagerBackend,
{
    let tracked_processes = process_backend
        .list_processes()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|process| tracked_process_from_snapshot(config, watch_set, process));
    registry.replace(tracked_processes);
    evaluate_agent_processes_stateful(
        config,
        &registry.processes(),
        power_backend,
        state,
        now_ms(),
    )
}

fn evaluate_registry_with_paths<W>(
    paths: &AgentPaths,
    power_backend: &W,
    state: &mut AgentRuntimeState,
    config: &AppConfig,
    registry: &ProcessRegistry,
) -> Result<AgentScanResult, String>
where
    W: PowerManagerBackend,
{
    let result = evaluate_agent_processes_stateful(
        config,
        &registry.processes(),
        power_backend,
        state,
        now_ms(),
    );
    write_scan_event(&paths.events, &result, power_backend);
    result
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

fn load_agent_config(path: &std::path::Path) -> Result<AppConfig, String> {
    if path.exists() {
        return ConfigStore::load_with_backup(path).map_err(|error| error.to_string());
    }
    Ok(AppConfig::default())
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
    use powershift_core::{ActiveProfile, DetectedProcess, PowerPlan, ProcessInfo, Profile};
    use powershift_windows::{ObservedProcess, PowerError, PowerResult, ProcessInstanceId};
    use std::cell::RefCell;
    use std::collections::BTreeSet;

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

    fn observed(pid: u32, creation_time: u64, name: &str) -> ObservedProcess {
        ObservedProcess {
            instance: ProcessInstanceId { pid, creation_time },
            process: ProcessInfo {
                pid,
                name: name.to_string(),
                path: None,
            },
            session_id: Some(1),
        }
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
    fn missing_config_uses_defaults_without_an_elevated_write() {
        let paths = temp_agent_paths("read-only-default-config");

        let config = load_agent_config(&paths.config).expect("default config");

        assert_eq!(config, AppConfig::default());
        assert!(!paths.config.exists());
        let _ = std::fs::remove_dir_all(paths.runtime_dir());
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
    fn tracked_exit_updates_only_the_closed_profile_without_a_snapshot_scan() {
        let config = multi_profile_config();
        let watch_set = AgentWatchSet::from_config(&config);
        let chrome = observed(10, 100, "chrome.exe");
        let game = observed(20, 200, "game.exe");
        let mut registry = ProcessRegistry::default();

        assert!(apply_observed_start(
            &config,
            &watch_set,
            &mut registry,
            chrome.clone(),
        ));
        assert!(apply_observed_start(
            &config,
            &watch_set,
            &mut registry,
            game.clone(),
        ));

        let mut state = AgentRuntimeState::default();
        let first = evaluate_agent_processes_stateful(
            &config,
            &registry.processes(),
            &power("balanced"),
            &mut state,
            1,
        )
        .expect("evaluate both active profiles");
        assert_eq!(first.matched_profile_id.as_deref(), Some("game"));

        assert!(registry.remove_exact(&game.instance));
        let power = power("high");
        let after_exit = evaluate_agent_processes_stateful(
            &config,
            &registry.processes(),
            &power,
            &mut state,
            2,
        )
        .expect("evaluate exact tracked exit");

        assert_eq!(after_exit.matched_profile_id.as_deref(), Some("chrome"));
        assert_eq!(
            power.set_calls.borrow().as_slice(),
            &["balanced".to_string()]
        );
    }

    #[test]
    fn stale_wmi_stop_cannot_remove_a_reused_pid_instance() {
        let config = config();
        let watch_set = AgentWatchSet::from_config(&config);
        let old = observed(42, 100, "demo.exe");
        let current = observed(42, 200, "demo.exe");
        let mut registry = ProcessRegistry::default();

        assert!(apply_observed_start(
            &config,
            &watch_set,
            &mut registry,
            old
        ));
        assert!(apply_observed_stop(
            &config,
            &watch_set,
            &mut registry,
            42,
            Some(current.clone()),
        ));

        assert!(registry.contains(&current.instance));
        assert_eq!(registry.processes()[0].pid, 42);
    }

    #[test]
    fn stop_without_a_live_pid_removes_the_tracked_process_immediately() {
        let config = config();
        let watch_set = AgentWatchSet::from_config(&config);
        let tracked = observed(42, 100, "demo.exe");
        let mut registry = ProcessRegistry::default();
        apply_observed_start(&config, &watch_set, &mut registry, tracked);

        assert!(apply_observed_stop(
            &config,
            &watch_set,
            &mut registry,
            42,
            None,
        ));
        assert!(registry.processes().is_empty());
    }

    #[test]
    fn service_session_processes_cannot_activate_user_profiles() {
        let config = config();
        let watch_set = AgentWatchSet::from_config(&config);
        let mut registry = ProcessRegistry::default();
        let service_process = ObservedProcess {
            instance: ProcessInstanceId {
                pid: 77,
                creation_time: 100,
            },
            process: ProcessInfo {
                pid: 77,
                name: "demo.exe".to_string(),
                path: None,
            },
            session_id: Some(0),
        };

        assert!(!apply_observed_start(
            &config,
            &watch_set,
            &mut registry,
            service_process,
        ));
        assert!(registry.processes().is_empty());
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

    fn handle_test_ipc_request(
        request: &str,
        shared: &AgentSharedState,
        sender: &mpsc::Sender<ProcessWatchMessage>,
    ) -> String {
        let event_log = std::env::temp_dir().join(format!(
            "powershift-ipc-events-{}.jsonl",
            std::process::id()
        ));
        handle_agent_ipc_request(request, shared, sender, IPC_TEST_TOKEN, &event_log)
    }

    #[test]
    fn ipc_status_returns_live_memory_state() {
        let state = PublishedAgentState {
            pid: 44,
            status: AgentStatus::Running,
            updated_at_ms: 123,
            last_scan: Some(active_scan()),
            last_error: None,
            process_tracking: ProcessTrackingStatus::default(),
            wmi_watchers: WmiWatcherStatus::default(),
        };
        let shared = shared_state_with(state.clone());
        let (sender, _receiver) = mpsc::channel();
        let request = serde_json::to_string(&AgentIpcRequest::GetStatus).expect("request json");

        let response: AgentIpcResponse =
            serde_json::from_str(&handle_test_ipc_request(&request, &shared, &sender))
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
            process_tracking: ProcessTrackingStatus::default(),
            wmi_watchers: WmiWatcherStatus::default(),
        });
        let (sender, receiver) = mpsc::channel();
        let request = serde_json::to_string(&AgentIpcRequest::Reevaluate {
            token: Some(IPC_TEST_TOKEN.to_string()),
        })
        .expect("request json");

        let response: AgentIpcResponse =
            serde_json::from_str(&handle_test_ipc_request(&request, &shared, &sender))
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

        let response: AgentIpcResponse =
            serde_json::from_str(&handle_test_ipc_request(&request, &shared, &sender))
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

        let response: AgentIpcResponse =
            serde_json::from_str(&handle_test_ipc_request(&request, &shared, &sender))
                .expect("response json");

        assert!(!response.ok);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn ipc_clear_events_removes_current_and_rotated_logs() {
        let path = std::env::temp_dir().join(format!(
            "powershift-ipc-clear-events-{}.jsonl",
            std::process::id()
        ));
        let rotated = path.with_extension("jsonl.1");
        std::fs::write(&path, "current").expect("seed current log");
        std::fs::write(&rotated, "rotated").expect("seed rotated log");
        let shared = AgentSharedState::default();
        let (sender, _receiver) = mpsc::channel();
        let request = serde_json::to_string(&AgentIpcRequest::ClearEvents {
            token: Some(IPC_TEST_TOKEN.to_string()),
        })
        .expect("request json");

        let response: AgentIpcResponse = serde_json::from_str(&handle_agent_ipc_request(
            &request,
            &shared,
            &sender,
            IPC_TEST_TOKEN,
            &path,
        ))
        .expect("response json");

        assert!(response.ok);
        assert!(!path.exists());
        assert!(!rotated.exists());
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
    fn watch_set_observes_all_names_for_folder_matchers() {
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
        assert!(watch_set.should_observe("unexpected-helper.exe"));
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
    fn scheduler_throttles_public_wake_events() {
        let mut scheduler = AgentScanScheduler::default();

        assert!(scheduler.record_public_wake(1_000));
        assert!(scheduler.due(1_000));
        scheduler.mark_scan_completed(1_000);

        assert!(!scheduler.record_public_wake(1_500));
        assert_eq!(scheduler.next_wait(1_500), None);

        assert!(scheduler.record_public_wake(3_000));
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
    fn scan_publish_preserves_process_tracking_and_watcher_health() {
        let paths = temp_agent_paths("scan-publish-tracking");
        let tracking = ProcessTrackingStatus {
            tracked_instances: 3,
            registered_exit_waits: 2,
            unavailable_exit_waits: 1,
            pending_targeted_inspections: 0,
        };
        let mut watchers = WmiWatcherStatus::default();
        watchers.mark_healthy(powershift_windows::ProcessWatcherKind::Starts, 50);
        watchers.mark_degraded(
            powershift_windows::ProcessWatcherKind::Stops,
            "WMI unavailable".to_string(),
            1_000,
            60,
        );
        let mut memory = AgentPublishMemory {
            process_tracking: tracking.clone(),
            wmi_watchers: watchers.clone(),
            ..AgentPublishMemory::default()
        };

        publish_scan_outcome(&paths, Ok(active_scan()), &mut memory).expect("publish scan");

        let state = read_published_state(&paths);
        assert_eq!(state.process_tracking, tracking);
        assert_eq!(state.wmi_watchers, watchers);
        let _ = std::fs::remove_dir_all(paths.state.parent().expect("state parent"));
    }

    #[test]
    fn process_watcher_error_keeps_last_successful_scan() {
        let paths = temp_agent_paths("watcher-error-memory");
        let mut memory = AgentPublishMemory {
            last_scan: Some(active_scan()),
            last_error: None,
            process_tracking: ProcessTrackingStatus::default(),
            wmi_watchers: WmiWatcherStatus::default(),
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
            process_tracking: ProcessTrackingStatus {
                tracked_instances: 3,
                registered_exit_waits: 3,
                unavailable_exit_waits: 0,
                pending_targeted_inspections: 0,
            },
            wmi_watchers: WmiWatcherStatus::default(),
        };
        let shared = AgentSharedState::default();

        publish_heartbeat_with_shared(&paths, &memory, Some(&shared)).expect("publish heartbeat");

        let state = shared.get().expect("shared heartbeat state");
        assert_eq!(state.status, AgentStatus::Running);
        assert_eq!(state.last_scan, Some(active_scan()));
        assert_eq!(state.last_error, None);
        assert_eq!(state.process_tracking, memory.process_tracking);
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
            process_tracking: ProcessTrackingStatus::default(),
            wmi_watchers: WmiWatcherStatus::default(),
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
                process_tracking: ProcessTrackingStatus::default(),
                wmi_watchers: WmiWatcherStatus::default(),
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
        let mut health = WmiWatcherStatus::default();
        health.mark_degraded(
            powershift_windows::ProcessWatcherKind::Starts,
            "test".to_string(),
            1_000,
            10,
        );

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
