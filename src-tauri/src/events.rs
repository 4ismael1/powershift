use powershift_agent::AgentPaths;
use powershift_agent::EventLogEntry;
use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

#[cfg(test)]
use std::{fs::OpenOptions, io::Write};

pub fn event_log_path() -> Result<PathBuf, String> {
    AgentPaths::from_environment()
        .map(|paths| paths.events)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
pub fn append_event_to_path(path: PathBuf, entry: &EventLogEntry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let line = serde_json::to_string(entry).map_err(|error| error.to_string())?;
    writeln!(file, "{line}").map_err(|error| error.to_string())
}

pub fn read_recent_events(limit: usize) -> Result<Vec<EventLogEntry>, String> {
    read_recent_events_from_path(event_log_path()?, limit)
}

pub fn clear_event_history() -> Result<(), String> {
    powershift_agent::request_agent_clear_events_via_ipc()
}

#[cfg(test)]
pub fn clear_event_history_at_path(path: PathBuf) -> Result<(), String> {
    remove_file_if_present(&path)?;
    remove_file_if_present(&path.with_extension("jsonl.1"))?;
    Ok(())
}

#[cfg(test)]
fn remove_file_if_present(path: &PathBuf) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn read_recent_events_from_path(
    path: PathBuf,
    limit: usize,
) -> Result<Vec<EventLogEntry>, String> {
    if !path.exists() || limit == 0 {
        return Ok(Vec::new());
    }

    let file = File::open(path).map_err(|error| error.to_string())?;
    let reader = BufReader::new(file);
    let mut parsed_events = Vec::new();

    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if let Ok(event) = serde_json::from_str::<EventLogEntry>(&line) {
            parsed_events.push(event);
        }
    }

    let mut events = VecDeque::with_capacity(limit);
    for event in parsed_events.into_iter().rev() {
        if should_hide_legacy_event(&event) {
            continue;
        }

        events.push_back(event);
        if events.len() == limit {
            break;
        }
    }

    Ok(events.into_iter().collect())
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_recent_events(limit: Option<usize>) -> Result<Vec<EventLogEntry>, String> {
    read_recent_events(limit.unwrap_or(50).min(200))
}

#[tauri::command(rename_all = "snake_case")]
pub fn clear_events() -> Result<(), String> {
    clear_event_history()
}

fn should_hide_legacy_event(event: &EventLogEntry) -> bool {
    event.kind == "process_watcher_error"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "powershift-events-{name}-{}.jsonl",
            std::process::id()
        ))
    }

    #[test]
    fn appends_and_reads_recent_events_newest_first() {
        let path = temp_log_path("recent");
        let _ = std::fs::remove_file(&path);

        append_event_to_path(path.clone(), &EventLogEntry::info("first", "Primer evento"))
            .expect("append first");
        append_event_to_path(
            path.clone(),
            &EventLogEntry::info("second", "Segundo evento"),
        )
        .expect("append second");

        let events = read_recent_events_from_path(path.clone(), 1).expect("read events");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "second");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn skips_invalid_json_lines() {
        let path = temp_log_path("invalid");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "not-json\n{\"timestamp_ms\":1,\"level\":\"info\",\"kind\":\"ok\",\"message\":\"ok\",\"profile_name\":null,\"plan_id\":null}\n",
        )
        .expect("write temp log");

        let events = read_recent_events_from_path(path.clone(), 10).expect("read events");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "ok");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn zero_limit_returns_no_events() {
        let path = temp_log_path("zero");
        let _ = std::fs::remove_file(&path);

        let events = read_recent_events_from_path(path, 0).expect("read events");

        assert!(events.is_empty());
    }

    #[test]
    fn hides_legacy_process_watcher_errors() {
        let path = temp_log_path("watcher-compact");
        let _ = std::fs::remove_file(&path);

        append_event_to_path(
            path.clone(),
            &EventLogEntry::error("process_watcher_error", "old"),
        )
        .expect("append old watcher error");
        append_event_to_path(
            path.clone(),
            &EventLogEntry::info("profile_activated", "ok"),
        )
        .expect("append profile event");
        append_event_to_path(
            path.clone(),
            &EventLogEntry::error("process_watcher_error", "new"),
        )
        .expect("append new watcher error");

        let events = read_recent_events_from_path(path.clone(), 10).expect("read events");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "profile_activated");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn clear_event_history_removes_current_and_rotated_logs() {
        let path = temp_log_path("clear");
        let rotated = path.with_extension("jsonl.1");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&rotated);
        std::fs::write(&path, "current").expect("write current log");
        std::fs::write(&rotated, "rotated").expect("write rotated log");

        clear_event_history_at_path(path.clone()).expect("clear history");

        assert!(!path.exists());
        assert!(!rotated.exists());
    }
}
