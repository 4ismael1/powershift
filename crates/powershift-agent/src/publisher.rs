use crate::ipc::AgentSharedState;
use crate::{now_ms, AgentPaths, AgentScanResult, PublishedAgentState};
use powershift_core::AgentStatus;
use powershift_windows::PowerManagerBackend;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const MAX_EVENT_LOG_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentPublishMemory {
    pub(crate) last_scan: Option<AgentScanResult>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub kind: String,
    pub message: String,
    pub profile_name: Option<String>,
    pub plan_id: Option<String>,
}

impl EventLogEntry {
    pub fn info(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            timestamp_ms: now_ms(),
            level: "info".to_string(),
            kind: kind.into(),
            message: message.into(),
            profile_name: None,
            plan_id: None,
        }
    }

    pub fn error(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            timestamp_ms: now_ms(),
            level: "error".to_string(),
            kind: kind.into(),
            message: message.into(),
            profile_name: None,
            plan_id: None,
        }
    }
}

#[cfg(test)]
pub(crate) fn publish_scan_outcome(
    paths: &AgentPaths,
    result: Result<AgentScanResult, String>,
    memory: &mut AgentPublishMemory,
) -> Result<(), String> {
    publish_scan_outcome_with_shared(paths, result, memory, None)
}

pub(crate) fn publish_scan_outcome_with_shared(
    paths: &AgentPaths,
    result: Result<AgentScanResult, String>,
    memory: &mut AgentPublishMemory,
    shared_state: Option<&AgentSharedState>,
) -> Result<(), String> {
    match result {
        Ok(scan) => {
            let should_publish =
                memory.last_error.is_some() || (memory.last_scan.as_ref() != Some(&scan));
            memory.last_scan = Some(scan.clone());
            memory.last_error = None;
            if should_publish {
                publish_state_best_effort_with_shared(
                    &paths.state,
                    PublishedAgentState {
                        pid: std::process::id(),
                        status: AgentStatus::Running,
                        updated_at_ms: now_ms(),
                        last_scan: Some(scan),
                        last_error: None,
                    },
                    shared_state,
                );
            }
        }
        Err(error) => {
            let message = agent_error_message(&error);
            let should_publish = memory.last_error.as_deref() != Some(message.as_str());
            memory.last_error = Some(message.clone());
            if should_publish {
                let _ = append_event_to_path(
                    paths.events.clone(),
                    &EventLogEntry::error("agent_error", message.clone()),
                );
                publish_state_best_effort_with_shared(
                    &paths.state,
                    PublishedAgentState {
                        pid: std::process::id(),
                        status: AgentStatus::Error,
                        updated_at_ms: now_ms(),
                        last_scan: memory.last_scan.clone(),
                        last_error: Some(message),
                    },
                    shared_state,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn publish_error(
    paths: &AgentPaths,
    error: &str,
    memory: &mut AgentPublishMemory,
) -> Result<(), String> {
    publish_error_with_shared(paths, error, memory, None)
}

pub(crate) fn publish_error_with_shared(
    paths: &AgentPaths,
    error: &str,
    memory: &mut AgentPublishMemory,
    shared_state: Option<&AgentSharedState>,
) -> Result<(), String> {
    let message = agent_error_message(error);
    if memory.last_error.as_deref() != Some(message.as_str()) {
        let _ = append_event_to_path(
            paths.events.clone(),
            &EventLogEntry::error("agent_error", message.clone()),
        );
        memory.last_error = Some(message.clone());
        publish_state_best_effort_with_shared(
            &paths.state,
            PublishedAgentState {
                pid: std::process::id(),
                status: AgentStatus::Error,
                updated_at_ms: now_ms(),
                last_scan: memory.last_scan.clone(),
                last_error: Some(message),
            },
            shared_state,
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn publish_heartbeat(
    paths: &AgentPaths,
    memory: &AgentPublishMemory,
) -> Result<(), String> {
    publish_heartbeat_with_shared(paths, memory, None)
}

pub(crate) fn publish_heartbeat_with_shared(
    _paths: &AgentPaths,
    memory: &AgentPublishMemory,
    shared_state: Option<&AgentSharedState>,
) -> Result<(), String> {
    if let Some(shared_state) = shared_state {
        shared_state.set(PublishedAgentState {
            pid: std::process::id(),
            status: if memory.last_error.is_some() {
                AgentStatus::Error
            } else {
                AgentStatus::Running
            },
            updated_at_ms: now_ms(),
            last_scan: memory.last_scan.clone(),
            last_error: memory.last_error.clone(),
        });
    }
    Ok(())
}

pub(crate) fn scan_event_entry<W>(
    result: &Result<AgentScanResult, String>,
    power_backend: &W,
) -> Option<EventLogEntry>
where
    W: PowerManagerBackend,
{
    match result {
        Ok(scan) if scan.changed_power_plan && scan.matched_profile_name.is_some() => {
            let profile_name = scan.matched_profile_name.as_deref().unwrap_or("Perfil");
            let plan_name = scan
                .target_plan_id
                .as_deref()
                .and_then(|plan_id| plan_display_name(power_backend, plan_id));
            let mut event = EventLogEntry::info(
                "profile_activated",
                activation_event_message(profile_name, plan_name.as_deref()),
            );
            event.profile_name = Some(profile_name.to_string());
            event.plan_id = scan.target_plan_id.clone();
            Some(event)
        }
        Ok(scan) if scan.restore_scheduled => {
            let mut event = EventLogEntry::info(
                "restore_scheduled",
                match scan.restore_profile_name.as_deref() {
                    Some(profile_name) => {
                        format!("Restore de plan programado tras cerrar {profile_name}")
                    }
                    None => "Restore de plan programado tras cerrar el perfil".to_string(),
                },
            );
            event.profile_name = scan.restore_profile_name.clone();
            Some(event)
        }
        Ok(scan) if scan.restored_power_plan => {
            let plan_name = scan
                .target_plan_id
                .as_deref()
                .and_then(|plan_id| plan_display_name(power_backend, plan_id));
            let mut event = EventLogEntry::info(
                "power_plan_restored",
                restore_event_message(plan_name.as_deref()),
            );
            event.profile_name = scan.restore_profile_name.clone();
            event.plan_id = scan.target_plan_id.clone();
            Some(event)
        }
        Ok(_) => None,
        Err(error) => Some(EventLogEntry::error(
            "agent_error",
            agent_error_message(error),
        )),
    }
}

pub fn append_event_to_path(path: PathBuf, entry: &EventLogEntry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    rotate_event_log_if_needed(&path)?;
    let mut line = serde_json::to_string(entry).map_err(|error| error.to_string())?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
        .map_err(|error| error.to_string())?;
    let _ = powershift_windows::signal_ipc_event(powershift_windows::EVENT_LOG_UPDATED_EVENT_NAME);
    Ok(())
}

pub fn publish_state(path: &PathBuf, state: PublishedAgentState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let value = serde_json::to_vec_pretty(&state).map_err(|error| error.to_string())?;
    powershift_core::write_file_atomically(path, &value).map_err(|error| error.to_string())
}

pub(crate) fn agent_error_message(error: &str) -> String {
    if error.contains("0x80041003") || error.to_ascii_lowercase().contains("access denied") {
        "WMI requiere permisos elevados para eventos de proceso. Instala o inicia el agente elevado."
            .to_string()
    } else {
        error.to_string()
    }
}

pub(crate) fn write_scan_event<W>(
    path: &Path,
    result: &Result<AgentScanResult, String>,
    power_backend: &W,
) where
    W: PowerManagerBackend,
{
    if let Some(entry) = scan_event_entry(result, power_backend) {
        let _ = append_event_to_path(path.to_path_buf(), &entry);
    }
}

fn activation_event_message(profile_name: &str, plan_name: Option<&str>) -> String {
    match plan_name {
        Some(plan_name) => format!("{profile_name} activo: {plan_name} aplicado"),
        None => format!("{profile_name} activo: plan aplicado"),
    }
}

fn restore_event_message(plan_name: Option<&str>) -> String {
    match plan_name {
        Some(plan_name) => format!("Plan restaurado: {plan_name}"),
        None => "Plan restaurado".to_string(),
    }
}

fn plan_display_name<W>(power_backend: &W, plan_id: &str) -> Option<String>
where
    W: PowerManagerBackend,
{
    power_backend
        .list_plans()
        .ok()?
        .into_iter()
        .find(|plan| plan.id.eq_ignore_ascii_case(plan_id))
        .map(|plan| plan.name)
}

fn publish_state_best_effort(path: &PathBuf, state: PublishedAgentState) {
    let _ = publish_state(path, state);
}

fn publish_state_best_effort_with_shared(
    path: &PathBuf,
    state: PublishedAgentState,
    shared_state: Option<&AgentSharedState>,
) {
    if let Some(shared_state) = shared_state {
        shared_state.set(state.clone());
    }
    publish_state_best_effort(path, state);
}

fn rotate_event_log_if_needed(path: &PathBuf) -> Result<(), String> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() <= MAX_EVENT_LOG_BYTES {
        return Ok(());
    }
    let rotated = path.with_extension("jsonl.1");
    let _ = std::fs::remove_file(&rotated);
    std::fs::rename(path, rotated).map_err(|error| error.to_string())
}
