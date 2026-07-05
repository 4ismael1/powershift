use crate::PowerResult;
use std::path::Path;

pub const AUTOSTART_VALUE_NAME: &str = "PowerShift";
pub const TRAY_AUTOSTART_VALUE_NAME: &str = "PowerShiftTray";
pub const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn autostart_value_for(exe_path: &Path, start_minimized: bool) -> String {
    let args = if start_minimized {
        vec!["--minimized"]
    } else {
        Vec::new()
    };
    autostart_value_with_args(exe_path, &args)
}

pub fn autostart_value_with_args(exe_path: &Path, args: &[&str]) -> String {
    let mut value = format!("\"{}\"", exe_path.display());
    for arg in args {
        value.push(' ');
        value.push_str(arg);
    }
    value
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool, start_minimized: bool) -> PowerResult<()> {
    let exe_path = std::env::current_exe().map_err(crate::PowerError::Io)?;
    let args = if start_minimized {
        vec!["--minimized"]
    } else {
        Vec::new()
    };
    set_autostart_for_executable(AUTOSTART_VALUE_NAME, enabled, &exe_path, &args)
}

#[cfg(windows)]
pub fn set_autostart_for_executable(
    value_name: &str,
    enabled: bool,
    exe_path: &Path,
    args: &[&str],
) -> PowerResult<()> {
    use crate::PowerError;
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu.create_subkey(RUN_KEY_PATH).map_err(PowerError::Io)?;

    if enabled {
        run_key
            .set_value(value_name, &autostart_value_with_args(exe_path, args))
            .map_err(PowerError::Io)?;
    } else {
        match run_key.delete_value(value_name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(PowerError::Io(error)),
        }
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn set_autostart(_enabled: bool, _start_minimized: bool) -> PowerResult<()> {
    Err(crate::PowerError::NotSupported("Windows autostart"))
}

#[cfg(not(windows))]
pub fn set_autostart_for_executable(
    _value_name: &str,
    _enabled: bool,
    _exe_path: &Path,
    _args: &[&str],
) -> PowerResult<()> {
    Err(crate::PowerError::NotSupported("Windows autostart"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn autostart_value_quotes_executable_path() {
        let value = autostart_value_for(
            &PathBuf::from(r"C:\Program Files\PowerShift\powershift.exe"),
            false,
        );

        assert_eq!(value, r#""C:\Program Files\PowerShift\powershift.exe""#);
    }

    #[test]
    fn autostart_value_adds_minimized_flag_when_enabled() {
        let value = autostart_value_for(&PathBuf::from(r"C:\Apps\PowerShift\powershift.exe"), true);

        assert_eq!(value, r#""C:\Apps\PowerShift\powershift.exe" --minimized"#);
    }

    #[test]
    fn autostart_value_supports_custom_arguments() {
        let value = autostart_value_with_args(
            &PathBuf::from(r"C:\Apps\PowerShift\powershift-tray.exe"),
            &["--open-ui"],
        );

        assert_eq!(
            value,
            r#""C:\Apps\PowerShift\powershift-tray.exe" --open-ui"#
        );
    }
}
