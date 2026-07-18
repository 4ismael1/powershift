use crate::engine::evaluate_agent_processes_with_journal;
use crate::power_lease::{PowerLeaseJournal, PowerLeaseStore};
use crate::process_registry::ProcessRegistry;
use crate::process_runtime::tracked_process_from_snapshot;
use crate::publisher::write_scan_event;
use crate::scheduler::AgentWatchSet;
use crate::{now_ms, AgentPaths, AgentRuntimeState, AgentScanResult};
use powershift_core::{AppConfig, ConfigStore};
use powershift_windows::{PowerManagerBackend, ProcessSnapshotBackend, ProcessWatchMessage};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

pub(super) struct AgentReconcileContext<'a> {
    pub(super) state: &'a mut AgentRuntimeState,
    pub(super) config: &'a mut AppConfig,
    pub(super) watch_set: &'a mut AgentWatchSet,
    pub(super) registry: &'a mut ProcessRegistry,
    pub(super) power_lease: &'a mut PowerLeaseStore,
}

pub(super) struct AgentReconcileOutcome {
    pub(super) scan_result: Result<AgentScanResult, String>,
    pub(super) config_warning: Option<String>,
}

pub(super) fn refresh_config_and_reconcile<P, W>(
    paths: &AgentPaths,
    process_backend: &P,
    power_backend: &W,
    context: AgentReconcileContext<'_>,
) -> AgentReconcileOutcome
where
    P: ProcessSnapshotBackend,
    W: PowerManagerBackend,
{
    let config_warning = match load_agent_config(&paths.config) {
        Ok(next_config) => {
            let next_watch_set = AgentWatchSet::from_config(&next_config);
            *context.config = next_config;
            *context.watch_set = next_watch_set;
            None
        }
        Err(error) => Some(format!(
            "No se pudo recargar la configuracion; se conserva la ultima version valida. {error}"
        )),
    };
    let scan_result = reconcile_process_registry(
        context.config,
        context.watch_set,
        process_backend,
        power_backend,
        context.state,
        context.registry,
        context.power_lease,
    );
    write_scan_event(&paths.events, &scan_result, power_backend);
    AgentReconcileOutcome {
        scan_result,
        config_warning,
    }
}

pub(super) fn reconcile_process_registry<P, W, J>(
    config: &AppConfig,
    watch_set: &AgentWatchSet,
    process_backend: &P,
    power_backend: &W,
    state: &mut AgentRuntimeState,
    registry: &mut ProcessRegistry,
    power_lease: &mut J,
) -> Result<AgentScanResult, String>
where
    P: ProcessSnapshotBackend,
    W: PowerManagerBackend,
    J: PowerLeaseJournal,
{
    let tracked_processes = process_backend
        .list_processes_for_tracking()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|process| tracked_process_from_snapshot(config, watch_set, process));
    registry.replace(tracked_processes);
    evaluate_agent_processes_with_journal(
        config,
        &registry.processes(),
        power_backend,
        state,
        now_ms(),
        power_lease,
    )
}

pub(super) fn evaluate_registry_with_paths<W, J>(
    paths: &AgentPaths,
    power_backend: &W,
    state: &mut AgentRuntimeState,
    config: &AppConfig,
    registry: &ProcessRegistry,
    power_lease: &mut J,
) -> Result<AgentScanResult, String>
where
    W: PowerManagerBackend,
    J: PowerLeaseJournal,
{
    let result = evaluate_agent_processes_with_journal(
        config,
        &registry.processes(),
        power_backend,
        state,
        now_ms(),
        power_lease,
    );
    write_scan_event(&paths.events, &result, power_backend);
    result
}

pub(super) fn receive_process_message(
    receiver: &mpsc::Receiver<ProcessWatchMessage>,
    timeout: Option<Duration>,
) -> Result<ProcessWatchMessage, RecvTimeoutError> {
    match timeout {
        Some(timeout) => receiver.recv_timeout(timeout),
        None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
    }
}

pub(super) fn next_restore_wait_at(state: &AgentRuntimeState, now_ms: u64) -> Option<Duration> {
    state
        .pending_restore
        .as_ref()
        .map(|restore| Duration::from_millis(restore.due_at_ms.saturating_sub(now_ms)))
}

pub(super) fn restore_due_at(state: &AgentRuntimeState, now_ms: u64) -> bool {
    state
        .pending_restore
        .as_ref()
        .is_some_and(|restore| restore.due_at_ms <= now_ms)
}

pub(super) fn load_agent_config(path: &std::path::Path) -> Result<AppConfig, String> {
    if path.exists() {
        return ConfigStore::load_with_backup(path).map_err(|error| error.to_string());
    }
    Ok(AppConfig::default())
}

pub(super) fn load_agent_config_for_startup(path: &std::path::Path) -> (AppConfig, Option<String>) {
    match load_agent_config(path) {
        Ok(config) => (config, None),
        Err(error) => (AppConfig::default(), Some(error)),
    }
}
