use crate::{current_user_sid_string, PowerError, PowerResult};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerShiftPaths {
    pub config: PathBuf,
    pub events: PathBuf,
    pub state: PathBuf,
}

impl PowerShiftPaths {
    pub fn from_environment() -> PowerResult<Self> {
        let app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| PowerError::Parse("APPDATA is not available".to_string()))?;
        let program_data = std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .ok_or_else(|| PowerError::Parse("PROGRAMDATA is not available".to_string()))?;
        Self::from_roots(app_data, program_data, current_user_sid_string()?)
    }

    fn from_roots(app_data: PathBuf, program_data: PathBuf, user_sid: String) -> PowerResult<Self> {
        validate_sid_component(&user_sid)?;
        let config_dir = app_data.join("PowerShift");
        let runtime_dir = program_data.join("PowerShift").join("users").join(user_sid);
        Ok(Self {
            config: config_dir.join("config.json"),
            events: runtime_dir.join("events.jsonl"),
            state: runtime_dir.join("agent-state.json"),
        })
    }

    pub fn runtime_dir(&self) -> &Path {
        self.state.parent().unwrap_or_else(|| Path::new("."))
    }

    pub fn control_token(&self) -> PathBuf {
        self.runtime_dir().join("agent-control.token")
    }

    pub fn power_control_lease(&self) -> PathBuf {
        self.runtime_dir().join("power-control-lease.json")
    }

    pub fn prepare_runtime_directory(&self) -> PowerResult<()> {
        prepare_runtime_directory(self.runtime_dir())
    }
}

fn validate_sid_component(sid: &str) -> PowerResult<()> {
    if sid.starts_with("S-1-")
        && sid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Ok(())
    } else {
        Err(PowerError::Parse("invalid Windows user SID".to_string()))
    }
}

#[cfg(windows)]
fn prepare_runtime_directory(runtime_dir: &Path) -> PowerResult<()> {
    let layout = runtime_directory_layout(runtime_dir)?;
    let descriptor = runtime_directory_security_descriptor(&layout.user_sid);

    // Parent directories are shared by every Windows account using PowerShift.
    // Giving either parent a per-user protected DACL would make the last user
    // that starts the agent lock out every other account. Only the SID-scoped
    // leaf contains private runtime data and receives the protected ACL.
    ensure_plain_directory(&layout.product_dir)?;
    ensure_plain_directory(&layout.users_dir)?;
    secure_directory(&layout.private_dir, &descriptor)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeDirectoryLayout {
    product_dir: PathBuf,
    users_dir: PathBuf,
    private_dir: PathBuf,
    user_sid: String,
}

fn runtime_directory_layout(runtime_dir: &Path) -> PowerResult<RuntimeDirectoryLayout> {
    let user_sid = runtime_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| PowerError::Parse("runtime directory has no user SID".to_string()))?;
    validate_sid_component(user_sid)?;
    let users_dir = runtime_dir
        .parent()
        .ok_or_else(|| PowerError::Parse("runtime directory has no users parent".to_string()))?;
    let product_dir = users_dir
        .parent()
        .ok_or_else(|| PowerError::Parse("runtime directory has no product parent".to_string()))?;

    Ok(RuntimeDirectoryLayout {
        product_dir: product_dir.to_path_buf(),
        users_dir: users_dir.to_path_buf(),
        private_dir: runtime_dir.to_path_buf(),
        user_sid: user_sid.to_string(),
    })
}

#[cfg(windows)]
fn ensure_plain_directory(path: &Path) -> PowerResult<()> {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(PowerError::Parse(error.to_string())),
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| PowerError::Parse(error.to_string()))?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(PowerError::Parse(format!(
            "refusing unsafe runtime directory: {}",
            path.display()
        )));
    }

    Ok(())
}

#[cfg(windows)]
fn secure_directory(path: &Path, sddl: &str) -> PowerResult<()> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };

    ensure_plain_directory(path)?;

    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|error| PowerError::Parse(error.to_string()))?;
    }
    let success = unsafe {
        SetFileSecurityW(
            &HSTRING::from(path.as_os_str()),
            DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    let result = if success.as_bool() {
        Ok(())
    } else {
        Err(PowerError::Parse(
            windows::core::Error::from_win32().to_string(),
        ))
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result
}

#[cfg(not(windows))]
fn prepare_runtime_directory(runtime_dir: &Path) -> PowerResult<()> {
    std::fs::create_dir_all(runtime_dir).map_err(|error| PowerError::Parse(error.to_string()))
}

fn runtime_directory_security_descriptor(user_sid: &str) -> String {
    format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{user_sid})S:(ML;OICI;NW;;;HI)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_user_config_from_elevated_runtime_files() {
        let paths = PowerShiftPaths::from_roots(
            PathBuf::from(r"C:\Users\Test\AppData\Roaming"),
            PathBuf::from(r"C:\ProgramData"),
            "S-1-5-21-1000".to_string(),
        )
        .expect("paths");

        assert_eq!(
            paths.config,
            PathBuf::from(r"C:\Users\Test\AppData\Roaming\PowerShift\config.json")
        );
        assert_eq!(
            paths.state,
            PathBuf::from(r"C:\ProgramData\PowerShift\users\S-1-5-21-1000\agent-state.json")
        );
        assert_eq!(
            paths.events,
            PathBuf::from(r"C:\ProgramData\PowerShift\users\S-1-5-21-1000\events.jsonl")
        );
        assert_eq!(
            paths.power_control_lease(),
            PathBuf::from(
                r"C:\ProgramData\PowerShift\users\S-1-5-21-1000\power-control-lease.json"
            )
        );
    }

    #[test]
    fn runtime_acl_is_high_integrity_and_scoped_to_the_user() {
        let descriptor = runtime_directory_security_descriptor("S-1-5-21-1000");

        assert!(descriptor.contains(";;;SY"));
        assert!(descriptor.contains(";;;BA"));
        assert!(descriptor.contains(";;;S-1-5-21-1000"));
        assert!(descriptor.contains("NW;;;HI"));
        assert!(!descriptor.contains(";;;IU"));
    }

    #[test]
    fn runtime_layout_keeps_shared_parents_separate_from_private_user_directories() {
        let first =
            runtime_directory_layout(Path::new(r"C:\ProgramData\PowerShift\users\S-1-5-21-1000"))
                .expect("first layout");
        let second =
            runtime_directory_layout(Path::new(r"C:\ProgramData\PowerShift\users\S-1-5-21-2000"))
                .expect("second layout");

        assert_eq!(first.product_dir, second.product_dir);
        assert_eq!(first.users_dir, second.users_dir);
        assert_ne!(first.private_dir, second.private_dir);
        assert_eq!(first.user_sid, "S-1-5-21-1000");
        assert_eq!(second.user_sid, "S-1-5-21-2000");
    }

    #[test]
    fn rejects_unsafe_sid_path_components() {
        assert!(PowerShiftPaths::from_roots(
            PathBuf::from("app"),
            PathBuf::from("program"),
            "..\\escape".to_string(),
        )
        .is_err());
    }
}
