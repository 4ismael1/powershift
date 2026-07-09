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
    let active_plan = power_backend
        .active_plan()
        .map_err(|error| error.to_string())?;
    let active_profiles = resolve_active_profiles(config, processes);

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
