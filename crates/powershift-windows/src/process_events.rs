use crate::ProcessInstanceId;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::Duration;

pub const PROCESS_START_TRACE_QUERY: &str =
    "SELECT ProcessID, ProcessName FROM Win32_ProcessStartTrace";
pub const PROCESS_STOP_TRACE_QUERY: &str =
    "SELECT ProcessID, ProcessName FROM Win32_ProcessStopTrace";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEventKind {
    Started,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEvent {
    pub kind: ProcessEventKind,
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProcessWatcherKind {
    Starts,
    Stops,
}

impl ProcessWatcherKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Starts => "inicio",
            Self::Stops => "cierre",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessWatchMessage {
    Event(ProcessEvent),
    TrackedProcessExited(ProcessInstanceId),
    Error(String),
    WatcherHealthy(ProcessWatcherKind),
    WatcherDegraded {
        kind: ProcessWatcherKind,
        error: String,
        retry_in_ms: u64,
    },
    Reevaluate,
    PromoteProfile(String),
    Shutdown,
}

#[cfg(windows)]
#[derive(Debug, serde::Deserialize)]
#[serde(rename = "Win32_ProcessStartTrace")]
#[serde(rename_all = "PascalCase")]
struct ProcessStartTrace {
    process_id: u32,
    process_name: String,
}

#[cfg(windows)]
#[derive(Debug, serde::Deserialize)]
#[serde(rename = "Win32_ProcessStopTrace")]
#[serde(rename_all = "PascalCase")]
struct ProcessStopTrace {
    process_id: u32,
    process_name: String,
}

pub fn spawn_process_event_watchers(sender: Sender<ProcessWatchMessage>) -> Vec<JoinHandle<()>> {
    spawn_platform_process_event_watchers(sender)
}

#[cfg(windows)]
fn spawn_platform_process_event_watchers(
    sender: Sender<ProcessWatchMessage>,
) -> Vec<JoinHandle<()>> {
    let start_sender = sender.clone();
    let start_handle = std::thread::spawn(move || {
        supervise_process_watcher(
            ProcessWatcherKind::Starts,
            start_sender,
            watch_process_starts,
        )
    });
    let stop_handle = std::thread::spawn(move || {
        supervise_process_watcher(ProcessWatcherKind::Stops, sender, watch_process_stops)
    });
    vec![start_handle, stop_handle]
}

#[cfg(not(windows))]
fn spawn_platform_process_event_watchers(
    _sender: Sender<ProcessWatchMessage>,
) -> Vec<JoinHandle<()>> {
    Vec::new()
}

#[cfg(windows)]
fn supervise_process_watcher(
    kind: ProcessWatcherKind,
    sender: Sender<ProcessWatchMessage>,
    watch_once: fn(ProcessWatcherKind, Sender<ProcessWatchMessage>) -> Result<(), String>,
) {
    let mut backoff = Duration::from_secs(1);

    loop {
        let started_at = std::time::Instant::now();
        match watch_once(kind.clone(), sender.clone()) {
            Ok(()) => break,
            Err(error) => {
                let retry_in_ms = duration_ms(backoff);
                let message = format!(
                    "Watcher WMI de {} degradado; reintentando en {} s. {error}",
                    kind.label(),
                    retry_in_ms / 1000
                );
                if sender
                    .send(ProcessWatchMessage::WatcherDegraded {
                        kind: kind.clone(),
                        error: message,
                        retry_in_ms,
                    })
                    .is_err()
                {
                    break;
                }
                std::thread::sleep(backoff);
                backoff = next_watcher_backoff(backoff, started_at.elapsed());
            }
        }
    }
}

#[cfg(windows)]
fn watch_process_starts(
    kind: ProcessWatcherKind,
    sender: Sender<ProcessWatchMessage>,
) -> Result<(), String> {
    use wmi::WMIConnection;

    let connection = WMIConnection::new().map_err(|error| error.to_string())?;
    let iterator = connection
        .raw_notification::<ProcessStartTrace>(PROCESS_START_TRACE_QUERY)
        .map_err(|error| error.to_string())?;
    if sender
        .send(ProcessWatchMessage::WatcherHealthy(kind))
        .is_err()
    {
        return Ok(());
    }
    for result in iterator {
        match result {
            Ok(trace) => {
                if sender.send(start_trace_to_event(trace)).is_err() {
                    return Ok(());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("la suscripcion WMI de inicio termino inesperadamente".to_string())
}

#[cfg(windows)]
fn watch_process_stops(
    kind: ProcessWatcherKind,
    sender: Sender<ProcessWatchMessage>,
) -> Result<(), String> {
    use wmi::WMIConnection;

    let connection = WMIConnection::new().map_err(|error| error.to_string())?;
    let iterator = connection
        .raw_notification::<ProcessStopTrace>(PROCESS_STOP_TRACE_QUERY)
        .map_err(|error| error.to_string())?;
    if sender
        .send(ProcessWatchMessage::WatcherHealthy(kind))
        .is_err()
    {
        return Ok(());
    }
    for result in iterator {
        match result {
            Ok(trace) => {
                if sender.send(stop_trace_to_event(trace)).is_err() {
                    return Ok(());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("la suscripcion WMI de cierre termino inesperadamente".to_string())
}

fn next_watcher_backoff(current: Duration, last_run: Duration) -> Duration {
    if last_run >= Duration::from_secs(30) {
        return Duration::from_secs(1);
    }
    current.saturating_mul(2).min(Duration::from_secs(60))
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(windows)]
fn start_trace_to_event(trace: ProcessStartTrace) -> ProcessWatchMessage {
    ProcessWatchMessage::Event(ProcessEvent {
        kind: ProcessEventKind::Started,
        pid: trace.process_id,
        name: trace.process_name,
        path: None,
    })
}

#[cfg(windows)]
fn stop_trace_to_event(trace: ProcessStopTrace) -> ProcessWatchMessage {
    ProcessWatchMessage::Event(ProcessEvent {
        kind: ProcessEventKind::Stopped,
        pid: trace.process_id,
        name: trace.process_name,
        path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_event_messages_are_cloneable_and_comparable() {
        let message = ProcessWatchMessage::Event(ProcessEvent {
            kind: ProcessEventKind::Started,
            pid: 42,
            name: "game.exe".to_string(),
            path: None,
        });

        assert_eq!(message.clone(), message);
        assert_eq!(ProcessWatchMessage::Shutdown, ProcessWatchMessage::Shutdown);
        assert_eq!(
            ProcessWatchMessage::WatcherHealthy(ProcessWatcherKind::Starts),
            ProcessWatchMessage::WatcherHealthy(ProcessWatcherKind::Starts)
        );
    }

    #[test]
    fn process_trace_queries_select_only_required_fields() {
        assert!(PROCESS_START_TRACE_QUERY.starts_with("SELECT ProcessID, ProcessName"));
        assert!(PROCESS_STOP_TRACE_QUERY.starts_with("SELECT ProcessID, ProcessName"));
        assert!(!PROCESS_START_TRACE_QUERY.contains("SELECT *"));
        assert!(!PROCESS_STOP_TRACE_QUERY.contains("SELECT *"));
    }

    #[test]
    fn watcher_backoff_grows_but_resets_after_stable_run() {
        assert_eq!(
            next_watcher_backoff(Duration::from_secs(1), Duration::from_secs(0)),
            Duration::from_secs(2)
        );
        assert_eq!(
            next_watcher_backoff(Duration::from_secs(60), Duration::from_secs(0)),
            Duration::from_secs(60)
        );
        assert_eq!(
            next_watcher_backoff(Duration::from_secs(30), Duration::from_secs(30)),
            Duration::from_secs(1)
        );
    }

    #[cfg(windows)]
    #[test]
    fn maps_start_trace_to_process_event() {
        let message = start_trace_to_event(ProcessStartTrace {
            process_id: 10,
            process_name: "game.exe".to_string(),
        });

        assert_eq!(
            message,
            ProcessWatchMessage::Event(ProcessEvent {
                kind: ProcessEventKind::Started,
                pid: 10,
                name: "game.exe".to_string(),
                path: None,
            })
        );
    }

    #[cfg(windows)]
    #[test]
    fn maps_stop_trace_to_process_event() {
        let message = stop_trace_to_event(ProcessStopTrace {
            process_id: 11,
            process_name: "game.exe".to_string(),
        });

        assert_eq!(
            message,
            ProcessWatchMessage::Event(ProcessEvent {
                kind: ProcessEventKind::Stopped,
                pid: 11,
                name: "game.exe".to_string(),
                path: None,
            })
        );
    }
}
