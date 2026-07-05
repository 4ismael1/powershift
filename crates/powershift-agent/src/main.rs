#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let result = match agent_cli_mode(&args) {
        AgentCliMode::ScanOnce => powershift_agent::run_scan_once().and_then(|scan| {
            println!(
                "{}",
                serde_json::to_string(&scan).map_err(|error| error.to_string())?
            );
            Ok(())
        }),
        AgentCliMode::StatusIpc => {
            powershift_agent::request_agent_status_via_ipc().and_then(|state| {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&state).map_err(|error| error.to_string())?
                );
                Ok(())
            })
        }
        AgentCliMode::ReevaluateIpc => powershift_agent::request_agent_reevaluate_via_ipc(),
        AgentCliMode::ShutdownIpc => powershift_agent::request_agent_shutdown_via_ipc(),
        AgentCliMode::Signal => {
            powershift_windows::signal_agent_wake().map_err(|error| error.to_string())
        }
        AgentCliMode::Run => {
            let instance = match powershift_windows::try_acquire_single_instance(
                powershift_windows::AGENT_INSTANCE_MUTEX_NAME,
            ) {
                Ok(Some(instance)) => instance,
                Ok(None) => return,
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            };
            let _keep_instance_alive = &instance;
            powershift_agent::run_agent_forever()
        }
    };

    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentCliMode {
    ScanOnce,
    StatusIpc,
    ReevaluateIpc,
    ShutdownIpc,
    Signal,
    Run,
}

fn agent_cli_mode(args: &[String]) -> AgentCliMode {
    if args.iter().any(|arg| arg == "--scan-once") {
        AgentCliMode::ScanOnce
    } else if args.iter().any(|arg| arg == "--status-ipc") {
        AgentCliMode::StatusIpc
    } else if args.iter().any(|arg| arg == "--reevaluate-ipc") {
        AgentCliMode::ReevaluateIpc
    } else if args.iter().any(|arg| arg == "--shutdown-ipc") {
        AgentCliMode::ShutdownIpc
    } else if args.iter().any(|arg| arg == "--signal") {
        AgentCliMode::Signal
    } else {
        AgentCliMode::Run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(flag: &str) -> Vec<String> {
        vec!["powershift-agent.exe".to_string(), flag.to_string()]
    }

    #[test]
    fn cli_mode_supports_ipc_commands() {
        assert_eq!(
            agent_cli_mode(&args("--status-ipc")),
            AgentCliMode::StatusIpc
        );
        assert_eq!(
            agent_cli_mode(&args("--reevaluate-ipc")),
            AgentCliMode::ReevaluateIpc
        );
        assert_eq!(
            agent_cli_mode(&args("--shutdown-ipc")),
            AgentCliMode::ShutdownIpc
        );
    }

    #[test]
    fn cli_mode_defaults_to_run() {
        assert_eq!(
            agent_cli_mode(&["powershift-agent.exe".to_string()]),
            AgentCliMode::Run
        );
    }
}
