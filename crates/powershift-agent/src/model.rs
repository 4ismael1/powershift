use powershift_core::AgentStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentScanResult {
    pub matched_profile_id: Option<String>,
    pub matched_profile_name: Option<String>,
    pub target_plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_profile_id: Option<String>,
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
    #[serde(default)]
    pub process_tracking: ProcessTrackingStatus,
    #[serde(default)]
    pub wmi_watchers: WmiWatcherStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProcessTrackingStatus {
    pub tracked_instances: u32,
    pub registered_exit_waits: u32,
    pub unavailable_exit_waits: u32,
    pub pending_targeted_inspections: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WmiWatcherState {
    #[default]
    Starting,
    Running,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WmiWatcherChannelStatus {
    pub state: WmiWatcherState,
    pub last_transition_ms: u64,
    pub retry_in_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WmiWatcherStatus {
    pub starts: WmiWatcherChannelStatus,
    pub stops: WmiWatcherChannelStatus,
}

impl WmiWatcherStatus {
    pub(crate) fn mark_healthy(
        &mut self,
        kind: powershift_windows::ProcessWatcherKind,
        now_ms: u64,
    ) {
        let channel = self.channel_mut(kind);
        channel.state = WmiWatcherState::Running;
        channel.last_transition_ms = now_ms;
        channel.retry_in_ms = None;
        channel.last_error = None;
    }

    pub(crate) fn mark_degraded(
        &mut self,
        kind: powershift_windows::ProcessWatcherKind,
        error: String,
        retry_in_ms: u64,
        now_ms: u64,
    ) {
        let channel = self.channel_mut(kind);
        channel.state = WmiWatcherState::Degraded;
        channel.last_transition_ms = now_ms;
        channel.retry_in_ms = Some(retry_in_ms);
        channel.last_error = Some(error);
    }

    pub(crate) fn is_degraded(&self) -> bool {
        self.starts.state == WmiWatcherState::Degraded
            || self.stops.state == WmiWatcherState::Degraded
    }

    fn channel_mut(
        &mut self,
        kind: powershift_windows::ProcessWatcherKind,
    ) -> &mut WmiWatcherChannelStatus {
        match kind {
            powershift_windows::ProcessWatcherKind::Starts => &mut self.starts,
            powershift_windows::ProcessWatcherKind::Stops => &mut self.stops,
        }
    }
}
