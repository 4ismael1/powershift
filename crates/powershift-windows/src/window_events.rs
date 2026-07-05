use crate::process_events::{ProcessEvent, ProcessEventKind, ProcessWatchMessage};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

pub fn spawn_window_activity_watcher(sender: Sender<ProcessWatchMessage>) -> JoinHandle<()> {
    spawn_platform_window_activity_watcher(sender)
}

#[cfg(windows)]
fn spawn_platform_window_activity_watcher(sender: Sender<ProcessWatchMessage>) -> JoinHandle<()> {
    std::thread::spawn(move || watch_window_activity(sender))
}

#[cfg(not(windows))]
fn spawn_platform_window_activity_watcher(_sender: Sender<ProcessWatchMessage>) -> JoinHandle<()> {
    std::thread::spawn(|| std::thread::park())
}

#[cfg(windows)]
fn watch_window_activity(sender: Sender<ProcessWatchMessage>) {
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::Foundation::{HMODULE, HWND};
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, GetWindowThreadProcessId, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY,
        EVENT_SYSTEM_FOREGROUND, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    };

    static WINDOW_EVENT_SENDER: OnceLock<Mutex<Option<Sender<ProcessWatchMessage>>>> =
        OnceLock::new();

    unsafe extern "system" fn callback(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        object_id: i32,
        _child_id: i32,
        _event_thread: u32,
        _event_time: u32,
    ) {
        if object_id != OBJID_WINDOW.0 || hwnd.0.is_null() {
            return;
        }

        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid == 0 {
            return;
        }

        let Some(kind) = event_kind_from_window_event(event) else {
            return;
        };
        let event = ProcessWatchMessage::Event(ProcessEvent {
            kind,
            pid,
            name: String::new(),
            path: None,
        });

        if let Some(sender) = WINDOW_EVENT_SENDER
            .get()
            .and_then(|sender| sender.lock().ok())
            .and_then(|sender| sender.clone())
        {
            let _ = sender.send(event);
        }
    }

    *WINDOW_EVENT_SENDER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("window event sender lock") = Some(sender.clone());

    let foreground_hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            Some(HMODULE::default()),
            Some(callback),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    let object_hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_DESTROY,
            Some(HMODULE::default()),
            Some(callback),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };

    if foreground_hook.0.is_null() && object_hook.0.is_null() {
        let _ = sender.send(ProcessWatchMessage::Error(
            "No se pudo iniciar el watcher de ventanas".to_string(),
        ));
        return;
    }

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
    }
}

#[cfg(windows)]
fn event_kind_from_window_event(event: u32) -> Option<ProcessEventKind> {
    use windows::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_SYSTEM_FOREGROUND,
    };

    match event {
        EVENT_OBJECT_DESTROY => Some(ProcessEventKind::Stopped),
        EVENT_OBJECT_CREATE | EVENT_SYSTEM_FOREGROUND => Some(ProcessEventKind::Started),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn maps_window_events_to_process_event_kinds() {
        use windows::Win32::UI::WindowsAndMessaging::{
            EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_SYSTEM_FOREGROUND,
        };

        assert_eq!(
            event_kind_from_window_event(EVENT_OBJECT_CREATE),
            Some(ProcessEventKind::Started)
        );
        assert_eq!(
            event_kind_from_window_event(EVENT_SYSTEM_FOREGROUND),
            Some(ProcessEventKind::Started)
        );
        assert_eq!(
            event_kind_from_window_event(EVENT_OBJECT_DESTROY),
            Some(ProcessEventKind::Stopped)
        );
        assert_eq!(event_kind_from_window_event(0), None);
    }
}
