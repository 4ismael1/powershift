mod agent_control;
mod config;
mod events;
mod icons;
mod power;
mod processes;
mod shell;
mod windowing;

pub fn repair_agent_task_elevated_cli() -> Result<(), String> {
    agent_control::repair_agent_task_elevated_cli()
}

pub use agent_control::REPAIR_AGENT_TASK_FLAG;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let ui_instance = match powershift_windows::try_acquire_single_instance(
        powershift_windows::UI_INSTANCE_MUTEX_NAME,
    ) {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            let _ = powershift_windows::signal_ipc_event(powershift_windows::UI_SHOW_EVENT_NAME);
            return;
        }
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            windowing::configure_windowing(app.handle())?;
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let _ = config::sync_startup_shell_settings(&app_handle);
            });
            Ok(())
        })
        .on_window_event(windowing::handle_close_requested)
        .invoke_handler(tauri::generate_handler![
            agent_control::agent_task_installed,
            agent_control::get_agent_state,
            agent_control::install_agent_task,
            agent_control::promote_active_profile,
            agent_control::start_agent_task,
            agent_control::wake_agent,
            config::get_app_config,
            config::save_app_config,
            config::take_config_recovery_warning,
            events::clear_events,
            events::get_recent_events,
            icons::get_executable_icon,
            power::get_power_plans,
            power::get_active_power_plan,
            power::set_active_power_plan,
            processes::get_open_processes,
            shell::open_executable_folder,
            shell::open_external_url,
            windowing::handle_close_button
        ])
        .build(tauri::generate_context!())
        .expect("error while building PowerShift")
        .run(move |_app, event| {
            let _keep_instance_alive = &ui_instance;
            if let tauri::RunEvent::ExitRequested { .. } = event {}
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn ui_instance_mutex_name_is_stable() {
        assert_eq!(
            powershift_windows::UI_INSTANCE_MUTEX_NAME,
            "Local\\PowerShiftUiInstance"
        );
    }

    #[test]
    fn startup_shell_sync_is_not_blocking_tauri_setup() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("lib.rs");
        let source = std::fs::read_to_string(source_path).expect("read source");

        assert!(source.contains("std::thread::spawn"));
        assert!(source.contains("sync_startup_shell_settings"));
    }
}
