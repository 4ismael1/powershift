use crate::PowerResult;

pub const AGENT_WAKE_EVENT_NAME: &str = "Local\\PowerShiftAgentWake";
pub const AGENT_WAKE_EVENT_SDDL: &str =
    "D:P(A;;0x00100002;;;SY)(A;;0x00100002;;;BA)(A;;0x00100002;;;IU)";

#[cfg(windows)]
pub fn create_agent_wake_event() -> PowerResult<windows::Win32::Foundation::HANDLE> {
    use windows::core::HSTRING;
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::System::Threading::CreateEventW;

    let (_descriptor, attributes) = wake_event_security_attributes()?;
    unsafe {
        CreateEventW(
            Some(&attributes as *const SECURITY_ATTRIBUTES),
            false,
            false,
            &HSTRING::from(AGENT_WAKE_EVENT_NAME),
        )
        .map_err(|error| crate::PowerError::Parse(error.to_string()))
    }
}

#[cfg(windows)]
struct LocalSecurityDescriptor(windows::Win32::Security::PSECURITY_DESCRIPTOR);

#[cfg(windows)]
impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        if self.0.is_invalid() {
            return;
        }

        unsafe {
            let _ = windows::Win32::Foundation::LocalFree(Some(
                windows::Win32::Foundation::HLOCAL(self.0 .0),
            ));
        }
    }
}

#[cfg(windows)]
fn wake_event_security_attributes() -> PowerResult<(
    LocalSecurityDescriptor,
    windows::Win32::Security::SECURITY_ATTRIBUTES,
)> {
    use windows::core::HSTRING;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(AGENT_WAKE_EVENT_SDDL),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|error| crate::PowerError::Parse(error.to_string()))?;
    }

    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };

    Ok((LocalSecurityDescriptor(descriptor), attributes))
}

#[cfg(windows)]
pub fn signal_agent_wake() -> PowerResult<()> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};

    let handle = unsafe {
        OpenEventW(
            EVENT_MODIFY_STATE,
            false,
            &HSTRING::from(AGENT_WAKE_EVENT_NAME),
        )
        .map_err(|error| crate::PowerError::Parse(error.to_string()))?
    };
    unsafe {
        SetEvent(handle).map_err(|error| crate::PowerError::Parse(error.to_string()))?;
        let _ = CloseHandle(handle);
    }
    Ok(())
}

#[cfg(windows)]
pub fn wait_for_agent_wake(handle: windows::Win32::Foundation::HANDLE) -> PowerResult<()> {
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
pub fn signal_agent_wake() -> PowerResult<()> {
    Err(crate::PowerError::NotSupported("agent wake event"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_event_name_is_session_local() {
        assert!(AGENT_WAKE_EVENT_NAME.starts_with("Local\\"));
    }

    #[test]
    fn wake_event_sddl_allows_interactive_ui_to_signal_elevated_agent() {
        assert!(AGENT_WAKE_EVENT_SDDL.contains("0x00100002"));
        assert!(AGENT_WAKE_EVENT_SDDL.contains(";;;SY"));
        assert!(AGENT_WAKE_EVENT_SDDL.contains(";;;BA"));
        assert!(AGENT_WAKE_EVENT_SDDL.contains(";;;IU"));
    }
}
