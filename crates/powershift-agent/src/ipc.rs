use crate::{AgentPaths, PublishedAgentState};
use powershift_windows::ProcessWatchMessage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{mpsc::Sender, Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AgentIpcRequest {
    GetStatus,
    Reevaluate {
        token: Option<String>,
    },
    SetStartup {
        enabled: bool,
        token: Option<String>,
    },
    ClearEvents {
        token: Option<String>,
    },
    Shutdown {
        token: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIpcResponse {
    pub ok: bool,
    pub state: Option<PublishedAgentState>,
    pub message: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct AgentSharedState {
    inner: Arc<Mutex<Option<PublishedAgentState>>>,
}

impl AgentSharedState {
    pub(crate) fn set(&self, state: PublishedAgentState) {
        if let Ok(mut value) = self.inner.lock() {
            *value = Some(state);
        }
    }

    pub(crate) fn get(&self) -> Option<PublishedAgentState> {
        self.inner.lock().ok().and_then(|value| value.clone())
    }
}

pub fn request_agent_status_via_ipc() -> Result<PublishedAgentState, String> {
    let response = call_agent_ipc(AgentIpcRequest::GetStatus)?;
    if response.ok {
        response
            .state
            .ok_or_else(|| "El agente respondio sin estado.".to_string())
    } else {
        Err(response
            .message
            .unwrap_or_else(|| "El agente no pudo entregar estado.".to_string()))
    }
}

pub fn request_agent_reevaluate_via_ipc() -> Result<(), String> {
    let response = call_agent_ipc(AgentIpcRequest::Reevaluate {
        token: read_control_token_from_app_data(),
    })?;
    if response.ok {
        Ok(())
    } else {
        Err(response
            .message
            .unwrap_or_else(|| "El agente no acepto revaluar.".to_string()))
    }
}

pub fn request_agent_shutdown_via_ipc() -> Result<(), String> {
    let response = call_agent_ipc(AgentIpcRequest::Shutdown {
        token: read_control_token_from_app_data(),
    })?;
    if response.ok {
        Ok(())
    } else {
        Err(response
            .message
            .unwrap_or_else(|| "El agente no acepto apagarse.".to_string()))
    }
}

pub fn request_agent_set_startup_via_ipc(enabled: bool) -> Result<(), String> {
    let response = call_agent_ipc(AgentIpcRequest::SetStartup {
        enabled,
        token: read_control_token_from_app_data(),
    })?;
    if response.ok {
        Ok(())
    } else {
        Err(response
            .message
            .unwrap_or_else(|| "El agente no pudo cambiar el inicio con Windows.".to_string()))
    }
}

pub fn request_agent_clear_events_via_ipc() -> Result<(), String> {
    let response = call_agent_ipc(AgentIpcRequest::ClearEvents {
        token: read_control_token_from_app_data(),
    })?;
    if response.ok {
        Ok(())
    } else {
        Err(response
            .message
            .unwrap_or_else(|| "El agente no pudo borrar los eventos.".to_string()))
    }
}

pub(crate) fn load_or_create_control_token(path: &Path) -> Result<String, String> {
    if let Ok(token) = read_control_token(path) {
        return Ok(token);
    }

    let token = generate_control_token()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    powershift_core::write_file_atomically(path, token.as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(token)
}

pub(crate) fn spawn_agent_ipc_server(
    sender: Sender<ProcessWatchMessage>,
    shared_state: AgentSharedState,
    control_token: String,
    event_log_path: PathBuf,
) {
    std::thread::spawn(move || {
        let pipe_name = powershift_windows::agent_pipe_name();
        let _ = powershift_windows::run_named_pipe_server(&pipe_name, move |request| {
            handle_agent_ipc_request(
                &request,
                &shared_state,
                &sender,
                &control_token,
                &event_log_path,
            )
        });
    });
}

fn call_agent_ipc(request: AgentIpcRequest) -> Result<AgentIpcResponse, String> {
    let request = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let pipe_name = powershift_windows::agent_pipe_name();
    let response = powershift_windows::call_named_pipe(&pipe_name, &request)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&response).map_err(|error| error.to_string())
}

fn read_control_token_from_app_data() -> Option<String> {
    let paths = AgentPaths::from_environment().ok()?;
    read_control_token(&paths.control_token()).ok()
}

fn read_control_token(path: &Path) -> Result<String, String> {
    let token = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let token = token.trim();
    if valid_control_token(token) {
        Ok(token.to_string())
    } else {
        Err("Token IPC de control invalido.".to_string())
    }
}

fn generate_control_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn valid_control_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn handle_agent_ipc_request(
    request: &str,
    shared_state: &AgentSharedState,
    sender: &Sender<ProcessWatchMessage>,
    control_token: &str,
    event_log_path: &Path,
) -> String {
    let response = match serde_json::from_str::<AgentIpcRequest>(request) {
        Ok(AgentIpcRequest::GetStatus) => match shared_state.get() {
            Some(state) => AgentIpcResponse {
                ok: true,
                state: Some(state),
                message: None,
            },
            None => AgentIpcResponse {
                ok: false,
                state: None,
                message: Some("Estado del agente aun no disponible.".to_string()),
            },
        },
        Ok(AgentIpcRequest::Reevaluate { token }) => {
            if !control_token_matches(token.as_deref(), control_token) {
                return agent_ipc_response_json(AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some("Token IPC de control invalido.".to_string()),
                });
            }
            match sender.send(ProcessWatchMessage::Reevaluate) {
                Ok(()) => AgentIpcResponse {
                    ok: true,
                    state: shared_state.get(),
                    message: Some("Reevaluacion solicitada.".to_string()),
                },
                Err(error) => AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some(error.to_string()),
                },
            }
        }
        Ok(AgentIpcRequest::SetStartup { enabled, token }) => {
            if !control_token_matches(token.as_deref(), control_token) {
                return agent_ipc_response_json(AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some("Token IPC de control invalido.".to_string()),
                });
            }
            match powershift_windows::set_agent_startup_trigger_enabled(enabled) {
                Ok(()) => AgentIpcResponse {
                    ok: true,
                    state: shared_state.get(),
                    message: Some(if enabled {
                        "Inicio con Windows habilitado.".to_string()
                    } else {
                        "Inicio con Windows deshabilitado.".to_string()
                    }),
                },
                Err(error) => AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some(error.to_string()),
                },
            }
        }
        Ok(AgentIpcRequest::ClearEvents { token }) => {
            if !control_token_matches(token.as_deref(), control_token) {
                return agent_ipc_response_json(AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some("Token IPC de control invalido.".to_string()),
                });
            }
            match crate::publisher::clear_event_history_at_path(event_log_path) {
                Ok(()) => AgentIpcResponse {
                    ok: true,
                    state: shared_state.get(),
                    message: Some("Historial de eventos borrado.".to_string()),
                },
                Err(error) => AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some(error),
                },
            }
        }
        Ok(AgentIpcRequest::Shutdown { token }) => {
            if !control_token_matches(token.as_deref(), control_token) {
                return agent_ipc_response_json(AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some("Token IPC de control invalido.".to_string()),
                });
            }
            match sender.send(ProcessWatchMessage::Shutdown) {
                Ok(()) => AgentIpcResponse {
                    ok: true,
                    state: shared_state.get(),
                    message: Some("Apagado solicitado.".to_string()),
                },
                Err(error) => AgentIpcResponse {
                    ok: false,
                    state: shared_state.get(),
                    message: Some(error.to_string()),
                },
            }
        }
        Err(error) => AgentIpcResponse {
            ok: false,
            state: shared_state.get(),
            message: Some(format!("Solicitud IPC invalida: {error}")),
        },
    };

    agent_ipc_response_json(response)
}

fn control_token_matches(request_token: Option<&str>, control_token: &str) -> bool {
    request_token.is_some_and(|token| token == control_token)
}

fn agent_ipc_response_json(response: AgentIpcResponse) -> String {
    serde_json::to_string(&response).unwrap_or_else(|error| {
        format!(
            r#"{{"ok":false,"state":null,"message":"No se pudo serializar respuesta IPC: {error}"}}"#
        )
    })
}
