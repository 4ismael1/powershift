use crate::power_lease::{NoopPowerLeaseJournal, PowerControlLease, PowerLeaseJournal};
use crate::{AgentActiveProfile, AgentRuntimeState, AgentScanResult, PendingRestoreState};
use powershift_core::{
    resolve_active_profiles, ActiveProfile, AppConfig, DetectedProcess, RestoreBehavior,
};
use powershift_windows::{PowerManagerBackend, ProcessSnapshotBackend};
use std::collections::BTreeSet;

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
    evaluate_agent_processes_stateful(config, &processes, power_backend, state, now_ms)
}

pub(crate) fn evaluate_agent_processes_stateful<W>(
    config: &AppConfig,
    processes: &[DetectedProcess],
    power_backend: &W,
    state: &mut AgentRuntimeState,
    now_ms: u64,
) -> Result<AgentScanResult, String>
where
    W: PowerManagerBackend,
{
    let mut journal = NoopPowerLeaseJournal;
    evaluate_agent_processes_with_journal(
        config,
        processes,
        power_backend,
        state,
        now_ms,
        &mut journal,
    )
}

pub(crate) fn evaluate_agent_processes_with_journal<W, J>(
    config: &AppConfig,
    processes: &[DetectedProcess],
    power_backend: &W,
    state: &mut AgentRuntimeState,
    now_ms: u64,
    journal: &mut J,
) -> Result<AgentScanResult, String>
where
    W: PowerManagerBackend,
    J: PowerLeaseJournal,
{
    let active_plan = power_backend
        .active_plan()
        .map_err(|error| error.to_string())?;
    let active_profiles = resolve_active_profiles(config, processes);

    if let Some(winner) =
        choose_winning_profile(&active_profiles, state.winning_profile_id.as_deref())
    {
        if state.active_profile_ids.is_empty()
            && state.pending_restore.is_none()
            && state.previous_plan_id.is_none()
        {
            state.previous_plan_id = Some(active_plan.id.clone());
        }

        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.id == winner.profile_id)
            .ok_or_else(|| "El perfil ganador ya no existe en la configuracion.".to_string())?;
        let previous_plan_id = state
            .previous_plan_id
            .as_deref()
            .unwrap_or(active_plan.id.as_str());
        journal.record(PowerControlLease::for_profile(
            profile,
            previous_plan_id,
            &winner.plan_id,
        ))?;

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
            restore_profile_id: None,
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
            journal.clear()?;
            state.pending_restore = None;
            state.previous_plan_id = None;
            return Ok(AgentScanResult {
                matched_profile_id: None,
                matched_profile_name: None,
                target_plan_id: Some(restore.plan_id),
                restore_profile_id: Some(restore.profile_id),
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
                    restore_profile_id: Some(profile.id.clone()),
                    restore_profile_name: Some(profile.name.clone()),
                    active_profiles: Vec::new(),
                    changed_power_plan: false,
                    restore_scheduled: true,
                    restored_power_plan: false,
                });
            }
            journal.clear()?;
            state.previous_plan_id = None;
        } else if let Some(lease) = journal.current_lease().cloned() {
            if let Some(plan_id) = lease.restore_plan_id {
                state.pending_restore = Some(PendingRestoreState {
                    due_at_ms: now_ms + u64::from(lease.close_delay_seconds) * 1000,
                    plan_id,
                    profile_id: lease.profile_id.clone(),
                    profile_name: lease.profile_name.clone(),
                });
                return Ok(AgentScanResult {
                    matched_profile_id: None,
                    matched_profile_name: None,
                    target_plan_id: None,
                    restore_profile_id: Some(lease.profile_id),
                    restore_profile_name: Some(lease.profile_name),
                    active_profiles: Vec::new(),
                    changed_power_plan: false,
                    restore_scheduled: true,
                    restored_power_plan: false,
                });
            }
            journal.clear()?;
            state.previous_plan_id = None;
        }
    }

    Ok(AgentScanResult {
        matched_profile_id: None,
        matched_profile_name: None,
        target_plan_id: None,
        restore_profile_id: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use powershift_core::{PowerPlan, Profile};
    use powershift_windows::PowerResult;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct RecordingJournal {
        lease: Option<PowerControlLease>,
        recorded: Rc<Cell<bool>>,
        clear_calls: usize,
    }

    impl RecordingJournal {
        fn empty(recorded: Rc<Cell<bool>>) -> Self {
            Self {
                lease: None,
                recorded,
                clear_calls: 0,
            }
        }
    }

    impl PowerLeaseJournal for RecordingJournal {
        fn current_lease(&self) -> Option<&PowerControlLease> {
            self.lease.as_ref()
        }

        fn record(&mut self, lease: PowerControlLease) -> Result<(), String> {
            self.recorded.set(true);
            self.lease = Some(lease);
            Ok(())
        }

        fn clear(&mut self) -> Result<(), String> {
            self.clear_calls += 1;
            self.lease = None;
            Ok(())
        }
    }

    struct OrderedPowerBackend {
        active: RefCell<PowerPlan>,
        journal_recorded: Rc<Cell<bool>>,
        set_calls: RefCell<Vec<String>>,
    }

    impl OrderedPowerBackend {
        fn new(active_plan_id: &str, journal_recorded: Rc<Cell<bool>>) -> Self {
            Self {
                active: RefCell::new(PowerPlan {
                    id: active_plan_id.to_string(),
                    name: active_plan_id.to_string(),
                }),
                journal_recorded,
                set_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl PowerManagerBackend for OrderedPowerBackend {
        fn list_plans(&self) -> PowerResult<Vec<PowerPlan>> {
            Ok(vec![self.active.borrow().clone()])
        }

        fn active_plan(&self) -> PowerResult<PowerPlan> {
            Ok(self.active.borrow().clone())
        }

        fn set_active_plan(&self, plan_id: &str) -> PowerResult<()> {
            assert!(
                self.journal_recorded.get(),
                "power control must be journaled before changing the Windows plan"
            );
            self.set_calls.borrow_mut().push(plan_id.to_string());
            self.active.borrow_mut().id = plan_id.to_string();
            Ok(())
        }
    }

    fn profile() -> Profile {
        Profile::new("game", "Game", "game.exe", "high")
    }

    fn matching_process() -> DetectedProcess {
        DetectedProcess {
            pid: 42,
            name: "game.exe".to_string(),
            path: None,
        }
    }

    #[test]
    fn journals_the_original_plan_before_applying_a_profile() {
        let recorded = Rc::new(Cell::new(false));
        let power = OrderedPowerBackend::new("balanced", recorded.clone());
        let mut journal = RecordingJournal::empty(recorded);
        let mut state = AgentRuntimeState::default();
        let config = AppConfig {
            profiles: vec![profile()],
            ..AppConfig::default()
        };

        let result = evaluate_agent_processes_with_journal(
            &config,
            &[matching_process()],
            &power,
            &mut state,
            10,
            &mut journal,
        )
        .expect("evaluate active profile");

        assert!(result.changed_power_plan);
        assert_eq!(power.set_calls.borrow().as_slice(), &["high"]);
        assert_eq!(
            journal
                .current_lease()
                .map(|lease| lease.previous_plan_id.as_str()),
            Some("balanced")
        );
    }

    #[test]
    fn deleting_the_controlling_profile_still_restores_from_the_journal() {
        let recorded = Rc::new(Cell::new(true));
        let power = OrderedPowerBackend::new("high", recorded.clone());
        let original_profile = profile();
        let mut journal = RecordingJournal {
            lease: Some(PowerControlLease::for_profile(
                &original_profile,
                "balanced",
                "high",
            )),
            recorded,
            clear_calls: 0,
        };
        let mut state = AgentRuntimeState {
            previous_plan_id: Some("balanced".to_string()),
            winning_profile_id: Some("game".to_string()),
            ..AgentRuntimeState::default()
        };

        let scheduled = evaluate_agent_processes_with_journal(
            &AppConfig::default(),
            &[],
            &power,
            &mut state,
            1_000,
            &mut journal,
        )
        .expect("schedule restore after deletion");
        assert!(scheduled.restore_scheduled);

        let due_at = state
            .pending_restore
            .as_ref()
            .expect("pending restore")
            .due_at_ms;
        let restored = evaluate_agent_processes_with_journal(
            &AppConfig::default(),
            &[],
            &power,
            &mut state,
            due_at,
            &mut journal,
        )
        .expect("restore captured plan");

        assert!(restored.restored_power_plan);
        assert_eq!(power.set_calls.borrow().as_slice(), &["balanced"]);
        assert!(journal.current_lease().is_none());
        assert_eq!(journal.clear_calls, 1);
    }
}
