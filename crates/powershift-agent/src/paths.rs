use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaths {
    pub config: PathBuf,
    pub events: PathBuf,
    pub state: PathBuf,
}

impl AgentPaths {
    pub fn from_app_data() -> Self {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join("PowerShift");
        Self {
            config: base.join("config.json"),
            events: base.join("events.jsonl"),
            state: base.join("agent-state.json"),
        }
    }

    pub fn control_token(&self) -> PathBuf {
        self.state
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agent-control.token")
    }
}
