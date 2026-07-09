use crate::{PowerError, PowerResult};

pub const LEGACY_AGENT_TASK_NAME: &str = "PowerShiftAgent";
const AGENT_TASK_NAME_PREFIX: &str = "PowerShiftAgent-";

pub fn agent_task_name() -> PowerResult<String> {
    current_user_sid_string().map(|sid| agent_task_name_for_sid(&sid))
}

pub fn agent_task_name_for_sid(sid: &str) -> String {
    format!("{AGENT_TASK_NAME_PREFIX}{sid}")
}

#[cfg(windows)]
pub fn current_user_sid_string() -> PowerResult<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|error| PowerError::Parse(error.to_string()))?;

    let result = (|| {
        let mut required_bytes = 0_u32;
        let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut required_bytes) };
        if required_bytes == 0 {
            return Err(PowerError::Parse(
                "GetTokenInformation returned an empty user token".to_string(),
            ));
        }

        let mut buffer = vec![0_u8; required_bytes as usize];
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                required_bytes,
                &mut required_bytes,
            )
        }
        .map_err(|error| PowerError::Parse(error.to_string()))?;

        let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut sid_string = PWSTR::null();
        unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) }
            .map_err(|error| PowerError::Parse(error.to_string()))?;
        let value =
            unsafe { sid_string.to_string() }.map_err(|error| PowerError::Parse(error.to_string()));
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sid_string.0.cast())));
        }
        value
    })();

    unsafe {
        let _ = CloseHandle(token);
    }
    result
}

#[cfg(not(windows))]
pub fn current_user_sid_string() -> PowerResult<String> {
    Err(PowerError::NotSupported("current user SID"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn current_user_sid_uses_windows_sid_format() {
        let sid = current_user_sid_string().expect("current user SID");

        assert!(sid.starts_with("S-1-"));
        assert!(sid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
    }

    #[test]
    fn agent_task_name_is_stable_and_scoped_to_one_sid() {
        assert_eq!(
            agent_task_name_for_sid("S-1-5-21-1000"),
            "PowerShiftAgent-S-1-5-21-1000"
        );
        assert_ne!(
            agent_task_name_for_sid("S-1-5-21-1000"),
            agent_task_name_for_sid("S-1-5-21-2000")
        );
    }
}
