use crate::PowerResult;
use powershift_core::ProcessInfo;
use sysinfo::System;

pub trait ProcessSnapshotBackend {
    fn list_processes(&self) -> PowerResult<Vec<ProcessInfo>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProcessBackend;

impl ProcessSnapshotBackend for SystemProcessBackend {
    fn list_processes(&self) -> PowerResult<Vec<ProcessInfo>> {
        let mut system = System::new();
        system.refresh_processes();
        Ok(sort_processes(
            system
                .processes()
                .iter()
                .map(|(pid, process)| ProcessInfo {
                    pid: pid.as_u32(),
                    name: process.name().to_string(),
                    path: process_path(process),
                })
                .collect(),
        ))
    }
}

pub fn sort_processes(mut processes: Vec<ProcessInfo>) -> Vec<ProcessInfo> {
    processes.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.pid.cmp(&right.pid))
    });
    processes
}

pub fn process_id_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    platform_process_id_is_running(pid)
}

#[cfg(windows)]
fn platform_process_id_is_running(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };

    let mut exit_code = 0;
    let running = unsafe { GetExitCodeProcess(handle, &mut exit_code).is_ok() }
        && exit_code == STILL_ACTIVE.0 as u32;
    let _ = unsafe { CloseHandle(handle) };
    running
}

#[cfg(not(windows))]
fn platform_process_id_is_running(_pid: u32) -> bool {
    false
}

fn process_path(process: &sysinfo::Process) -> Option<String> {
    process
        .exe()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_processes_orders_by_name_case_insensitively_then_pid() {
        let processes = vec![
            ProcessInfo {
                pid: 30,
                name: "zeta.exe".to_string(),
                path: None,
            },
            ProcessInfo {
                pid: 20,
                name: "Alpha.exe".to_string(),
                path: None,
            },
            ProcessInfo {
                pid: 10,
                name: "alpha.exe".to_string(),
                path: None,
            },
        ];

        let sorted = sort_processes(processes);

        assert_eq!(
            sorted.iter().map(|process| process.pid).collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
    }

    #[test]
    fn system_backend_lists_at_least_current_test_process() {
        let processes = SystemProcessBackend
            .list_processes()
            .expect("list system processes");

        assert!(processes.iter().any(|process| process.pid > 0));
        assert!(processes.iter().any(|process| !process.name.is_empty()));
    }

    #[test]
    fn detects_current_process_as_running() {
        assert!(process_id_is_running(std::process::id()));
        assert!(!process_id_is_running(0));
    }
}
