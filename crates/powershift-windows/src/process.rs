use crate::{PowerError, PowerResult, ProcessWatchMessage};
use powershift_core::ProcessInfo;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessInstanceId {
    pub pid: u32,
    /// Windows FILETIME for the process creation moment. Pairing it with the
    /// PID makes a late exit signal harmless if Windows reuses that PID.
    pub creation_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedProcess {
    pub instance: ProcessInstanceId,
    pub process: ProcessInfo,
    /// Session zero is reserved for services. A user-facing power profile
    /// should not become active because a similarly named service starts.
    pub session_id: Option<u32>,
}

/// Owns an OS thread-pool wait registration. Dropping it unregisters the wait
/// before the underlying process handle is released.
pub struct ProcessExitWatch {
    #[cfg(windows)]
    wait_handle: windows::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    process_handle: windows::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    callback: std::sync::Arc<ExitWaitCallback>,
    #[cfg(windows)]
    callback_context: *const ExitWaitCallback,
}

impl std::fmt::Debug for ProcessExitWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProcessExitWatch")
    }
}

pub trait ProcessSnapshotBackend {
    fn list_processes(&self) -> PowerResult<Vec<ProcessInfo>>;

    /// Lightweight process-table snapshot for the background agent. Backends
    /// may omit executable paths because the agent resolves metadata only for
    /// names that can affect a configured profile.
    fn list_processes_for_tracking(&self) -> PowerResult<Vec<ProcessInfo>> {
        self.list_processes()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProcessBackend;

impl ProcessSnapshotBackend for SystemProcessBackend {
    fn list_processes(&self) -> PowerResult<Vec<ProcessInfo>> {
        platform_list_processes(true).map(sort_processes)
    }

    fn list_processes_for_tracking(&self) -> PowerResult<Vec<ProcessInfo>> {
        platform_list_processes(false).map(sort_processes)
    }
}

#[cfg(windows)]
fn platform_list_processes(include_paths: bool) -> PowerResult<Vec<ProcessInfo>> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let current_session_id = process_session_id(std::process::id()).ok_or_else(|| {
        PowerError::Parse("No se pudo determinar la sesion actual de Windows".to_string())
    })?;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|error| PowerError::Parse(format!("CreateToolhelp32Snapshot fallo: {error}")))?;
    let result = (|| {
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..PROCESSENTRY32W::default()
        };
        match unsafe { Process32FirstW(snapshot, &mut entry) } {
            Ok(()) => {}
            Err(error) if error.code().0 as u32 == ERROR_NO_MORE_FILES.to_hresult().0 as u32 => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(PowerError::Parse(format!("Process32FirstW fallo: {error}")));
            }
        }

        let mut processes = Vec::new();
        loop {
            let name = utf16_nul_terminated(&entry.szExeFile);
            if entry.th32ProcessID != 0
                && !name.trim().is_empty()
                && process_session_id(entry.th32ProcessID) == Some(current_session_id)
            {
                processes.push(ProcessInfo {
                    pid: entry.th32ProcessID,
                    name,
                    path: if include_paths {
                        query_process_path_by_pid(entry.th32ProcessID)
                    } else {
                        None
                    },
                });
            }

            match unsafe { Process32NextW(snapshot, &mut entry) } {
                Ok(()) => {}
                Err(error)
                    if error.code().0 as u32 == ERROR_NO_MORE_FILES.to_hresult().0 as u32 =>
                {
                    break;
                }
                Err(error) => {
                    return Err(PowerError::Parse(format!("Process32NextW fallo: {error}")));
                }
            }
        }
        Ok(processes)
    })();
    let _ = unsafe { CloseHandle(snapshot) };
    result
}

#[cfg(not(windows))]
fn platform_list_processes(_include_paths: bool) -> PowerResult<Vec<ProcessInfo>> {
    Err(PowerError::NotSupported("Windows process snapshots"))
}

#[cfg(windows)]
fn utf16_nul_terminated(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}

