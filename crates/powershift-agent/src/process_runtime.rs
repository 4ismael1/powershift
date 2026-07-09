use crate::process_registry::{ProcessRegistry, TrackedProcess};
use crate::scheduler::AgentWatchSet;
use powershift_core::{process_matches_enabled_profile, AppConfig, DetectedProcess, ProcessInfo};
use powershift_windows::{inspect_process, ObservedProcess, ProcessInstanceId};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ProcessEventApplication {
    pub(crate) changed: bool,
    pub(crate) deferred_inspection: Option<(u32, String)>,
}

pub(crate) fn apply_process_event(
    event: &powershift_windows::ProcessEvent,
    config: &AppConfig,
    watch_set: &AgentWatchSet,
    registry: &mut ProcessRegistry,
) -> ProcessEventApplication {
    match event.kind {
        powershift_windows::ProcessEventKind::Started => {
            if !watch_set.should_observe(&event.name) {
                return ProcessEventApplication::default();
            }
            let Some(observed) = inspect_process(event.pid, &event.name) else {
                return ProcessEventApplication {
                    changed: false,
                    deferred_inspection: (event.pid != 0).then(|| (event.pid, event.name.clone())),
                };
            };
            ProcessEventApplication {
                changed: apply_observed_start(config, watch_set, registry, observed),
                deferred_inspection: None,
            }
        }
        powershift_windows::ProcessEventKind::Stopped => {
            let current = inspect_process(event.pid, &event.name);
            ProcessEventApplication {
                changed: apply_observed_stop(config, watch_set, registry, event.pid, current),
                deferred_inspection: None,
            }
        }
    }
}

pub(crate) fn apply_observed_start(
    config: &AppConfig,
    watch_set: &AgentWatchSet,
    registry: &mut ProcessRegistry,
    observed: ObservedProcess,
) -> bool {
    tracked_process_from_observed(config, watch_set, observed)
        .is_some_and(|process| registry.upsert(process))
}

pub(crate) fn apply_observed_stop(
    config: &AppConfig,
    watch_set: &AgentWatchSet,
    registry: &mut ProcessRegistry,
    pid: u32,
    current: Option<ObservedProcess>,
) -> bool {
    let current_instance = current.as_ref().map(|process| &process.instance);
    let mut changed = registry.remove_stopped_pid(pid, current_instance);
    if let Some(process) =
        current.and_then(|process| tracked_process_from_observed(config, watch_set, process))
    {
        changed |= registry.upsert(process);
    }
    changed
}

pub(crate) fn tracked_process_from_snapshot(
    config: &AppConfig,
    watch_set: &AgentWatchSet,
    process: ProcessInfo,
) -> Option<TrackedProcess> {
    if !watch_set.should_observe(&process.name) {
        return None;
    }

    let observed = inspect_process(process.pid, &process.name).unwrap_or(ObservedProcess {
        instance: ProcessInstanceId {
            pid: process.pid,
            creation_time: 0,
        },
        process,
        session_id: None,
    });
    tracked_process_from_observed(config, watch_set, observed)
}

fn tracked_process_from_observed(
    config: &AppConfig,
    watch_set: &AgentWatchSet,
    observed: ObservedProcess,
) -> Option<TrackedProcess> {
    if observed.session_id == Some(0) {
        return None;
    }
    if !watch_set.should_observe(&observed.process.name) {
        return None;
    }
    let process = DetectedProcess {
        pid: observed.process.pid,
        name: observed.process.name,
        path: observed.process.path,
    };
    process_matches_enabled_profile(config, &process)
        .then(|| TrackedProcess::new(observed.instance, process))
}
