use powershift_core::ProcessInfo;
use powershift_windows::{ProcessSnapshotBackend, SystemProcessBackend};

pub fn list_open_processes_with<B: ProcessSnapshotBackend>(
    backend: &B,
) -> Result<Vec<ProcessInfo>, String> {
    backend.list_processes().map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_open_processes() -> Result<Vec<ProcessInfo>, String> {
    list_open_processes_with(&SystemProcessBackend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use powershift_windows::{PowerError, PowerResult};

    struct FakeProcessBackend {
        processes: Vec<ProcessInfo>,
        fail: bool,
    }

    impl ProcessSnapshotBackend for FakeProcessBackend {
        fn list_processes(&self) -> PowerResult<Vec<ProcessInfo>> {
            if self.fail {
                Err(PowerError::Parse("process boom".to_string()))
            } else {
                Ok(self.processes.clone())
            }
        }
    }

    #[test]
    fn list_open_processes_returns_backend_processes() {
        let backend = FakeProcessBackend {
            fail: false,
            processes: vec![ProcessInfo {
                pid: 123,
                name: "demo.exe".to_string(),
                path: Some("C:\\Games\\Demo\\demo.exe".to_string()),
            }],
        };

        let processes = list_open_processes_with(&backend).expect("process list");

        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].name, "demo.exe");
    }

    #[test]
    fn list_open_processes_converts_backend_errors_to_string() {
        let backend = FakeProcessBackend {
            fail: true,
            processes: Vec::new(),
        };

        let error = list_open_processes_with(&backend).expect_err("expected error");

        assert!(error.contains("process boom"));
    }
}