#[cfg(windows)]
fn query_process_path_by_pid(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let path = query_process_path(handle);
    let _ = unsafe { CloseHandle(handle) };
    path
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

/// Reads only the metadata Windows exposes for a single process. This is used
/// for a WMI start event and never enumerates the process table.
pub fn inspect_process(pid: u32, fallback_name: &str) -> Option<ObservedProcess> {
    if pid == 0 {
        return None;
    }
    platform_inspect_process(pid, fallback_name)
}

/// Registers a one-shot Windows thread-pool wait for an exact process
/// instance. The callback only posts an in-memory message to the agent.
pub fn register_process_exit_wait(
    instance: ProcessInstanceId,
    sender: std::sync::mpsc::Sender<ProcessWatchMessage>,
) -> Option<ProcessExitWatch> {
    if instance.pid == 0 || instance.creation_time == 0 {
        return None;
    }
    platform_register_process_exit_wait(instance, sender)
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

#[cfg(windows)]
fn platform_inspect_process(pid: u32, fallback_name: &str) -> Option<ObservedProcess> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    }
    .ok()?;
    let current_session_id = process_session_id(std::process::id());
    let observed = observed_process_from_handle(handle, pid, fallback_name)
        .filter(|process| process_is_in_session(process, current_session_id));
    let _ = unsafe { CloseHandle(handle) };
    observed
}

#[cfg(windows)]
fn platform_register_process_exit_wait(
    instance: ProcessInstanceId,
    sender: std::sync::mpsc::Sender<ProcessWatchMessage>,
) -> Option<ProcessExitWatch> {
    use std::ffi::c_void;
    use std::sync::Arc;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{
        OpenProcess, RegisterWaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, WT_EXECUTEONLYONCE,
    };

    let process_handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            instance.pid,
        )
    }
    .ok()?;

    let Some(creation_time) = process_creation_time(process_handle) else {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(process_handle) };
        return None;
    };
    if creation_time != instance.creation_time {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(process_handle) };
        return None;
    }

    let callback = Arc::new(ExitWaitCallback {
        sender,
        instance,
        invoked: std::sync::atomic::AtomicBool::new(false),
    });
    let callback_context = Arc::into_raw(Arc::clone(&callback));
    let mut wait_handle = HANDLE::default();
    let registration = unsafe {
        RegisterWaitForSingleObject(
            &mut wait_handle,
            process_handle,
            Some(process_exit_callback),
            Some(callback_context.cast::<c_void>()),
            u32::MAX,
            WT_EXECUTEONLYONCE,
        )
    };
    if registration.is_err() {
        unsafe { drop(Arc::from_raw(callback_context)) };
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(process_handle) };
        return None;
    }

    Some(ProcessExitWatch {
        wait_handle,
        process_handle,
        callback,
        callback_context,
    })
}

#[cfg(not(windows))]
fn platform_process_id_is_running(_pid: u32) -> bool {
    false
}

#[cfg(not(windows))]
fn platform_inspect_process(_pid: u32, _fallback_name: &str) -> Option<ObservedProcess> {
    None
}

#[cfg(not(windows))]
fn platform_register_process_exit_wait(
    _instance: ProcessInstanceId,
    _sender: std::sync::mpsc::Sender<ProcessWatchMessage>,
) -> Option<ProcessExitWatch> {
    None
}

#[cfg(windows)]
struct ExitWaitCallback {
    sender: std::sync::mpsc::Sender<ProcessWatchMessage>,
    instance: ProcessInstanceId,
    invoked: std::sync::atomic::AtomicBool,
}

#[cfg(windows)]
unsafe extern "system" fn process_exit_callback(context: *mut std::ffi::c_void, _timed_out: bool) {
    if context.is_null() {
        return;
    }

    let callback = unsafe { std::sync::Arc::from_raw(context.cast::<ExitWaitCallback>()) };
    callback
        .invoked
        .store(true, std::sync::atomic::Ordering::Release);
    let _ = callback
        .sender
        .send(ProcessWatchMessage::TrackedProcessExited(
            callback.instance.clone(),
        ));
}

#[cfg(windows)]
impl Drop for ProcessExitWatch {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows::Win32::System::Threading::UnregisterWaitEx;

        // WT_EXECUTEONLYONCE stops future callbacks but does not release the
        // registered wait. A synchronous unregister also makes it safe to
        // reclaim the raw Arc when the callback never ran.
        let unregistered =
            unsafe { UnregisterWaitEx(self.wait_handle, Some(INVALID_HANDLE_VALUE)) };

        if unregistered.is_ok() {
            if !self.callback.invoked.load(Ordering::Acquire) {
                // No callback will consume the Arc retained by the Windows
                // registration, so reclaim it after synchronous unregister.
                unsafe { drop(std::sync::Arc::from_raw(self.callback_context)) };
            }
            let _ = unsafe { CloseHandle(self.process_handle) };
        }
    }
}

#[cfg(not(windows))]
impl Drop for ProcessExitWatch {
    fn drop(&mut self) {}
}

