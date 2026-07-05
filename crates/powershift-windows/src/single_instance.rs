use crate::PowerResult;

pub const UI_INSTANCE_MUTEX_NAME: &str = "Local\\PowerShiftUiInstance";
pub const TRAY_INSTANCE_MUTEX_NAME: &str = "Local\\PowerShiftTrayInstance";
pub const AGENT_INSTANCE_MUTEX_NAME: &str = "Local\\PowerShiftAgentInstance";

#[cfg(windows)]
#[derive(Debug)]
pub struct SingleInstanceGuard {
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.handle);
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
pub fn try_acquire_single_instance(name: &str) -> PowerResult<Option<SingleInstanceGuard>> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let handle = unsafe {
        CreateMutexW(None, true, &HSTRING::from(name))
            .map_err(|error| crate::PowerError::Parse(error.to_string()))?
    };
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_exists {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
        Ok(None)
    } else {
        Ok(Some(SingleInstanceGuard { handle }))
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
pub struct SingleInstanceGuard;

#[cfg(not(windows))]
pub fn try_acquire_single_instance(_name: &str) -> PowerResult<Option<SingleInstanceGuard>> {
    Ok(Some(SingleInstanceGuard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutex_names_are_session_local() {
        assert!(UI_INSTANCE_MUTEX_NAME.starts_with("Local\\"));
        assert!(TRAY_INSTANCE_MUTEX_NAME.starts_with("Local\\"));
        assert!(AGENT_INSTANCE_MUTEX_NAME.starts_with("Local\\"));
    }
}
