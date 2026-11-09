use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::{AppError, ssh::TransportError};

pub const TERMINAL_EVENT: &str = "terminal-event";
pub const DEFAULT_TERMINAL_TERM: &str = "xterm-256color";
pub const MAX_TERMINAL_INPUT_BYTES: usize = 16 * 1024;
pub const MAX_TERMINAL_DIMENSION: u32 = 1_000;
pub const MAX_TERMINAL_PIXEL_DIMENSION: u32 = 10_000;

const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    pub cols: u32,
    pub rows: u32,
    #[serde(default)]
    pub pixel_width: u32,
    #[serde(default)]
    pub pixel_height: u32,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl TerminalSize {
    pub fn validate(self) -> Result<(), AppError> {
        if self.cols == 0
            || self.cols > MAX_TERMINAL_DIMENSION
            || self.rows == 0
            || self.rows > MAX_TERMINAL_DIMENSION
        {
            return Err(AppError::validation(format!(
                "Terminal dimensions must be between 1 and {MAX_TERMINAL_DIMENSION} columns and rows."
            )));
        }
        if self.pixel_width > MAX_TERMINAL_PIXEL_DIMENSION
            || self.pixel_height > MAX_TERMINAL_PIXEL_DIMENSION
        {
            return Err(AppError::validation(format!(
                "Terminal pixel dimensions must not exceed {MAX_TERMINAL_PIXEL_DIMENSION}."
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOptions {
    pub term: String,
    pub size: TerminalSize,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            term: DEFAULT_TERMINAL_TERM.to_string(),
            size: TerminalSize::default(),
        }
    }
}

impl TerminalOptions {
    pub fn new(term: Option<String>, size: Option<TerminalSize>) -> Result<Self, AppError> {
        let term = term.unwrap_or_else(|| DEFAULT_TERMINAL_TERM.to_string());
        let options = Self {
            term,
            size: size.unwrap_or_default(),
        };
        options.validate()?;
        Ok(options)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        let term = self.term.trim();
        if term.is_empty()
            || term.len() > 64
            || !term.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
        {
            return Err(AppError::validation(
                "The terminal type must be a short terminal name such as xterm-256color.",
            ));
        }
        self.size.validate()
    }
}

/// Events emitted by an interactive terminal channel.
///
/// `Data` is passed through byte-for-byte so ANSI control sequences and
/// colors reach the frontend unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalChannelEvent {
    Data(Vec<u8>),
    ExitStatus(u32),
    ExitSignal(String),
    Closed,
}

/// A single interactive PTY-backed SSH channel.
#[async_trait]
pub trait TerminalChannel: Send + Sync {
    /// Waits for the next channel message. `None` means the channel ended.
    async fn read(&self) -> Option<TerminalChannelEvent>;

    async fn write(&self, data: &[u8]) -> Result<(), TransportError>;

    async fn resize(&self, size: TerminalSize) -> Result<(), TransportError>;

    async fn close(&self) -> Result<(), TransportError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSessionInfo {
    pub session_id: String,
    pub server_id: String,
    pub term: String,
    pub cols: u32,
    pub rows: u32,
    pub opened_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalExitReason {
    RemoteExit,
    Closed,
    Disconnected,
    Error,
}

/// Typed payload for the `terminal-event` Tauri event.
///
/// Terminal output is base64 encoded so arbitrary bytes and ANSI escapes are
/// preserved by JSON. Debug output intentionally omits terminal bytes: events
/// and input must never leak into application logs.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalEvent {
    Output {
        session_id: String,
        sequence: u64,
        data: String,
    },
    Exited {
        session_id: String,
        sequence: u64,
        reason: TerminalExitReason,
        exit_code: Option<u32>,
        message: Option<String>,
    },
}

impl fmt::Debug for TerminalEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Output {
                session_id,
                sequence,
                ..
            } => formatter
                .debug_struct("Output")
                .field("session_id", session_id)
                .field("sequence", sequence)
                .field("data", &"<redacted>")
                .finish(),
            Self::Exited {
                session_id,
                sequence,
                reason,
                exit_code,
                message,
            } => formatter
                .debug_struct("Exited")
                .field("session_id", session_id)
                .field("sequence", sequence)
                .field("reason", reason)
                .field("exit_code", exit_code)
                .field("message", message)
                .finish(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInput {
    pub session_id: String,
    pub data: String,
}

impl fmt::Debug for TerminalInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalInput")
            .field("session_id", &self.session_id)
            .field("data", &"<redacted>")
            .finish()
    }
}

pub trait TerminalEventSink: Send + Sync {
    fn emit(&self, event: &TerminalEvent);
}

struct TerminalEntry {
    info: TerminalSessionInfo,
    channel: Arc<dyn TerminalChannel>,
    sequence: AtomicU64,
    closing: AtomicBool,
    exited: AtomicBool,
}

impl TerminalEntry {
    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }
}

pub struct TerminalManager {
    sessions: Mutex<HashMap<String, Arc<TerminalEntry>>>,
    events: Arc<dyn TerminalEventSink>,
}

impl TerminalManager {
    pub fn new(events: Arc<dyn TerminalEventSink>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            events,
        }
    }

    /// Registers an already-opened PTY channel and starts forwarding its output.
    ///
    /// The channel is owned by the manager from this point on. If the caller
    /// cannot register the session, it must close the channel itself.
    pub fn open(
        self: &Arc<Self>,
        server_id: &str,
        options: TerminalOptions,
        channel: Arc<dyn TerminalChannel>,
    ) -> Result<TerminalSessionInfo, AppError> {
        if server_id.trim().is_empty() {
            return Err(AppError::validation("Server ID is required."));
        }
        options.validate()?;

        let info = TerminalSessionInfo {
            session_id: Uuid::new_v4().to_string(),
            server_id: server_id.to_string(),
            term: options.term,
            cols: options.size.cols,
            rows: options.size.rows,
            opened_at: now_millis(),
        };
        let entry = Arc::new(TerminalEntry {
            info: info.clone(),
            channel,
            sequence: AtomicU64::new(0),
            closing: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });

        self.lock()?
            .insert(info.session_id.clone(), Arc::clone(&entry));
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            manager.forward_output(entry).await;
        });

        Ok(info)
    }

    pub fn list(&self) -> Result<Vec<TerminalSessionInfo>, AppError> {
        let mut sessions = self
            .lock()?
            .values()
            .map(|entry| entry.info.clone())
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.opened_at);
        Ok(sessions)
    }

    pub async fn write(&self, session_id: &str, data: &[u8]) -> Result<(), AppError> {
        if data.is_empty() {
            return Ok(());
        }
        if data.len() > MAX_TERMINAL_INPUT_BYTES {
            return Err(AppError::validation(format!(
                "Terminal input chunks must not exceed {MAX_TERMINAL_INPUT_BYTES} bytes."
            )));
        }

        let entry = self.entry(session_id)?;
        match tokio::time::timeout(WRITE_TIMEOUT, entry.channel.write(data)).await {
            Err(_) => Err(AppError::timeout(
                "The terminal input could not be sent before the write timed out.",
            )),
            Ok(Err(error)) => Err(error.to_app_error()),
            Ok(Ok(())) => Ok(()),
        }
    }

    pub async fn resize(&self, session_id: &str, size: TerminalSize) -> Result<(), AppError> {
        size.validate()?;
        let entry = self.entry(session_id)?;
        match tokio::time::timeout(WRITE_TIMEOUT, entry.channel.resize(size)).await {
            Err(_) => Err(AppError::timeout(
                "The terminal resize event could not be sent before the write timed out.",
            )),
            Ok(Err(error)) => Err(error.to_app_error()),
            Ok(Ok(())) => Ok(()),
        }
    }

    /// Closes one terminal. Closing an unknown or already-closed session is a
    /// no-op so repeated UI cleanup cannot fail.
    pub async fn close(&self, session_id: &str) -> Result<(), AppError> {
        let entry = self.lock()?.remove(session_id);
        if let Some(entry) = entry {
            self.close_entry(entry, TerminalExitReason::Closed, None, None)
                .await;
        }
        Ok(())
    }

    pub async fn close_server(&self, server_id: &str) {
        let entries = {
            let mut sessions = match self.sessions.lock() {
                Ok(sessions) => sessions,
                Err(_) => return,
            };
            let ids = sessions
                .values()
                .filter(|entry| entry.info.server_id == server_id)
                .map(|entry| entry.info.session_id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| sessions.remove(&id))
                .collect::<Vec<_>>()
        };

        for entry in entries {
            self.close_entry(entry, TerminalExitReason::Closed, None, None)
                .await;
        }
    }

    pub async fn close_all(&self) {
        let entries = {
            let mut sessions = match self.sessions.lock() {
                Ok(sessions) => sessions,
                Err(_) => return,
            };
            std::mem::take(&mut *sessions)
                .into_values()
                .collect::<Vec<_>>()
        };

        for entry in entries {
            self.close_entry(entry, TerminalExitReason::Closed, None, None)
                .await;
        }
    }

    fn entry(&self, session_id: &str) -> Result<Arc<TerminalEntry>, AppError> {
        self.lock()?
            .get(session_id)
            .filter(|entry| !entry.exited.load(Ordering::SeqCst))
            .cloned()
            .ok_or_else(|| AppError::not_found("The terminal session is not open."))
    }

    async fn forward_output(&self, entry: Arc<TerminalEntry>) {
        let mut reason = TerminalExitReason::Disconnected;
        let mut exit_code = None;
        let mut message = None;

        loop {
            if entry.closing.load(Ordering::SeqCst) {
                reason = TerminalExitReason::Closed;
                break;
            }

            match entry.channel.read().await {
                Some(TerminalChannelEvent::Data(data)) => {
                    if !data.is_empty() {
                        let event = TerminalEvent::Output {
                            session_id: entry.info.session_id.clone(),
                            sequence: entry.next_sequence(),
                            data: STANDARD.encode(data),
                        };
                        self.events.emit(&event);
                    }
                }
                Some(TerminalChannelEvent::ExitStatus(code)) => {
                    reason = TerminalExitReason::RemoteExit;
                    exit_code = Some(code);
                    break;
                }
                Some(TerminalChannelEvent::ExitSignal(signal)) => {
                    reason = TerminalExitReason::RemoteExit;
                    message = Some(format!("The remote process terminated by signal {signal}."));
                    break;
                }
                Some(TerminalChannelEvent::Closed) | None => break,
            }
        }

        if entry.closing.load(Ordering::SeqCst) {
            reason = TerminalExitReason::Closed;
        }

        let _ = self
            .lock()
            .map(|mut sessions| sessions.remove(&entry.info.session_id));
        self.emit_exit(&entry, reason, exit_code, message);
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, entry.channel.close()).await;
    }

    async fn close_entry(
        &self,
        entry: Arc<TerminalEntry>,
        reason: TerminalExitReason,
        exit_code: Option<u32>,
        message: Option<String>,
    ) {
        entry.closing.store(true, Ordering::SeqCst);
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, entry.channel.close()).await;
        self.emit_exit(&entry, reason, exit_code, message);
    }

    fn emit_exit(
        &self,
        entry: &TerminalEntry,
        reason: TerminalExitReason,
        exit_code: Option<u32>,
        message: Option<String>,
    ) {
        if entry.exited.swap(true, Ordering::SeqCst) {
            return;
        }
        self.events.emit(&TerminalEvent::Exited {
            session_id: entry.info.session_id.clone(),
            sequence: entry.next_sequence(),
            reason,
            exit_code,
            message,
        });
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, Arc<TerminalEntry>>>, AppError> {
        self.sessions
            .lock()
            .map_err(|_| AppError::internal("Terminal manager lock was poisoned."))
    }
}

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use tokio::sync::{Notify, mpsc};

    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<TerminalEvent>>,
        notify: Notify,
    }

    impl RecordingSink {
        fn events(&self) -> Vec<TerminalEvent> {
            self.events.lock().expect("event lock").clone()
        }

        async fn wait_for<F>(&self, predicate: F) -> TerminalEvent
        where
            F: Fn(&TerminalEvent) -> bool,
        {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let notified = self.notify.notified();
                    if let Some(event) = self
                        .events
                        .lock()
                        .expect("event lock")
                        .iter()
                        .find(|event| predicate(event))
                        .cloned()
                    {
                        return event;
                    }
                    notified.await;
                }
            })
            .await
            .expect("terminal event")
        }

        async fn wait_for_exit(&self, session_id: &str) -> TerminalEvent {
            self.wait_for(|event| {
                matches!(
                    event,
                    TerminalEvent::Exited { session_id: id, .. } if id == session_id
                )
            })
            .await
        }
    }

    impl TerminalEventSink for RecordingSink {
        fn emit(&self, event: &TerminalEvent) {
            self.events.lock().expect("event lock").push(event.clone());
            self.notify.notify_waiters();
        }
    }

    struct MockChannel {
        outputs: tokio::sync::Mutex<mpsc::Receiver<TerminalChannelEvent>>,
        output_sender: mpsc::Sender<TerminalChannelEvent>,
        inputs: Mutex<Vec<Vec<u8>>>,
        resizes: Mutex<Vec<TerminalSize>>,
        close_count: AtomicUsize,
        fail_write: AtomicBool,
    }

    impl MockChannel {
        fn new() -> Arc<Self> {
            let (output_sender, outputs) = mpsc::channel(16);
            Arc::new(Self {
                outputs: tokio::sync::Mutex::new(outputs),
                output_sender,
                inputs: Mutex::new(Vec::new()),
                resizes: Mutex::new(Vec::new()),
                close_count: AtomicUsize::new(0),
                fail_write: AtomicBool::new(false),
            })
        }

        fn send(&self, event: TerminalChannelEvent) {
            self.output_sender
                .try_send(event)
                .expect("mock terminal channel output");
        }

        fn inputs(&self) -> Vec<Vec<u8>> {
            self.inputs.lock().expect("input lock").clone()
        }

        fn resizes(&self) -> Vec<TerminalSize> {
            self.resizes.lock().expect("resize lock").clone()
        }

        fn close_count(&self) -> usize {
            self.close_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl TerminalChannel for MockChannel {
        async fn read(&self) -> Option<TerminalChannelEvent> {
            self.outputs.lock().await.recv().await
        }

        async fn write(&self, data: &[u8]) -> Result<(), TransportError> {
            if self.fail_write.load(Ordering::SeqCst) {
                return Err(TransportError::Network);
            }
            self.inputs.lock().expect("input lock").push(data.to_vec());
            Ok(())
        }

        async fn resize(&self, size: TerminalSize) -> Result<(), TransportError> {
            self.resizes.lock().expect("resize lock").push(size);
            Ok(())
        }

        async fn close(&self) -> Result<(), TransportError> {
            self.close_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn manager_with_sink() -> (Arc<TerminalManager>, Arc<RecordingSink>) {
        let sink = Arc::new(RecordingSink::default());
        let manager = Arc::new(TerminalManager::new(
            Arc::clone(&sink) as Arc<dyn TerminalEventSink>
        ));
        (manager, sink)
    }

    fn options() -> TerminalOptions {
        TerminalOptions {
            term: "xterm-256color".to_string(),
            size: TerminalSize {
                cols: 100,
                rows: 40,
                pixel_width: 0,
                pixel_height: 0,
            },
        }
    }

    #[tokio::test]
    async fn streams_output_as_base64_with_ansi_preserved() {
        let (manager, sink) = manager_with_sink();
        let channel = MockChannel::new();
        let info = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&channel) as Arc<dyn TerminalChannel>,
            )
            .expect("open terminal");

        channel.send(TerminalChannelEvent::Data(
            b"\x1b[32mgreen\x1b[0m\r\n".to_vec(),
        ));

        let event = sink
            .wait_for(|event| matches!(event, TerminalEvent::Output { .. }))
            .await;
        let TerminalEvent::Output { data, sequence, .. } = event else {
            panic!("expected output event");
        };
        assert_eq!(sequence, 1);
        assert_eq!(
            STANDARD.decode(data).expect("base64 output"),
            b"\x1b[32mgreen\x1b[0m\r\n"
        );
        assert_eq!(info.cols, 100);
        assert_eq!(info.rows, 40);
        assert_eq!(manager.list().expect("list").len(), 1);
    }

    #[tokio::test]
    async fn routes_input_and_resize_without_emitting_input_events() {
        let (manager, sink) = manager_with_sink();
        let channel = MockChannel::new();
        let info = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&channel) as Arc<dyn TerminalChannel>,
            )
            .expect("open terminal");

        let secret = b"sudo secret-password\r";
        manager
            .write(&info.session_id, secret)
            .await
            .expect("write");
        let size = TerminalSize {
            cols: 132,
            rows: 43,
            pixel_width: 1_320,
            pixel_height: 860,
        };
        manager
            .resize(&info.session_id, size)
            .await
            .expect("resize");

        assert_eq!(channel.inputs(), vec![secret.to_vec()]);
        assert_eq!(channel.resizes(), vec![size]);

        tokio::time::sleep(Duration::from_millis(10)).await;
        let serialized = format!("{:?}", sink.events());
        assert!(!serialized.contains("secret-password"));
        assert!(
            sink.events()
                .iter()
                .all(|event| !matches!(event, TerminalEvent::Output { .. }))
        );
    }

    #[tokio::test]
    async fn rejects_oversized_input_and_invalid_dimensions() {
        let (manager, _sink) = manager_with_sink();
        let channel = MockChannel::new();
        let info = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&channel) as Arc<dyn TerminalChannel>,
            )
            .expect("open terminal");

        let error = manager
            .write(&info.session_id, &vec![b'x'; MAX_TERMINAL_INPUT_BYTES + 1])
            .await
            .expect_err("oversized input");
        assert_eq!(error.code, crate::core::ErrorCode::Validation);
        assert!(channel.inputs().is_empty());

        let error = manager
            .resize(
                &info.session_id,
                TerminalSize {
                    cols: 0,
                    ..TerminalSize::default()
                },
            )
            .await
            .expect_err("invalid dimensions");
        assert_eq!(error.code, crate::core::ErrorCode::Validation);

        assert!(
            TerminalOptions::new(Some("xterm; rm -rf /".to_string()), None).is_err(),
            "terminal names must be validated"
        );
    }

    #[tokio::test]
    async fn remote_exit_emits_status_and_removes_the_session() {
        let (manager, sink) = manager_with_sink();
        let channel = MockChannel::new();
        let info = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&channel) as Arc<dyn TerminalChannel>,
            )
            .expect("open terminal");

        channel.send(TerminalChannelEvent::ExitStatus(7));

        let TerminalEvent::Exited {
            reason, exit_code, ..
        } = sink.wait_for_exit(&info.session_id).await
        else {
            panic!("expected exit event");
        };
        assert_eq!(reason, TerminalExitReason::RemoteExit);
        assert_eq!(exit_code, Some(7));
        assert!(manager.list().expect("list").is_empty());
        assert_eq!(channel.close_count(), 1);
    }

    #[tokio::test]
    async fn close_releases_only_the_requested_channel() {
        let (manager, sink) = manager_with_sink();
        let first_channel = MockChannel::new();
        let second_channel = MockChannel::new();
        let first = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&first_channel) as Arc<dyn TerminalChannel>,
            )
            .expect("first terminal");
        let second = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&second_channel) as Arc<dyn TerminalChannel>,
            )
            .expect("second terminal");

        manager.close(&first.session_id).await.expect("close");
        manager
            .close(&first.session_id)
            .await
            .expect("idempotent close");

        let TerminalEvent::Exited { reason, .. } = sink.wait_for_exit(&first.session_id).await
        else {
            panic!("expected exit event");
        };
        assert_eq!(reason, TerminalExitReason::Closed);
        assert_eq!(first_channel.close_count(), 1);
        assert_eq!(second_channel.close_count(), 0);

        let sessions = manager.list().expect("list");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, second.session_id);

        manager.close_all().await;
        assert!(manager.list().expect("list").is_empty());
        assert_eq!(second_channel.close_count(), 1);
    }

    #[tokio::test]
    async fn close_server_leaves_other_servers_running() {
        let (manager, sink) = manager_with_sink();
        let first_channel = MockChannel::new();
        let second_channel = MockChannel::new();
        let first = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&first_channel) as Arc<dyn TerminalChannel>,
            )
            .expect("first terminal");
        let second = manager
            .open(
                "server-2",
                options(),
                Arc::clone(&second_channel) as Arc<dyn TerminalChannel>,
            )
            .expect("second terminal");

        manager.close_server("server-1").await;

        sink.wait_for_exit(&first.session_id).await;
        assert_eq!(first_channel.close_count(), 1);
        assert_eq!(second_channel.close_count(), 0);
        assert_eq!(manager.list().expect("list").len(), 1);
        assert_eq!(
            manager.list().expect("list")[0].session_id,
            second.session_id
        );

        manager
            .close(&second.session_id)
            .await
            .expect("close second");
    }

    #[tokio::test]
    async fn a_closed_channel_reports_disconnect_and_cleans_up() {
        let (manager, sink) = manager_with_sink();
        let channel = MockChannel::new();
        let info = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&channel) as Arc<dyn TerminalChannel>,
            )
            .expect("open terminal");

        channel.send(TerminalChannelEvent::Closed);

        let TerminalEvent::Exited { reason, .. } = sink.wait_for_exit(&info.session_id).await
        else {
            panic!("expected exit event");
        };
        assert_eq!(reason, TerminalExitReason::Disconnected);
        assert!(manager.list().expect("list").is_empty());
        assert_eq!(channel.close_count(), 1);
    }

    #[tokio::test]
    async fn write_errors_are_typed_and_sessions_can_be_closed_after_remote_exit() {
        let (manager, sink) = manager_with_sink();
        let channel = MockChannel::new();
        let info = manager
            .open(
                "server-1",
                options(),
                Arc::clone(&channel) as Arc<dyn TerminalChannel>,
            )
            .expect("open terminal");
        channel.fail_write.store(true, Ordering::SeqCst);

        let error = manager
            .write(&info.session_id, b"x")
            .await
            .expect_err("write failure");
        assert_eq!(error.code, crate::core::ErrorCode::Network);

        channel.send(TerminalChannelEvent::ExitStatus(0));
        sink.wait_for_exit(&info.session_id).await;
        assert!(manager.write(&info.session_id, b"x").await.is_err());
        manager.close(&info.session_id).await.expect("close exited");
    }

    #[test]
    fn debug_output_redacts_terminal_bytes() {
        let input = TerminalInput {
            session_id: "terminal-1".to_string(),
            data: "secret keystrokes".to_string(),
        };
        let event = TerminalEvent::Output {
            session_id: "terminal-1".to_string(),
            sequence: 1,
            data: STANDARD.encode(b"secret keystrokes"),
        };

        let input_debug = format!("{input:?}");
        let event_debug = format!("{event:?}");
        assert!(input_debug.contains("<redacted>"));
        assert!(!input_debug.contains("secret keystrokes"));
        assert!(event_debug.contains("<redacted>"));
        assert!(!event_debug.contains("secret keystrokes"));
        assert!(!event_debug.contains(&STANDARD.encode(b"secret keystrokes")));
    }
}