#[cfg(windows)]
fn observed_process_from_handle(
    handle: windows::Win32::Foundation::HANDLE,
    pid: u32,
    fallback_name: &str,
) -> Option<ObservedProcess> {
    if !process_handle_is_running(handle) {
        return None;
    }
    let creation_time = process_creation_time(handle)?;
    let path = query_process_path(handle);
    let name = path
        .as_deref()
        .and_then(file_name_from_path)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fallback_name.to_string());
    if name.trim().is_empty() {
        return None;
    }

    Some(ObservedProcess {
        instance: ProcessInstanceId { pid, creation_time },
        process: ProcessInfo { pid, name, path },
        session_id: process_session_id(pid),
    })
}

#[cfg(windows)]
fn process_session_id(pid: u32) -> Option<u32> {
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;

    let mut session_id = 0;
    unsafe { ProcessIdToSessionId(pid, &mut session_id) }
        .ok()
        .map(|()| session_id)
}

fn process_is_in_session(process: &ObservedProcess, current_session_id: Option<u32>) -> bool {
    process.session_id.is_some() && process.session_id == current_session_id
}

#[cfg(windows)]
fn process_handle_is_running(handle: windows::Win32::Foundation::HANDLE) -> bool {
    use windows::Win32::Foundation::STILL_ACTIVE;
    use windows::Win32::System::Threading::GetExitCodeProcess;

    let mut exit_code = 0;
    unsafe { GetExitCodeProcess(handle, &mut exit_code) }.is_ok()
        && exit_code == STILL_ACTIVE.0 as u32
}

#[cfg(windows)]
fn process_creation_time(handle: windows::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetProcessTimes;

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }.ok()?;
    Some((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

#[cfg(windows)]
fn query_process_path(handle: windows::Win32::Foundation::HANDLE) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_WIN32};

    let mut capacity = 260usize;
    while capacity <= 32_768 {
        let mut buffer = vec![0u16; capacity];
        let mut length = buffer.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .is_ok()
        {
            return Some(String::from_utf16_lossy(&buffer[..length as usize]));
        }
        capacity *= 2;
    }
    None
}

#[cfg(windows)]
fn file_name_from_path(path: &str) -> Option<String> {
    path.replace('/', "\\")
        .rsplit('\\')
        .next()
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
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

    #[cfg(windows)]
    #[test]
    fn system_backend_lists_at_least_current_test_process() {
        let processes = SystemProcessBackend
            .list_processes()
            .expect("list system processes");

        assert!(processes.iter().any(|process| process.pid > 0));
        assert!(processes.iter().any(|process| !process.name.is_empty()));
    }

    #[cfg(windows)]
    #[test]
    fn tracking_snapshot_avoids_global_executable_path_queries() {
        let processes = SystemProcessBackend
            .list_processes_for_tracking()
            .expect("list tracking processes");

        assert!(!processes.is_empty());
        assert!(processes.iter().all(|process| process.path.is_none()));
    }

    #[test]
    fn detects_current_process_as_running() {
        assert!(process_id_is_running(std::process::id()));
        assert!(!process_id_is_running(0));
    }

    #[test]
    fn session_filter_rejects_unknown_and_other_windows_sessions() {
        let process = ObservedProcess {
            instance: ProcessInstanceId {
                pid: 42,
                creation_time: 100,
            },
            process: ProcessInfo {
                pid: 42,
                name: "game.exe".to_string(),
                path: None,
            },
            session_id: Some(2),
        };

        assert!(process_is_in_session(&process, Some(2)));
        assert!(!process_is_in_session(&process, Some(3)));
        assert!(!process_is_in_session(&process, None));

        let mut unknown = process;
        unknown.session_id = None;
        assert!(!process_is_in_session(&unknown, Some(2)));
    }

    #[cfg(windows)]
    #[test]
    fn registered_exit_wait_reports_the_exact_child_instance() {
        use std::process::Command;
        use std::sync::mpsc;
        use std::time::Duration;

        let mut child = Command::new("cmd")
            .args(["/C", "ping 127.0.0.1 -n 3 > NUL"])
            .spawn()
            .expect("spawn short lived child");
        let observed = inspect_process(child.id(), "cmd.exe").expect("observe child process");
        assert_ne!(observed.instance.creation_time, 0);

        let (sender, receiver) = mpsc::channel();
        let _watch = register_process_exit_wait(observed.instance.clone(), sender)
            .expect("register process exit wait");
        child.wait().expect("wait for child");

        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("exit wait message"),
            ProcessWatchMessage::TrackedProcessExited(observed.instance),
        );
    }
}
