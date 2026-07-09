#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    if let Err(error) = powershift_tray_main() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("powershift-tray is only supported on Windows");
}

#[cfg(windows)]
fn powershift_tray_main() -> Result<(), String> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--quit") {
        let _ = powershift_windows::signal_ipc_event(powershift_windows::UI_EXIT_EVENT_NAME);
        let _ = powershift_windows::signal_ipc_event(powershift_windows::TRAY_QUIT_EVENT_NAME);
        stop_agent_task();
        return Ok(());
    }

    let instance = powershift_windows::try_acquire_single_instance(
        powershift_windows::TRAY_INSTANCE_MUTEX_NAME,
    )
    .map_err(|error| error.to_string())?;
    if instance.is_none() {
        if args.iter().any(|arg| arg == "--open-ui") {
            let _ = launch_or_show_ui();
        }
        return Ok(());
    }

    if args.iter().any(|arg| arg == "--open-ui") {
        let _ = launch_or_show_ui();
    }

    tray::run_tray_loop()
}

#[cfg(windows)]
fn launch_or_show_ui() -> Result<(), String> {
    if powershift_windows::signal_ipc_event(powershift_windows::UI_SHOW_EVENT_NAME).is_ok() {
        return Ok(());
    }

    let ui_path = resolve_ui_exe_path().ok_or_else(|| {
        "No se encontro powershift.exe junto al tray o en rutas de desarrollo.".to_string()
    })?;
    let mut command = std::process::Command::new(ui_path);
    configure_quiet_command(&mut command);
    command.spawn().map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(windows)]
fn resolve_ui_exe_path() -> Option<std::path::PathBuf> {
    ui_exe_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
}

#[cfg(windows)]
fn ui_exe_candidates() -> Vec<std::path::PathBuf> {
    let current = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let directory = current
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let mut candidates = vec![directory.join("powershift.exe")];
    if let Some(parent) = directory.parent() {
        candidates.push(parent.join("powershift.exe"));
    }
    candidates.push(cwd.join("powershift.exe"));
    candidates.push(
        cwd.join("src-tauri")
            .join("target")
            .join("debug")
            .join("powershift.exe"),
    );
    candidates.push(
        cwd.join("src-tauri")
            .join("target")
            .join("release")
            .join("powershift.exe"),
    );
    candidates
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(windows)]
const AGENT_TASK_NAME: &str = "PowerShiftAgent";

#[cfg(windows)]
fn configure_quiet_command(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(windows)]
fn stop_agent_task() {
    if agent_ipc_command("shutdown").is_ok() {
        return;
    }

    let mut command = std::process::Command::new("schtasks");
    configure_quiet_command(&mut command);
    let _ = command
        .args(["/End", "/TN", AGENT_TASK_NAME])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(windows)]
fn agent_ipc_command(command: &str) -> Result<serde_json::Value, String> {
    let request = agent_ipc_request(command).to_string();
    let pipe_name = powershift_windows::agent_pipe_name();
    let response = powershift_windows::call_named_pipe(&pipe_name, &request)
        .map_err(|error| error.to_string())?;
    let value =
        serde_json::from_str::<serde_json::Value>(&response).map_err(|error| error.to_string())?;
    if value["ok"].as_bool() == Some(true) {
        Ok(value)
    } else {
        Err(value["message"]
            .as_str()
            .unwrap_or("El agente rechazo el comando IPC.")
            .to_string())
    }
}

#[cfg(windows)]
fn agent_ipc_request(command: &str) -> serde_json::Value {
    agent_ipc_request_with_token(command, read_agent_control_token())
}

#[cfg(windows)]
fn agent_ipc_request_with_token(command: &str, token: Option<String>) -> serde_json::Value {
    match (command, token) {
        ("get_status", _) => serde_json::json!({ "command": command }),
        (_, Some(token)) => serde_json::json!({ "command": command, "token": token }),
        _ => serde_json::json!({ "command": command }),
    }
}

#[cfg(windows)]
fn read_agent_control_token() -> Option<String> {
    let path = powershift_windows::PowerShiftPaths::from_environment()
        .ok()?
        .control_token();
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    (token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| token.to_string())
}

#[cfg(windows)]
mod tray {
    use super::agent_ipc_command;
    use super::launch_or_show_ui;
    use super::stop_agent_task;
    use std::sync::{Mutex, OnceLock};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{
        GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
        NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage, RegisterClassW,
        SetForegroundWindow, TrackPopupMenu, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
        IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND,
        WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    };

