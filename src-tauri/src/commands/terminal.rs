use std::sync::Arc;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::core::{
    AppError,
    ssh::ConnectionManager,
    terminal::{
        TERMINAL_EVENT, TerminalEvent, TerminalEventSink, TerminalInput, TerminalManager,
        TerminalOptions, TerminalSessionInfo, TerminalSize,
    },
};

pub struct TauriTerminalEventSink {
    app: AppHandle,
}

impl TauriTerminalEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl TerminalEventSink for TauriTerminalEventSink {
    fn emit(&self, event: &TerminalEvent) {
        let _ = self.app.emit(TERMINAL_EVENT, event.clone());
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTerminalInput {
    pub server_id: String,
    pub term: Option<String>,
    pub size: Option<TerminalSize>,
}

#[tauri::command]
pub async fn open_terminal(
    connections: State<'_, Arc<ConnectionManager>>,
    terminal: State<'_, Arc<TerminalManager>>,
    input: OpenTerminalInput,
) -> Result<TerminalSessionInfo, AppError> {
    if input.server_id.trim().is_empty() {
        return Err(AppError::validation("Server ID is required."));
    }
    let options = TerminalOptions::new(input.term, input.size)?;
    let channel = connections
        .open_terminal(&input.server_id, options.clone())
        .await?;

    match terminal.open(&input.server_id, options, channel) {
        Ok(info) => Ok(info),
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn terminal_input(
    terminal: State<'_, Arc<TerminalManager>>,
    input: TerminalInput,
) -> Result<(), AppError> {
    terminal
        .write(&input.session_id, input.data.as_bytes())
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeTerminalInput {
    pub session_id: String,
    pub size: TerminalSize,
}

#[tauri::command]
pub async fn resize_terminal(
    terminal: State<'_, Arc<TerminalManager>>,
    input: ResizeTerminalInput,
) -> Result<(), AppError> {
    terminal.resize(&input.session_id, input.size).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTerminalInput {
    pub session_id: String,
}

#[tauri::command]
pub async fn close_terminal(
    terminal: State<'_, Arc<TerminalManager>>,
    input: CloseTerminalInput,
) -> Result<(), AppError> {
    terminal.close(&input.session_id).await
}

#[tauri::command]
pub fn list_terminal_sessions(
    terminal: State<'_, Arc<TerminalManager>>,
) -> Result<Vec<TerminalSessionInfo>, AppError> {
    terminal.list()
}
