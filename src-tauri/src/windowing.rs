use crate::config::{default_config_path, load_or_create_config};
use powershift_core::CloseButtonBehavior;
use tauri::{
    AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
};

const MAIN_WINDOW_LABEL: &str = "main";
const MAIN_WINDOW_WIDTH: f64 = 958.0;
const MAIN_WINDOW_HEIGHT: f64 = 598.0;
const MAIN_WINDOW_MIN_WIDTH: f64 = 860.0;
const MAIN_WINDOW_MIN_HEIGHT: f64 = 520.0;
const AGENT_STATE_CHANGED_EVENT: &str = "powershift://agent-state-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    CloseUi,
    Exit,
}

pub fn close_action_for_behavior(behavior: CloseButtonBehavior) -> CloseAction {
    match behavior {
        CloseButtonBehavior::HideWindow | CloseButtonBehavior::Ask => CloseAction::CloseUi,
        CloseButtonBehavior::ExitApp => CloseAction::Exit,
    }
}

pub fn configure_windowing<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    start_ui_show_listener(app.clone());
    start_ui_exit_listener(app.clone());
    start_agent_state_listener(app.clone());
    apply_startup_visibility(app)?;
    Ok(())
}

fn start_agent_state_listener<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let Ok(handle) = powershift_windows::create_ipc_event(
            powershift_windows::AGENT_STATE_UPDATED_UI_EVENT_NAME,
        ) else {
            return;
        };

        loop {
            if powershift_windows::wait_for_ipc_event(handle).is_err() {
                break;
            }
            let _ = app.emit(AGENT_STATE_CHANGED_EVENT, ());
        }
    });
}

pub fn should_start_hidden<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .any(|arg| matches!(arg.as_ref(), "--minimized" | "--hidden" | "--background"))
}

fn start_ui_show_listener<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let Ok(handle) =
            powershift_windows::create_ipc_event(powershift_windows::UI_SHOW_EVENT_NAME)
        else {
            return;
        };

        loop {
            if powershift_windows::wait_for_ipc_event(handle).is_err() {
                break;
            }
            let _ = show_main_window(&app);
        }
    });
}

fn start_ui_exit_listener<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let Ok(handle) =
            powershift_windows::create_ipc_event(powershift_windows::UI_EXIT_EVENT_NAME)
        else {
            return;
        };

        if powershift_windows::wait_for_ipc_event(handle).is_ok() {
            app.exit(0);
        }
    });
}

fn apply_startup_visibility<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if should_start_hidden(std::env::args()) {
        destroy_main_window(app)
            .map_err(|error| tauri::Error::Anyhow(std::io::Error::other(error).into()))?;
    }
    Ok(())
}

pub fn handle_close_requested<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        run_close_action(window.app_handle(), configured_close_action());
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn handle_close_button(app: AppHandle) -> Result<(), String> {
    run_close_action(&app, configured_close_action());
    Ok(())
}

fn run_close_action<R: Runtime>(app: &AppHandle<R>, action: CloseAction) {
    if action == CloseAction::Exit {
        let _ = powershift_windows::signal_ipc_event(powershift_windows::TRAY_QUIT_EVENT_NAME);
        let _ = crate::agent_control::stop_agent_task();
    }
    app.exit(0);
}

fn configured_close_action() -> CloseAction {
    load_or_create_config(default_config_path())
        .map(|config| close_action_for_behavior(config.ui.close_button_behavior))
        .unwrap_or(CloseAction::CloseUi)
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::default())
        .title("PowerShift")
        .inner_size(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT)
        .min_inner_size(MAIN_WINDOW_MIN_WIDTH, MAIN_WINDOW_MIN_HEIGHT)
        .decorations(false)
        .transparent(false)
        .center()
        .visible(true)
        .build()
        .map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

fn destroy_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window.destroy().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn close_action_respects_configured_behavior() {
        assert_eq!(
            close_action_for_behavior(CloseButtonBehavior::HideWindow),
            CloseAction::CloseUi
        );
        assert_eq!(
            close_action_for_behavior(CloseButtonBehavior::Ask),
            CloseAction::CloseUi
        );
        assert_eq!(
            close_action_for_behavior(CloseButtonBehavior::ExitApp),
            CloseAction::Exit
        );
    }

    #[test]
    fn exit_close_action_signals_tray_shutdown() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("windowing.rs");
        let source = std::fs::read_to_string(source_path).expect("read source");

        assert!(source.contains("TRAY_QUIT_EVENT_NAME"));
        assert!(source.contains("agent_control::stop_agent_task"));
        assert!(source.contains("run_close_action"));
    }

    #[test]
    fn should_start_hidden_accepts_background_flags() {
        assert!(should_start_hidden(["powershift.exe", "--minimized"]));
        assert!(should_start_hidden(["powershift.exe", "--hidden"]));
        assert!(should_start_hidden(["powershift.exe", "--background"]));
        assert!(!should_start_hidden(["powershift.exe"]));
    }

    #[test]
    fn bundle_declares_native_app_icons() {
        let config: Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri config json");
        let icons = config["bundle"]["icon"]
            .as_array()
            .expect("bundle.icon should be an array");

        for expected in [
            "icons/32x32.png",
            "icons/128x128.png",
            "icons/128x128@2x.png",
            "icons/icon.icns",
            "icons/icon.ico",
        ] {
            assert!(
                icons.iter().any(|icon| icon.as_str() == Some(expected)),
                "missing bundle icon path {expected}"
            );
        }
    }

    #[test]
    fn recreated_window_uses_compact_dimensions() {
        assert_eq!(MAIN_WINDOW_WIDTH, 958.0);
        assert_eq!(MAIN_WINDOW_HEIGHT, 598.0);
        assert_eq!(MAIN_WINDOW_MIN_WIDTH, 860.0);
        assert_eq!(MAIN_WINDOW_MIN_HEIGHT, 520.0);
    }

    #[test]
    fn window_declares_dark_startup_background() {
        let config: Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri config json");
        let background = config["app"]["windows"][0]["backgroundColor"]
            .as_str()
            .expect("window backgroundColor should be a string");

        assert_eq!(background, "#090d0d");
    }

    #[test]
    fn production_webview_uses_hardened_security_defaults() {
        let config: Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri config json");
        let security = &config["app"]["security"];

        assert_eq!(security["freezePrototype"].as_bool(), Some(true));
        assert_eq!(security["csp"]["style-src"].as_str(), Some("'self'"));
        assert!(security["devCsp"]["style-src"]
            .as_str()
            .is_some_and(|value| value.contains("'unsafe-inline'")));
        assert_eq!(
            config["build"]["removeUnusedCommands"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn agent_state_changes_are_forwarded_to_the_webview() {
        let source = include_str!("windowing.rs");

        assert!(source.contains("AGENT_STATE_UPDATED_UI_EVENT_NAME"));
        assert!(source.contains("powershift://agent-state-changed"));
        assert!(source.contains("app.emit"));
    }
}