    const TRAY_UID: u32 = 1;
    pub(super) const WM_TRAY: u32 = WM_APP + 42;
    pub(super) const WM_TRAY_QUIT: u32 = WM_APP + 43;
    pub(super) const WM_EVENT_LOG_UPDATED: u32 = WM_APP + 44;
    pub(super) const WM_AGENT_STATE_UPDATED: u32 = WM_APP + 45;
    pub(super) const WM_CONTEXTMENU_VALUE: u32 = 0x007b;
    pub(super) const NIN_SELECT_VALUE: u32 = 0x0400;
    pub(super) const NIN_KEYSELECT_VALUE: u32 = 0x0401;
    const CMD_SHOW: usize = 1001;
    const CMD_EXIT: usize = 1002;
    static LAST_NOTIFICATION_EVENT: OnceLock<Mutex<Option<String>>> = OnceLock::new();

    pub fn run_tray_loop() -> Result<(), String> {
        let hwnd = create_hidden_window()?;
        add_tray_icon(hwnd)?;
        refresh_tray_tooltip(hwnd);
        prime_notification_cursor();
        spawn_quit_listener(hwnd);
        spawn_event_log_listener(hwnd);
        spawn_agent_state_listener(hwnd);

        let mut message = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if result.0 <= 0 {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        remove_tray_icon(hwnd);
        Ok(())
    }

    fn create_hidden_window() -> Result<HWND, String> {
        let instance = unsafe { GetModuleHandleW(None).map_err(|error| error.to_string())? };
        let class_name = w!("PowerShiftTrayWindow");
        let window_class = WNDCLASSW {
            hInstance: instance.into(),
            lpszClassName: class_name,
            lpfnWndProc: Some(window_proc),
            style: CS_HREDRAW | CS_VREDRAW,
            ..Default::default()
        };

        let atom = unsafe { RegisterClassW(&window_class) };
        if atom == 0 {
            let error = unsafe { GetLastError() };
            if error.0 != 1410 {
                return Err(format!("RegisterClassW failed: {}", error.0));
            }
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!("PowerShift Tray"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|error| error.to_string())?
        };
        Ok(hwnd)
    }

    fn add_tray_icon(hwnd: HWND) -> Result<(), String> {
        let icon = tray_icon().map_err(|error| error.to_string())?;
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            ..Default::default()
        };
        write_tip(&mut data, "PowerShift");

        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() };
        if !added {
            return Err("Shell_NotifyIconW(NIM_ADD) failed".to_string());
        }

