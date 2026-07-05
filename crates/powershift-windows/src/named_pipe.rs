use crate::PowerResult;

pub const AGENT_PIPE_NAME_PREFIX: &str = r"\\.\pipe\PowerShiftAgent";
pub const AGENT_PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";
const PIPE_BUFFER_BYTES: u32 = 256 * 1024;
const PIPE_CLIENT_TIMEOUT_MS: u32 = 1_500;

pub fn agent_pipe_name() -> String {
    format!("{AGENT_PIPE_NAME_PREFIX}-{}", current_session_id())
}

#[cfg(windows)]
fn current_session_id() -> u32 {
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::GetCurrentProcessId;

    let mut session_id = 0_u32;
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id)
            .map(|()| session_id)
            .unwrap_or_default()
    }
}

#[cfg(not(windows))]
fn current_session_id() -> u32 {
    0
}

#[cfg(windows)]
pub fn call_named_pipe(pipe_name: &str, request: &str) -> PowerResult<String> {
    use windows::core::HSTRING;
    use windows::Win32::System::Pipes::CallNamedPipeW;

    let request = request.as_bytes();
    let mut response = vec![0_u8; PIPE_BUFFER_BYTES as usize];
    let mut bytes_read = 0_u32;
    let ok = unsafe {
        CallNamedPipeW(
            &HSTRING::from(pipe_name),
            Some(request.as_ptr().cast()),
            request.len() as u32,
            Some(response.as_mut_ptr().cast()),
            response.len() as u32,
            &mut bytes_read,
            PIPE_CLIENT_TIMEOUT_MS,
        )
    };

    if !ok.as_bool() {
        return Err(crate::PowerError::Parse(
            windows::core::Error::from_win32().to_string(),
        ));
    }

    response.truncate(bytes_read as usize);
    String::from_utf8(response).map_err(|error| crate::PowerError::Parse(error.to_string()))
}

#[cfg(windows)]
pub fn run_named_pipe_server<F>(pipe_name: &str, mut handler: F) -> PowerResult<()>
where
    F: FnMut(String) -> String,
{
    loop {
        let (_descriptor, attributes) = named_pipe_security_attributes()?;
        let handle = create_server_pipe(pipe_name, &attributes)?;
        let result = serve_one_client(handle, &mut handler);
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
        result?;
    }
}

#[cfg(windows)]
fn create_server_pipe(
    pipe_name: &str,
    attributes: &windows::Win32::Security::SECURITY_ATTRIBUTES,
) -> PowerResult<windows::Win32::Foundation::HANDLE> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES,
        PIPE_WAIT,
    };

    let handle = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(pipe_name),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            PIPE_CLIENT_TIMEOUT_MS,
            Some(attributes as *const windows::Win32::Security::SECURITY_ATTRIBUTES),
        )
    };

    if handle.is_invalid() {
        Err(crate::PowerError::Parse(
            windows::core::Error::from_win32().to_string(),
        ))
    } else {
        Ok(handle)
    }
}

#[cfg(windows)]
fn serve_one_client<F>(
    handle: windows::Win32::Foundation::HANDLE,
    handler: &mut F,
) -> PowerResult<()>
where
    F: FnMut(String) -> String,
{
    connect_pipe_client(handle)?;
    let request = read_pipe_message(handle)?;
    let response = handler(request);
    write_pipe_message(handle, &response)?;
    unsafe {
        let _ = windows::Win32::Storage::FileSystem::FlushFileBuffers(handle);
        let _ = windows::Win32::System::Pipes::DisconnectNamedPipe(handle);
    }
    Ok(())
}

#[cfg(windows)]
fn connect_pipe_client(handle: windows::Win32::Foundation::HANDLE) -> PowerResult<()> {
    use windows::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED};
    use windows::Win32::System::Pipes::ConnectNamedPipe;

    match unsafe { ConnectNamedPipe(handle, None) } {
        Ok(()) => Ok(()),
        Err(error) => {
            if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
                Ok(())
            } else {
                Err(crate::PowerError::Parse(error.to_string()))
            }
        }
    }
}

#[cfg(windows)]
fn read_pipe_message(handle: windows::Win32::Foundation::HANDLE) -> PowerResult<String> {
    use windows::Win32::Foundation::{GetLastError, ERROR_MORE_DATA};
    use windows::Win32::Storage::FileSystem::ReadFile;

    let mut output = Vec::new();
    loop {
        let mut chunk = vec![0_u8; 8192];
        let mut bytes_read = 0_u32;
        let result = unsafe { ReadFile(handle, Some(&mut chunk), Some(&mut bytes_read), None) };
        chunk.truncate(bytes_read as usize);
        output.extend_from_slice(&chunk);

        match result {
            Ok(()) => break,
            Err(_error) if unsafe { GetLastError() } == ERROR_MORE_DATA => continue,
            Err(error) => return Err(crate::PowerError::Parse(error.to_string())),
        }
    }

    String::from_utf8(output).map_err(|error| crate::PowerError::Parse(error.to_string()))
}

#[cfg(windows)]
fn write_pipe_message(
    handle: windows::Win32::Foundation::HANDLE,
    response: &str,
) -> PowerResult<()> {
    use windows::Win32::Storage::FileSystem::WriteFile;

    let bytes = response.as_bytes();
    let mut bytes_written = 0_u32;
    unsafe { WriteFile(handle, Some(bytes), Some(&mut bytes_written), None) }
        .map_err(|error| crate::PowerError::Parse(error.to_string()))?;
    if bytes_written as usize == bytes.len() {
        Ok(())
    } else {
        Err(crate::PowerError::Parse(format!(
            "partial named pipe write: {bytes_written}/{} bytes",
            bytes.len()
        )))
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
fn named_pipe_security_attributes() -> PowerResult<(
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
            &HSTRING::from(AGENT_PIPE_SDDL),
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

#[cfg(not(windows))]
pub fn call_named_pipe(_pipe_name: &str, _request: &str) -> PowerResult<String> {
    Err(crate::PowerError::NotSupported("named pipe client"))
}

#[cfg(not(windows))]
pub fn run_named_pipe_server<F>(_pipe_name: &str, _handler: F) -> PowerResult<()>
where
    F: FnMut(String) -> String,
{
    Err(crate::PowerError::NotSupported("named pipe server"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_pipe_name_uses_session_scoped_local_named_pipe_namespace() {
        let name = agent_pipe_name();

        assert!(name.starts_with(r"\\.\pipe\PowerShiftAgent-"));
    }

    #[test]
    fn agent_pipe_security_allows_interactive_read_write() {
        assert!(AGENT_PIPE_SDDL.contains(";;;SY"));
        assert!(AGENT_PIPE_SDDL.contains(";;;BA"));
        assert!(AGENT_PIPE_SDDL.contains("GRGW;;;IU"));
        assert!(!AGENT_PIPE_SDDL.contains("GA;;;IU"));
    }
}
