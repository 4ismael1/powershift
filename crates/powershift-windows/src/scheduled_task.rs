use crate::{PowerError, PowerResult};

pub fn set_agent_startup_trigger_enabled(enabled: bool) -> PowerResult<()> {
    set_agent_startup_trigger_enabled_for(&crate::agent_task_name()?, enabled)
}

#[cfg(windows)]
fn set_agent_startup_trigger_enabled_for(task_name: &str, enabled: bool) -> PowerResult<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let script = startup_trigger_script(task_name, enabled);
    let status = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(PowerError::Io)?;

    if status.success() {
        Ok(())
    } else {
        Err(PowerError::CommandFailed {
            command: "Set-ScheduledTask",
            code: status.code(),
            stderr: String::new(),
        })
    }
}

#[cfg(not(windows))]
fn set_agent_startup_trigger_enabled_for(_task_name: &str, _enabled: bool) -> PowerResult<()> {
    Err(PowerError::NotSupported("scheduled task startup trigger"))
}

fn startup_trigger_script(task_name: &str, enabled: bool) -> String {
    let enabled = if enabled { "$true" } else { "$false" };
    format!(
        "$task = Get-ScheduledTask -TaskName '{task_name}' -ErrorAction Stop; \
         foreach ($trigger in $task.Triggers) {{ $trigger.Enabled = {enabled} }}; \
         Set-ScheduledTask -TaskName '{task_name}' -Trigger $task.Triggers | Out-Null"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_trigger_script_changes_only_trigger_enabled_state() {
        let disabled = startup_trigger_script("PowerShiftAgent-S-1-5-21-1000", false);
        let enabled = startup_trigger_script("PowerShiftAgent-S-1-5-21-1000", true);

        assert!(disabled.contains("$trigger.Enabled = $false"));
        assert!(enabled.contains("$trigger.Enabled = $true"));
        assert!(disabled.contains("Set-ScheduledTask"));
        assert!(!disabled.contains("Stop-ScheduledTask"));
        assert!(!disabled.contains("Unregister-ScheduledTask"));
    }
}