        Ok(())
    }

    fn tray_icon() -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
        let instance = unsafe { GetModuleHandleW(None)? };
        unsafe {
            LoadIconW(Some(HINSTANCE(instance.0)), make_int_resource(1))
                .or_else(|_| LoadIconW(None, IDI_APPLICATION))
        }
    }

    #[allow(clippy::manual_dangling_ptr)]
    fn make_int_resource(id: usize) -> PCWSTR {
        PCWSTR(id as *const u16)
    }

    fn remove_tray_icon(hwnd: HWND) {
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            ..Default::default()
        };
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }

    fn refresh_tray_tooltip(hwnd: HWND) {
        let active_plan_name = current_power_plan_name();
        let active_profile_name = current_profile_name();
        update_tray_tooltip(
            hwnd,
            &tray_tooltip_text(active_plan_name.as_deref(), active_profile_name.as_deref()),
        );
    }

    fn update_tray_tooltip(hwnd: HWND, value: &str) {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_TIP,
            ..Default::default()
        };
        write_tip(&mut data, value);
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    fn write_tip(data: &mut NOTIFYICONDATAW, value: &str) {
        write_wide_truncated(&mut data.szTip, value);
    }

    fn write_wide_truncated<const N: usize>(target: &mut [u16; N], value: &str) {
        target.fill(0);
        let source = value.encode_utf16().collect::<Vec<_>>();
        let max = target.len().saturating_sub(1);
        for (index, ch) in source.iter().take(max).enumerate() {
            target[index] = *ch;
        }
    }

    fn current_power_plan_name() -> Option<String> {
        use powershift_windows::PowerManagerBackend;

        powershift_windows::PowerManager::new()
            .active_plan()
            .ok()
            .map(|plan| plan.name)
    }

    fn current_profile_name() -> Option<String> {
        if let Ok(response) = agent_ipc_command("get_status") {
            if let Some(profile_name) = profile_name_from_ipc_response(&response) {
                return Some(profile_name);
            }
        }

        let path = powershift_windows::PowerShiftPaths::from_environment()
            .ok()?
            .state;
        let value = std::fs::read_to_string(path).ok()?;
        profile_name_from_state_json(&value)
    }

    pub(super) fn profile_name_from_ipc_response(value: &serde_json::Value) -> Option<String> {
        value
            .get("state")?
            .get("last_scan")?
            .get("matched_profile_name")?
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .map(ToOwned::to_owned)
    }

    pub(super) fn profile_name_from_state_json(value: &str) -> Option<String> {
        let value = serde_json::from_str::<serde_json::Value>(value).ok()?;
        value
            .get("last_scan")?
            .get("matched_profile_name")?
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .map(ToOwned::to_owned)
    }

    pub(super) fn tray_tooltip_text(plan_name: Option<&str>, profile_name: Option<&str>) -> String {
        match (profile_name, plan_name) {
            (Some(profile), Some(plan)) => format!("PowerShift - {profile} activo - {plan}"),
            (Some(profile), None) => format!("PowerShift - {profile} activo"),
            (None, Some(plan)) => format!("PowerShift - Plan actual: {plan}"),
            (None, None) => "PowerShift".to_string(),
        }
    }

    fn prime_notification_cursor() {
        set_last_notification_key(latest_event().and_then(|event| notification_key(&event)));
    }

    fn show_latest_event_notification(hwnd: HWND) {
        let Some(event) = latest_event() else {
            return;
        };
        let Some(key) = notification_key(&event) else {
            return;
        };
        if last_notification_key().as_deref() == Some(key.as_str()) {
            return;
        }
        set_last_notification_key(Some(key));

        if !should_notify_event(&event, current_config_json().as_ref()) {
            return;
        }

        let message = event
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("PowerShift actualizo un evento.");
        show_balloon_notification(hwnd, "PowerShift", message);
    }

    fn show_balloon_notification(hwnd: HWND, title: &str, message: &str) {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_INFO,
            ..Default::default()
        };
        write_wide_truncated(&mut data.szInfoTitle, title);
        write_wide_truncated(&mut data.szInfo, message);
        data.dwInfoFlags = windows::Win32::UI::Shell::NIIF_INFO;
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    fn last_notification_key() -> Option<String> {
        LAST_NOTIFICATION_EVENT
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }

    fn set_last_notification_key(key: Option<String>) {
        if let Ok(mut value) = LAST_NOTIFICATION_EVENT
            .get_or_init(|| Mutex::new(None))
            .lock()
        {
            *value = key;
        }
    }

    fn latest_event() -> Option<serde_json::Value> {
        let path = powershift_windows::PowerShiftPaths::from_environment()
            .ok()?
            .events;
        let value = std::fs::read_to_string(path).ok()?;
        value
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
    }

    pub(super) fn notification_key(event: &serde_json::Value) -> Option<String> {
        Some(format!(
            "{}:{}:{}",
            event.get("timestamp_ms")?.as_u64()?,
            event.get("kind")?.as_str()?,
            event.get("message")?.as_str()?
        ))
    }

    fn current_config_json() -> Option<serde_json::Value> {
        let path = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)?
            .join("PowerShift")
            .join("config.json");
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub(super) fn should_notify_event(
        event: &serde_json::Value,
        config: Option<&serde_json::Value>,
    ) -> bool {
        if !global_notifications_enabled(config) {
            return false;
        }

        let kind = event.get("kind").and_then(serde_json::Value::as_str);
        if kind == Some("agent_error") {
            return true;
        }

        let Some(profile_name) = event
            .get("profile_name")
            .and_then(serde_json::Value::as_str)
        else {
            return false;
        };
        let Some(profile) = find_profile_by_name(config, profile_name) else {
            return true;
        };

        let notifications = &profile["notifications"];
        match kind {
            Some("profile_activated") => notifications["on_activate"].as_bool().unwrap_or(true),
            Some("power_plan_restored") => notifications["on_restore"].as_bool().unwrap_or(true),
            _ => false,
        }
    }

    fn global_notifications_enabled(config: Option<&serde_json::Value>) -> bool {
        config
            .and_then(|config| config.get("automation"))
            .and_then(|automation| automation.get("notifications_enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    }

    fn find_profile_by_name<'a>(
        config: Option<&'a serde_json::Value>,
        profile_name: &str,
    ) -> Option<&'a serde_json::Value> {
        config?.get("profiles")?.as_array()?.iter().find(|profile| {
            profile
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(profile_name))
        })
    }

    fn spawn_quit_listener(hwnd: HWND) {
        let hwnd_value = hwnd.0 as isize;
        std::thread::spawn(move || {
            let Ok(handle) =
                powershift_windows::create_ipc_event(powershift_windows::TRAY_QUIT_EVENT_NAME)
            else {
                return;
            };

            if powershift_windows::wait_for_ipc_event(handle).is_ok() {
                unsafe {
                    let hwnd = HWND(hwnd_value as *mut _);
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(hwnd),
                        WM_TRAY_QUIT,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    fn spawn_event_log_listener(hwnd: HWND) {
        let hwnd_value = hwnd.0 as isize;
        std::thread::spawn(move || {
            let Ok(handle) = powershift_windows::create_ipc_event(
                powershift_windows::EVENT_LOG_UPDATED_EVENT_NAME,
            ) else {
                return;
            };

            loop {
                if powershift_windows::wait_for_ipc_event(handle).is_err() {
                    break;
                }
                unsafe {
                    let hwnd = HWND(hwnd_value as *mut _);
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(hwnd),
                        WM_EVENT_LOG_UPDATED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    fn spawn_agent_state_listener(hwnd: HWND) {
        let hwnd_value = hwnd.0 as isize;
        std::thread::spawn(move || {
            let Ok(handle) = powershift_windows::create_ipc_event(
                powershift_windows::AGENT_STATE_UPDATED_TRAY_EVENT_NAME,
            ) else {
                return;
            };

            loop {
                if powershift_windows::wait_for_ipc_event(handle).is_err() {
                    break;
                }
                unsafe {
                    let hwnd = HWND(hwnd_value as *mut _);
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(hwnd),
                        WM_AGENT_STATE_UPDATED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_TRAY => {
                handle_tray_message(hwnd, lparam.0 as u32);
                LRESULT(0)
            }
            WM_COMMAND => {
                handle_command(hwnd, wparam.0 & 0xffff);
                LRESULT(0)
            }
            WM_CONTEXTMENU_VALUE => {
                refresh_tray_tooltip(hwnd);
                show_context_menu(hwnd);
                LRESULT(0)
            }
            WM_AGENT_STATE_UPDATED => {
                refresh_tray_tooltip(hwnd);
                LRESULT(0)
            }
            WM_EVENT_LOG_UPDATED => {
                show_latest_event_notification(hwnd);
                LRESULT(0)
            }
            WM_TRAY_QUIT => unsafe {
                let _ =
                    powershift_windows::signal_ipc_event(powershift_windows::UI_EXIT_EVENT_NAME);
                stop_agent_task();
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            },
            WM_DESTROY => {
                remove_tray_icon(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn handle_tray_message(hwnd: HWND, message: u32) {
        match message {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT_VALUE | NIN_KEYSELECT_VALUE => {
                let _ = launch_or_show_ui();
            }
            WM_RBUTTONUP | WM_CONTEXTMENU_VALUE => show_context_menu(hwnd),
            _ => {}
        }
    }

    fn show_context_menu(hwnd: HWND) {
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
            return;
        };

        unsafe {
            let _ = AppendMenuW(menu, MF_STRING, CMD_SHOW, w!("Abrir PowerShift"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, w!("Salir"));
        }

        let mut point = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut point);
            let _ = SetForegroundWindow(hwnd);
            let selected = TrackPopupMenu(
                menu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
                point.x,
                point.y,
                Some(0),
                hwnd,
                None,
            );
            if selected.0 > 0 {
                handle_command(hwnd, selected.0 as usize);
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hwnd),
                WM_NULL,
                WPARAM(0),
                LPARAM(0),
            );
            let _ = DestroyMenu(menu);
        }
    }

    fn handle_command(hwnd: HWND, command: usize) {
        match command {
            CMD_SHOW => {
                let _ = launch_or_show_ui();
            }
            CMD_EXIT => unsafe {
                let _ =
                    powershift_windows::signal_ipc_event(powershift_windows::UI_EXIT_EVENT_NAME);
                stop_agent_task();
                let _ = DestroyWindow(hwnd);
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ui_candidates_include_adjacent_executable() {
        let candidates = super::ui_exe_candidates();

        assert!(candidates.iter().any(|candidate| candidate
            .file_name()
            .and_then(|name| name.to_str())
            == Some("powershift.exe")));
    }

    #[test]
    fn tray_ui_messages_cover_legacy_and_modern_shell_callbacks() {
        assert_eq!(super::tray::WM_CONTEXTMENU_VALUE, 0x007b);
        assert_eq!(super::tray::NIN_SELECT_VALUE, 0x0400);
        assert_eq!(super::tray::NIN_KEYSELECT_VALUE, 0x0401);
        assert_ne!(super::tray::WM_TRAY, super::tray::WM_TRAY_QUIT);
        assert_ne!(super::tray::WM_TRAY_QUIT, super::tray::WM_EVENT_LOG_UPDATED);
    }

    #[test]
    fn tray_tooltip_prefers_profile_and_current_plan() {
        assert_eq!(
            super::tray::tray_tooltip_text(Some("Equilibrado"), Some("Chrome")),
            "PowerShift - Chrome activo - Equilibrado"
        );
        assert_eq!(
            super::tray::tray_tooltip_text(Some("Equilibrado"), None),
            "PowerShift - Plan actual: Equilibrado"
        );
        assert_eq!(super::tray::tray_tooltip_text(None, None), "PowerShift");
    }

    #[test]
    fn tray_reads_profile_name_from_published_state() {
        let state = r#"{
          "last_scan": { "matched_profile_name": "Apex Legends" }
        }"#;

        assert_eq!(
            super::tray::profile_name_from_state_json(state).as_deref(),
            Some("Apex Legends")
        );
        assert_eq!(
            super::tray::profile_name_from_state_json(r#"{"last_scan": null}"#),
            None
        );
    }

    #[test]
    fn tray_reads_profile_name_from_ipc_state() {
        let state = serde_json::json!({
            "ok": true,
            "state": {
                "pid": 10,
                "status": "running",
                "updated_at_ms": 1,
                "last_scan": {
                    "matched_profile_id": "chrome",
                    "matched_profile_name": "Chrome",
                    "target_plan_id": "high",
                    "active_profiles": [],
                    "changed_power_plan": false,
                    "restore_scheduled": false,
                    "restored_power_plan": false
                },
                "last_error": null
            },
            "message": null
        });

        assert_eq!(
            super::tray::profile_name_from_ipc_response(&state).as_deref(),
            Some("Chrome")
        );
    }

    #[cfg(windows)]
    #[test]
    fn tray_adds_control_token_only_to_mutating_ipc_commands() {
        let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let status = super::agent_ipc_request_with_token("get_status", Some(token.to_string()));
        let shutdown = super::agent_ipc_request_with_token("shutdown", Some(token.to_string()));

        assert_eq!(status["command"].as_str(), Some("get_status"));
        assert!(status.get("token").is_none());
        assert_eq!(shutdown["command"].as_str(), Some("shutdown"));
        assert_eq!(shutdown["token"].as_str(), Some(token));
    }

    #[test]
    fn tray_notification_respects_profile_preference() {
        let event = serde_json::json!({
            "timestamp_ms": 1,
            "kind": "profile_activated",
            "message": "Apex activo",
            "profile_name": "Apex Legends"
        });
        let enabled_config = serde_json::json!({
            "profiles": [{
                "name": "Apex Legends",
                "notifications": { "on_activate": true, "on_restore": true, "on_error": true }
            }]
        });
        let disabled_config = serde_json::json!({
            "profiles": [{
                "name": "Apex Legends",
                "notifications": { "on_activate": false, "on_restore": true, "on_error": true }
            }]
        });

        assert!(super::tray::should_notify_event(
            &event,
            Some(&enabled_config)
        ));
        assert!(!super::tray::should_notify_event(
            &event,
            Some(&disabled_config)
        ));
    }

    #[test]
    fn tray_notification_respects_global_preference() {
        let event = serde_json::json!({
            "timestamp_ms": 1,
            "kind": "profile_activated",
            "message": "Apex activo",
            "profile_name": "Apex Legends"
        });
        let config = serde_json::json!({
            "automation": { "notifications_enabled": false },
            "profiles": [{
                "name": "Apex Legends",
                "notifications": { "on_activate": true, "on_restore": true, "on_error": true }
            }]
        });

        assert!(!super::tray::should_notify_event(&event, Some(&config)));
    }

    #[test]
    fn tray_notification_respects_restore_preference_for_profile() {
        let event = serde_json::json!({
            "timestamp_ms": 1,
            "kind": "power_plan_restored",
            "message": "Plan restaurado",
            "profile_name": "Apex Legends"
        });
        let disabled_config = serde_json::json!({
            "profiles": [{
                "name": "Apex Legends",
                "notifications": { "on_activate": true, "on_restore": false, "on_error": true }
            }]
        });

        assert!(!super::tray::should_notify_event(
            &event,
            Some(&disabled_config)
        ));
    }

    #[test]
    fn tray_notification_uses_stable_event_key() {
        let event = serde_json::json!({
            "timestamp_ms": 99,
            "kind": "profile_activated",
            "message": "Chrome activo",
            "profile_name": "Chrome"
        });

        assert_eq!(
            super::tray::notification_key(&event).as_deref(),
            Some("99:profile_activated:Chrome activo")
        );
    }
}
