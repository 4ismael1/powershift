use powershift_core::DetectedProcess;
use powershift_windows::ProcessInstanceId;
use std::collections::{BTreeMap, BTreeSet};

/// In-memory source of truth for processes that can affect a profile. The
/// instance key includes process creation time, so a delayed stop signal for a
/// recycled PID cannot remove the newer process.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessRegistry {
    entries: BTreeMap<ProcessInstanceId, DetectedProcess>,
}

impl ProcessRegistry {
    pub(crate) fn replace(&mut self, processes: impl IntoIterator<Item = TrackedProcess>) -> bool {
        let next = processes
            .into_iter()
            .map(|tracked| (tracked.instance, tracked.process))
            .collect::<BTreeMap<_, _>>();
        if self.entries == next {
            return false;
        }
        self.entries = next;
        true
    }

    pub(crate) fn upsert(&mut self, tracked: TrackedProcess) -> bool {
        let stale_instances = self
            .entries
            .keys()
            .filter(|instance| {
                instance.pid == tracked.instance.pid && **instance != tracked.instance
            })
            .cloned()
            .collect::<Vec<_>>();
        for instance in stale_instances {
            self.entries.remove(&instance);
        }

        if self.entries.get(&tracked.instance) == Some(&tracked.process) {
            return false;
        }
        self.entries.insert(tracked.instance, tracked.process);
        true
    }

    pub(crate) fn remove_exact(&mut self, instance: &ProcessInstanceId) -> bool {
        self.entries.remove(instance).is_some()
    }

    /// WMI stop events do not include creation time. If a PID is still alive,
    /// retain the current observed instance and only remove stale prior ones.
    pub(crate) fn remove_stopped_pid(
        &mut self,
        pid: u32,
        current_instance: Option<&ProcessInstanceId>,
    ) -> bool {
        let before = self.entries.len();
        self.entries.retain(|instance, _| {
            instance.pid != pid || current_instance.is_some_and(|current| current == instance)
        });
        self.entries.len() != before
    }

    pub(crate) fn contains(&self, instance: &ProcessInstanceId) -> bool {
        self.entries.contains_key(instance)
    }

    pub(crate) fn instances(&self) -> impl Iterator<Item = &ProcessInstanceId> {
        self.entries.keys()
    }

    pub(crate) fn processes(&self) -> Vec<DetectedProcess> {
        self.entries.values().cloned().collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedProcess {
    pub(crate) instance: ProcessInstanceId,
    pub(crate) process: DetectedProcess,
}

impl TrackedProcess {
    pub(crate) fn new(instance: ProcessInstanceId, process: DetectedProcess) -> Self {
        Self { instance, process }
    }
}

/// Keeps the RAII wait registrations separate from the registry's pure state.
/// A process that cannot be opened remains tracked and falls back to WMI or
/// degraded reconciliation; it is not retried on every event.
#[derive(Debug, Default)]
pub(crate) struct ProcessExitWatchSet {
    watches: BTreeMap<ProcessInstanceId, powershift_windows::ProcessExitWatch>,
    unavailable: BTreeSet<ProcessInstanceId>,
}

impl ProcessExitWatchSet {
    pub(crate) fn synchronize(
        &mut self,
        registry: &ProcessRegistry,
        sender: &std::sync::mpsc::Sender<powershift_windows::ProcessWatchMessage>,
    ) {
        self.watches
            .retain(|instance, _| registry.contains(instance));
        self.unavailable
            .retain(|instance| registry.contains(instance));

        for instance in registry.instances() {
            if self.watches.contains_key(instance) || self.unavailable.contains(instance) {
                continue;
            }
            match powershift_windows::register_process_exit_wait(instance.clone(), sender.clone()) {
                Some(watch) => {
                    self.watches.insert(instance.clone(), watch);
                }
                None => {
                    self.unavailable.insert(instance.clone());
                }
            }
        }
    }

    pub(crate) fn clear_unavailable(&mut self) {
        self.unavailable.clear();
    }

    pub(crate) fn registered_count(&self) -> usize {
        self.watches.len()
    }

    pub(crate) fn unavailable_count(&self) -> usize {
        self.unavailable.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(pid: u32, creation_time: u64) -> ProcessInstanceId {
        ProcessInstanceId { pid, creation_time }
    }

    fn tracked(pid: u32, creation_time: u64, name: &str) -> TrackedProcess {
        TrackedProcess::new(
            instance(pid, creation_time),
            DetectedProcess::new(pid, name, None::<String>),
        )
    }

    #[test]
    fn removes_only_the_exact_exited_process_instance() {
        let old = instance(42, 100);
        let current = instance(42, 200);
        let mut registry = ProcessRegistry::default();
        registry.upsert(tracked(42, 200, "fortnite.exe"));

        assert!(!registry.remove_exact(&old));
        assert!(registry.contains(&current));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn replacing_a_reused_pid_discards_the_old_instance() {
        let old = instance(42, 100);
        let current = instance(42, 200);
        let mut registry = ProcessRegistry::default();
        registry.upsert(tracked(42, 100, "game.exe"));

        assert!(registry.upsert(tracked(42, 200, "game.exe")));
        assert!(!registry.contains(&old));
        assert!(registry.contains(&current));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn wmi_stop_keeps_a_reused_pid_that_is_still_observed() {
        let current = instance(42, 200);
        let mut registry = ProcessRegistry::default();
        registry.upsert(tracked(42, 200, "game.exe"));

        assert!(!registry.remove_stopped_pid(42, Some(&current)));
        assert!(registry.contains(&current));
    }

    #[test]
    fn wmi_stop_removes_tracked_process_when_pid_is_gone() {
        let mut registry = ProcessRegistry::default();
        registry.upsert(tracked(42, 200, "game.exe"));

        assert!(registry.remove_stopped_pid(42, None));
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn replace_is_stable_for_an_identical_snapshot() {
        let mut registry = ProcessRegistry::default();
        let snapshot = vec![tracked(1, 10, "game.exe")];

        assert!(registry.replace(snapshot.clone()));
        assert!(!registry.replace(snapshot));
    }
}
