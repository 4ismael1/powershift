use crate::{PowerError, PowerResult};
use std::path::Path;

#[cfg(windows)]
pub fn run_elevated_and_wait(executable: &Path, arguments: &str) -> PowerResult<u32> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let verb = HSTRING::from("runas");
    let file = HSTRING::from(executable.as_os_str());
    let parameters = HSTRING::from(arguments);
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    unsafe { ShellExecuteExW(&mut execute) }
        .map_err(|error| PowerError::Parse(error.to_string()))?;
    if execute.hProcess.is_invalid() {
        return Err(PowerError::Parse(
            "elevated process did not return a process handle".to_string(),
        ));
    }

    let result = unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    let exit_code = if result == WAIT_OBJECT_0 {
        let mut exit_code = 0_u32;
        unsafe { GetExitCodeProcess(execute.hProcess, &mut exit_code) }
            .map_err(|error| PowerError::Parse(error.to_string()))?;
        Ok(exit_code)
    } else {
        Err(PowerError::Parse(format!(
            "waiting for elevated process failed: {result:?}"
        )))
    };
    unsafe {
        let _ = CloseHandle(execute.hProcess);
    }
    exit_code
}

#[cfg(not(windows))]
pub fn run_elevated_and_wait(_executable: &Path, _arguments: &str) -> PowerResult<u32> {
    Err(PowerError::NotSupported("elevated process launch"))
}
