use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const GITHUB_PROFILE_URL: &str = "https://github.com/4ismael1";

#[tauri::command(rename_all = "snake_case")]
pub fn open_executable_folder(executable_path: String) -> Result<(), String> {
    open_executable_folder_path(PathBuf::from(executable_path))
}

pub fn open_executable_folder_path(path: PathBuf) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("No hay ruta de ejecutable para abrir.".to_string());
    }

    if path.exists() {
        return reveal_path_in_explorer(&path);
    }

    let Some(parent) = path.parent().filter(|parent| parent.exists()) else {
        return Err("La carpeta del ejecutable no existe.".to_string());
    };

    open_folder_in_explorer(parent)
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_external_url(url: String) -> Result<(), String> {
    open_external_url_value(&url)
}

pub fn open_external_url_value(url: &str) -> Result<(), String> {
    if url != GITHUB_PROFILE_URL {
        return Err("URL externa no permitida.".to_string());
    }

    let mut command = Command::new("explorer.exe");
    configure_quiet_command(&mut command);
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn reveal_path_in_explorer(path: &Path) -> Result<(), String> {
    let mut command = Command::new("explorer.exe");
    configure_quiet_command(&mut command);
    let status = command
        .arg(format!("/select,{}", path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;

    if status.success() {
        Ok(())
    } else {
        open_folder_in_explorer(path.parent().unwrap_or_else(|| Path::new(".")))
    }
}

fn open_folder_in_explorer(path: &Path) -> Result<(), String> {
    let mut command = Command::new("explorer.exe");
    configure_quiet_command(&mut command);
    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn configure_quiet_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_quiet_command(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_executable_path_is_rejected() {
        let error = open_executable_folder_path(PathBuf::new()).expect_err("expected empty error");

        assert!(error.contains("No hay ruta"));
    }

    #[test]
    fn missing_parent_is_reported() {
        let path = PathBuf::from(format!(
            "C:\\PowerShift\\missing-{}\\game.exe",
            std::process::id()
        ));

        let error = open_executable_folder_path(path).expect_err("expected missing parent");

        assert!(error.contains("no existe"));
    }

    #[test]
    fn external_url_opener_allows_only_project_profile() {
        assert!(open_external_url_value("https://example.com").is_err());
        assert_eq!(GITHUB_PROFILE_URL, "https://github.com/4ismael1");
    }
}
