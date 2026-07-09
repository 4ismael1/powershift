pub mod autostart;
pub mod error;
pub mod icon;
pub mod identity;
pub mod ipc_events;
pub mod manager;
pub mod named_pipe;
pub mod powercfg;
pub mod process;
pub mod process_events;
pub mod runtime_paths;
pub mod single_instance;
pub mod wake;
pub mod window_events;

#[cfg(windows)]
pub mod native;

pub use autostart::{
    autostart_value_for, autostart_value_with_args, set_autostart, set_autostart_for_executable,
    TRAY_AUTOSTART_VALUE_NAME,
};
pub use error::{PowerError, PowerResult};
pub use icon::{png_data_url, png_data_url_from_executable};
pub use identity::current_user_sid_string;
pub use ipc_events::{
    create_ipc_event, signal_ipc_event, wait_for_ipc_event, AGENT_STATE_UPDATED_EVENT_NAME,
    EVENT_LOG_UPDATED_EVENT_NAME, TRAY_QUIT_EVENT_NAME, UI_EXIT_EVENT_NAME, UI_SHOW_EVENT_NAME,
};
pub use manager::{PowerManager, PowerManagerBackend};
pub use named_pipe::{
    agent_pipe_name, call_named_pipe, run_named_pipe_server, AGENT_PIPE_NAME_PREFIX,
};
pub use powercfg::{parse_powercfg_list, PowerCfgBackend};
pub use process::{
    inspect_process, process_id_is_running, register_process_exit_wait, ObservedProcess,
    ProcessExitWatch, ProcessInstanceId, ProcessSnapshotBackend, SystemProcessBackend,
};
pub use process_events::{
    spawn_process_event_watchers, ProcessEvent, ProcessEventKind, ProcessWatchMessage,
    ProcessWatcherKind,
};
pub use runtime_paths::PowerShiftPaths;
pub use single_instance::{
    try_acquire_single_instance, SingleInstanceGuard, AGENT_INSTANCE_MUTEX_NAME,
    TRAY_INSTANCE_MUTEX_NAME, UI_INSTANCE_MUTEX_NAME,
};
pub use wake::{
    create_agent_wake_event, signal_agent_wake, wait_for_agent_wake, AGENT_WAKE_EVENT_NAME,
};
pub use window_events::spawn_window_activity_watcher;
