use crate::PowerResult;

pub const UI_SHOW_EVENT_NAME: &str = "Local\\PowerShiftUiShow";
pub const UI_EXIT_EVENT_NAME: &str = "Local\\PowerShiftUiExit";
pub const TRAY_QUIT_EVENT_NAME: &str = "Local\\PowerShiftTrayQuit";
pub const EVENT_LOG_UPDATED_EVENT_NAME: &str = "Local\\PowerShiftEventLogUpdated";
pub const AGENT_STATE_UPDATED_UI_EVENT_NAME: &str = "Local\\PowerShiftAgentStateUpdatedUi";
pub const AGENT_STATE_UPDATED_TRAY_EVENT_NAME: &str = "Local\\PowerShiftAgentStateUpdatedTray";

#[cfg(windows)]
pub type EventHandle = windows::Win32::Foundation::HANDLE;

#[cfg(windows)]
pub fn create_ipc_event(name: &str) -> PowerResult<EventHandle> {
    use windows::core::HSTRING;
    use windows::Win32::System::Threading::CreateEventW;

    unsafe {
        CreateEventW(None, false, false, &HSTRING::from(name))
            .map_err(|error| crate::PowerError::Parse(error.to_string()))
    }
}

#[cfg(windows)]
pub fn signal_ipc_event(name: &str) -> PowerResult<()> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};

    let handle = unsafe {
        OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(name))
            .map_err(|error| crate::PowerError::Parse(error.to_string()))?
    };
    unsafe {
        SetEvent(handle).map_err(|error| crate::PowerError::Parse(error.to_string()))?;
        let _ = CloseHandle(handle);
    }
    Ok(())
}

#[cfg(windows)]
pub fn wait_for_ipc_event(handle: EventHandle) -> PowerResult<()> {
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};

    let result = unsafe { WaitForSingleObject(handle, INFINITE) };
    if result == WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err(crate::PowerError::Parse(format!(
            "WaitForSingleObject returned {:?}",
            result
        )))
    }
}

#[cfg(not(windows))]
pub fn create_ipc_event(_name: &str) -> PowerResult<()> {
    Err(crate::PowerError::NotSupported("Windows IPC events"))
}

#[cfg(not(windows))]
pub fn signal_ipc_event(_name: &str) -> PowerResult<()> {
    Err(crate::PowerError::NotSupported("Windows IPC events"))
}

/// Broadcasts a state invalidation to every resident consumer. Win32
/// auto-reset events wake only one waiter, so the UI and tray intentionally use
/// distinct event objects instead of competing for a shared signal.
pub fn signal_agent_state_updated() {
    let _ = signal_ipc_event(AGENT_STATE_UPDATED_UI_EVENT_NAME);
    let _ = signal_ipc_event(AGENT_STATE_UPDATED_TRAY_EVENT_NAME);
}

#[cfg(not(windows))]
pub fn wait_for_ipc_event(_handle: ()) -> PowerResult<()> {
    Err(crate::PowerError::NotSupported("Windows IPC events"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_are_session_local() {
        assert!(UI_SHOW_EVENT_NAME.starts_with("Local\\"));
        assert!(UI_EXIT_EVENT_NAME.starts_with("Local\\"));
        assert!(TRAY_QUIT_EVENT_NAME.starts_with("Local\\"));
        assert!(EVENT_LOG_UPDATED_EVENT_NAME.starts_with("Local\\"));
        assert!(AGENT_STATE_UPDATED_UI_EVENT_NAME.starts_with("Local\\"));
        assert!(AGENT_STATE_UPDATED_TRAY_EVENT_NAME.starts_with("Local\\"));
        assert_ne!(
            AGENT_STATE_UPDATED_UI_EVENT_NAME,
            AGENT_STATE_UPDATED_TRAY_EVENT_NAME
        );
    }
}
