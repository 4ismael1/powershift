#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|argument| argument == powershift_lib::REPAIR_AGENT_TASK_FLAG) {
        if powershift_lib::repair_agent_task_elevated_cli().is_err() {
            std::process::exit(1);
        }
        return;
    }

    powershift_lib::run();
}
