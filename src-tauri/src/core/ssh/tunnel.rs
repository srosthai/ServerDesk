//! Loopback-only SSH local forwarding for database profiles.
//!
//! A tunnel binds `127.0.0.1` on an ephemeral port and opens one
//! `direct-tcpip` channel through the already authenticated SSH session for
//! each local client. Nothing is bound to a public or wildcard address, and
//! forwarded bytes are never logged.

use std::{
    collections::HashMap,
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    task::{JoinHandle, JoinSet},
};

use crate::core::AppError;

use super::{
    ConnectionEventSink, ConnectionManager, ConnectionSnapshot, ConnectionState, SshSession,
    TransportError, types::now_millis,
};

/// Default upper bound for simultaneously active database tunnels.
pub const DEFAULT_MAX_TUNNELS: usize = 16;

/// Upper bound for forwarded client connections inside one tunnel.
pub const MAX_FORWARDED_CONNECTIONS: usize = 8;

const LOOPBACK_HOST: &str = "127.0.0.1";
const MAX_REMOTE_HOST_LENGTH: usize = 255;
const MAX_SOCKET_PATH_LENGTH: usize = 4096;
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(50);
const SESSION_HEALTH_INTERVAL: Duration = Duration::from_secs(1);

/// Observable lifecycle of one database tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TunnelState {
    Opening,
    Active,
    Closed,
}

/// One request to forward a database profile through its SSH server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelRequest {
    pub profile_id: String,
    pub server_id: String,
    pub remote_host: String,
    pub remote_port: u16,
    /// Optional Unix socket metadata from the saved database profile.
    ///
    /// The local listener still forwards to `remote_host`/`remote_port`; the
    /// socket path is carried as profile context for the database adapter and
    /// never used as a local bind address.
    pub socket: Option<String>,
}

/// Typed state returned to the frontend for one database profile.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelSnapshot {
    pub profile_id: String,
    pub server_id: String,
    pub state: TunnelState,
    pub local_host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_port: Option<u16>,
    pub remote_host: String,
    pub remote_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    pub opened_at: i64,
}

/// Time bounds for opening and closing a forwarded channel and listener.
#[derive(Debug, Clone)]
pub struct TunnelTimeouts {
    pub open: Duration,
    pub close: Duration,
}

impl Default for TunnelTimeouts {
    fn default() -> Self {
        Self {
            open: Duration::from_secs(10),
            close: Duration::from_secs(5),
        }
    }
}

/// Creates the loopback listener used by a database tunnel.
///
/// Implementations must bind a loopback address. The default implementation
/// always binds `127.0.0.1` with an ephemeral port and cannot produce a
/// wildcard listener.
pub trait TunnelListenerFactory: Send + Sync {
    fn bind_loopback(&self) -> io::Result<StdTcpListener>;
}

/// Default listener factory: loopback IPv4 with an OS-assigned ephemeral port.
pub struct LoopbackTunnelListenerFactory;

impl TunnelListenerFactory for LoopbackTunnelListenerFactory {
    fn bind_loopback(&self) -> io::Result<StdTcpListener> {
        StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TunnelTarget {
    host: String,
    port: u16,
    socket: Option<String>,
}

type SharedListener = Arc<Mutex<Option<TcpListener>>>;

enum TunnelEntry {
    Opening {
        server_id: String,
        target: TunnelTarget,
        opened_at: i64,
        generation: u64,
        listener: SharedListener,
        _permit: OwnedSemaphorePermit,
    },
    Active {
        server_id: String,
        target: TunnelTarget,
        local_port: u16,
        opened_at: i64,
        generation: u64,
        shutdown: watch::Sender<bool>,
        task: JoinHandle<()>,
        _permit: OwnedSemaphorePermit,
    },
}

impl TunnelEntry {
    fn server_id(&self) -> &str {
        match self {
            Self::Opening { server_id, .. } | Self::Active { server_id, .. } => server_id,
        }
    }

    fn target(&self) -> &TunnelTarget {
        match self {
            Self::Opening { target, .. } | Self::Active { target, .. } => target,
        }
    }

    fn generation(&self) -> u64 {
        match self {
            Self::Opening { generation, .. } | Self::Active { generation, .. } => *generation,
        }
    }

    fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    fn into_permit(self) -> OwnedSemaphorePermit {
        match self {
            Self::Opening { _permit, .. } | Self::Active { _permit, .. } => _permit,
        }
    }

    fn snapshot(&self, profile_id: &str) -> TunnelSnapshot {
        match self {
            Self::Opening {
                server_id,
                target,
                opened_at,
                ..
            } => TunnelSnapshot {
                profile_id: profile_id.to_string(),
                server_id: server_id.clone(),
                state: TunnelState::Opening,
                local_host: LOOPBACK_HOST.to_string(),
                local_port: None,
                remote_host: target.host.clone(),
                remote_port: target.port,
                socket: target.socket.clone(),
                opened_at: *opened_at,
            },
            Self::Active {
                server_id,
                target,
                local_port,
                opened_at,
                ..
            } => TunnelSnapshot {
                profile_id: profile_id.to_string(),
                server_id: server_id.clone(),
                state: TunnelState::Active,
                local_host: LOOPBACK_HOST.to_string(),
                local_port: Some(*local_port),
                remote_host: target.host.clone(),
                remote_port: target.port,
                socket: target.socket.clone(),
                opened_at: *opened_at,
            },
        }
    }
}

#[derive(Default)]
struct TunnelInner {
    tunnels: HashMap<String, TunnelEntry>,
}

/// Owns database tunnel listeners and their forwarded SSH channels.
///
/// Tunnels are keyed by database profile. A duplicate request for the same
/// profile and target reuses the active tunnel; a conflicting target, a request
/// while the first is still opening, or a request past the concurrency bound is
/// rejected with a deterministic `conflict` error.
pub struct TunnelManager {
    connections: Arc<ConnectionManager>,
    listener_factory: Arc<dyn TunnelListenerFactory>,
    timeouts: TunnelTimeouts,
    slots: Arc<Semaphore>,
    inner: Mutex<TunnelInner>,
    next_generation: AtomicU64,
}

impl TunnelManager {
    pub fn new(connections: Arc<ConnectionManager>) -> Arc<Self> {
        Self::with_options(
            connections,
            Arc::new(LoopbackTunnelListenerFactory),
            TunnelTimeouts::default(),
            DEFAULT_MAX_TUNNELS,
        )
    }

    /// Builds a manager with explicit limits. The bound is clamped to at least
    /// one tunnel so the manager can never launch unbounded listeners.
    pub fn with_options(
        connections: Arc<ConnectionManager>,
        listener_factory: Arc<dyn TunnelListenerFactory>,
        timeouts: TunnelTimeouts,
        max_tunnels: usize,
    ) -> Arc<Self> {
        let manager = Arc::new(Self {
            connections: Arc::clone(&connections),
            listener_factory,
            timeouts,
            slots: Arc::new(Semaphore::new(max_tunnels.max(1))),
            inner: Mutex::new(TunnelInner::default()),
            next_generation: AtomicU64::new(0),
        });
        connections.subscribe(Arc::new(TunnelConnectionEventSink {
            manager: Arc::downgrade(&manager),
        }));
        manager
    }

    /// Opens or reuses the tunnel for one database profile.
    pub async fn open(&self, request: TunnelRequest) -> Result<TunnelSnapshot, AppError> {
        let (request, target) = validate_request(request)?;
        let profile_id = request.profile_id;
        let server_id = request.server_id;

        // A tunnel can only exist on an authenticated session. This surfaces
        // host-key, authentication, and missing-session failures as their
        // existing typed errors before any database client is started.
        let session = self.connections.session(&server_id)?;
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let listener: SharedListener = Arc::new(Mutex::new(None));

        {
            let mut inner = self.lock()?;
            if let Some(existing) = inner.tunnels.get(&profile_id) {
                if existing.is_active() && existing.target() == &target {
                    return Ok(existing.snapshot(&profile_id));
                }

                let message = if existing.is_active() {
                    "A database tunnel is already active for this profile with a different remote endpoint."
                } else {
                    "A database tunnel for this profile is already being opened."
                };
                return Err(AppError::conflict(message).with_details(
                    "Close the existing tunnel before opening a different one for this profile.",
                ));
            }

            let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
                AppError::conflict("The maximum number of database tunnels is already open.")
            })?;
            inner.tunnels.insert(
                profile_id.clone(),
                TunnelEntry::Opening {
                    server_id: server_id.clone(),
                    target: target.clone(),
                    opened_at: now_millis(),
                    generation,
                    listener: Arc::clone(&listener),
                    _permit: permit,
                },
            );
        }

        let local_listener = match self.bind_loopback() {
            Ok(listener) => listener,
            Err(error) => {
                self.remove_opening(&profile_id, generation);
                return Err(error);
            }
        };
        let local_port = match local_listener.local_addr() {
            Ok(address) => address.port(),
            Err(_) => {
                self.remove_opening(&profile_id, generation);
                return Err(loopback_error());
            }
        };

        if let Ok(mut shared) = listener.lock() {
            *shared = Some(local_listener);
        } else {
            self.remove_opening(&profile_id, generation);
            return Err(AppError::internal(
                "The database tunnel listener lock was poisoned.",
            ));
        }

        // Confirm the remote endpoint is reachable through this SSH session
        // before publishing the local port. The probe channel is closed
        // immediately; the listener opens a fresh channel per client.
        let probe = tokio::time::timeout(
            self.timeouts.open,
            session.open_direct_tcpip(&target.host, target.port),
        )
        .await;
        match probe {
            Err(_) => {
                self.discard_opening(&profile_id, generation, &listener);
                return Err(AppError::timeout(format!(
                    "The database tunnel could not be opened within {} seconds.",
                    self.timeouts.open.as_secs().max(1)
                )));
            }
            Ok(Err(error)) => {
                self.discard_opening(&profile_id, generation, &listener);
                return Err(map_open_error(error));
            }
            Ok(Ok(channel)) => drop(channel),
        }

        let Some(local_listener) = take_listener(&listener) else {
            self.remove_opening(&profile_id, generation);
            return Err(AppError::cancelled(
                "The database tunnel was closed while it was opening.",
            ));
        };

        let (shutdown, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_tunnel_listener(
            local_listener,
            Arc::clone(&session),
            target.host.clone(),
            target.port,
            shutdown_rx,
            self.timeouts.open,
        ));

        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => {
                let _ = shutdown.send(true);
                task.abort();
                return Err(AppError::internal(
                    "The database tunnel manager lock was poisoned.",
                ));
            }
        };
        let is_current = inner
            .tunnels
            .get(&profile_id)
            .is_some_and(|entry| entry.generation() == generation);
        if !is_current {
            let _ = shutdown.send(true);
            task.abort();
            return Err(AppError::cancelled(
                "The database tunnel was closed while it was opening.",
            ));
        }

        let permit = inner
            .tunnels
            .remove(&profile_id)
            .expect("the checked opening entry exists")
            .into_permit();
        let entry = TunnelEntry::Active {
            server_id,
            target,
            local_port,
            opened_at: now_millis(),
            generation,
            shutdown,
            task,
            _permit: permit,
        };
        let snapshot = entry.snapshot(&profile_id);
        inner.tunnels.insert(profile_id, entry);
        Ok(snapshot)
    }

    /// Closes one profile tunnel and waits for its listener task to stop.
    pub async fn close(&self, profile_id: &str) -> Result<(), AppError> {
        let entry = {
            let mut inner = self.lock()?;
            inner.tunnels.remove(profile_id)
        };

        match entry {
            None => Ok(()),
            Some(TunnelEntry::Opening { listener, .. }) => {
                // The opening task notices the missing entry before it
                // publishes a listener. Drop the shared listener now so even an
                // in-flight setup cannot leave a port bound.
                let _ = take_listener(&listener);
                Ok(())
            }
            Some(TunnelEntry::Active {
                shutdown, mut task, ..
            }) => {
                let _ = shutdown.send(true);
                if tokio::time::timeout(self.timeouts.close, &mut task)
                    .await
                    .is_err()
                {
                    task.abort();
                    let _ = task.await;
                }
                Ok(())
            }
        }
    }

    /// Closes every tunnel attached to one SSH server profile.
    pub async fn close_server(&self, server_id: &str) {
        let profile_ids = match self.inner.lock() {
            Ok(inner) => inner
                .tunnels
                .iter()
                .filter(|(_, entry)| entry.server_id() == server_id)
                .map(|(profile_id, _)| profile_id.clone())
                .collect::<Vec<_>>(),
            Err(_) => return,
        };

        for profile_id in profile_ids {
            let _ = self.close(&profile_id).await;
        }
    }

    /// Closes every tunnel. Called during application exit.
    pub async fn close_all(&self) {
        let profile_ids = match self.inner.lock() {
            Ok(inner) => inner.tunnels.keys().cloned().collect::<Vec<_>>(),
            Err(_) => return,
        };

        for profile_id in profile_ids {
            let _ = self.close(&profile_id).await;
        }
    }

    /// Returns the current tunnel state for one database profile.
    pub fn snapshot(&self, profile_id: &str) -> Result<Option<TunnelSnapshot>, AppError> {
        let inner = self.lock()?;
        Ok(inner
            .tunnels
            .get(profile_id)
            .map(|entry| entry.snapshot(profile_id)))
    }

    /// Lists every open or opening tunnel in deterministic profile order.
    pub fn list(&self) -> Result<Vec<TunnelSnapshot>, AppError> {
        let inner = self.lock()?;
        let mut snapshots = inner
            .tunnels
            .iter()
            .map(|(profile_id, entry)| entry.snapshot(profile_id))
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
        Ok(snapshots)
    }

    fn bind_loopback(&self) -> Result<TcpListener, AppError> {
        let listener = self
            .listener_factory
            .bind_loopback()
            .map_err(|error| loopback_error().with_details(format!("{error}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| loopback_error())?;
        TcpListener::from_std(listener).map_err(|_| loopback_error())
    }

    fn discard_opening(&self, profile_id: &str, generation: u64, listener: &SharedListener) {
        let _ = take_listener(listener);
        self.remove_opening(profile_id, generation);
    }

    fn remove_opening(&self, profile_id: &str, generation: u64) {
        if let Ok(mut inner) = self.inner.lock()
            && inner
                .tunnels
                .get(profile_id)
                .is_some_and(|entry| entry.generation() == generation)
        {
            inner.tunnels.remove(profile_id);
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, TunnelInner>, AppError> {
        self.inner
            .lock()
            .map_err(|_| AppError::internal("Database tunnel manager lock was poisoned."))
    }
}

impl Drop for TunnelManager {
    fn drop(&mut self) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };

        for (_, entry) in inner.tunnels.drain() {
            match entry {
                TunnelEntry::Opening { listener, .. } => {
                    let _ = take_listener(&listener);
                }
                TunnelEntry::Active { shutdown, task, .. } => {
                    let _ = shutdown.send(true);
                    task.abort();
                }
            }
        }
    }
}

struct TunnelConnectionEventSink {
    manager: Weak<TunnelManager>,
}

impl ConnectionEventSink for TunnelConnectionEventSink {
    fn emit(&self, snapshot: &ConnectionSnapshot) {
        if snapshot.state == ConnectionState::Connected {
            return;
        }
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let server_id = snapshot.server_id.clone();
        tokio::spawn(async move {
            manager.close_server(&server_id).await;
        });
    }
}

async fn run_tunnel_listener(
    listener: TcpListener,
    session: Arc<dyn SshSession>,
    remote_host: String,
    remote_port: u16,
    mut shutdown: watch::Receiver<bool>,
    open_timeout: Duration,
) {
    let clients = Arc::new(Semaphore::new(MAX_FORWARDED_CONNECTIONS));
    let mut health = tokio::time::interval(SESSION_HEALTH_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut tasks = JoinSet::new();

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = health.tick() => {
                if !session.is_healthy().await {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let Ok(permit) = Arc::clone(&clients).try_acquire_owned() else {
                            // Bound forwarded clients deterministically instead
                            // of growing an unbounded task set.
                            drop(stream);
                            continue;
                        };
                        let session = Arc::clone(&session);
                        let remote_host = remote_host.clone();
                        tasks.spawn(forward_connection(
                            stream,
                            session,
                            remote_host,
                            remote_port,
                            open_timeout,
                            permit,
                        ));
                    }
                    Err(_) => {
                        if *shutdown.borrow() {
                            break;
                        }
                        tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                    }
                }
            }
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }

    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn forward_connection(
    mut local: TcpStream,
    session: Arc<dyn SshSession>,
    remote_host: String,
    remote_port: u16,
    open_timeout: Duration,
    _permit: OwnedSemaphorePermit,
) {
    let Ok(Ok(mut remote)) = tokio::time::timeout(
        open_timeout,
        session.open_direct_tcpip(&remote_host, remote_port),
    )
    .await
    else {
        return;
    };

    let _ = copy_bidirectional(&mut local, &mut remote).await;
    let _ = remote.shutdown().await;
    let _ = local.shutdown().await;
}

fn validate_request(request: TunnelRequest) -> Result<(TunnelRequest, TunnelTarget), AppError> {
    let profile_id = request.profile_id.trim();
    if profile_id.is_empty() {
        return Err(AppError::validation("A database profile is required."));
    }

    let server_id = request.server_id.trim();
    if server_id.is_empty() {
        return Err(AppError::validation("A server profile is required."));
    }

    let remote_host = request.remote_host.trim();
    if remote_host.is_empty() {
        return Err(AppError::validation(
            "The database tunnel remote host is required.",
        ));
    }
    if remote_host.len() > MAX_REMOTE_HOST_LENGTH
        || remote_host.chars().any(char::is_control)
        || remote_host.chars().any(char::is_whitespace)
    {
        return Err(AppError::validation(
            "The database tunnel remote host is not valid.",
        ));
    }

    if request.remote_port == 0 {
        return Err(AppError::validation(
            "The database tunnel remote port must be between 1 and 65535.",
        ));
    }

    let socket = request
        .socket
        .map(|socket| socket.trim().to_string())
        .filter(|socket| !socket.is_empty());
    if socket.as_ref().is_some_and(|socket| {
        socket.len() > MAX_SOCKET_PATH_LENGTH
            || socket.chars().any(char::is_control)
            || socket.contains('\0')
    }) {
        return Err(AppError::validation(
            "The database Unix socket metadata is not valid.",
        ));
    }

    let target = TunnelTarget {
        host: remote_host.to_string(),
        port: request.remote_port,
        socket: socket.clone(),
    };
    Ok((
        TunnelRequest {
            profile_id: profile_id.to_string(),
            server_id: server_id.to_string(),
            remote_host: remote_host.to_string(),
            remote_port: request.remote_port,
            socket,
        },
        target,
    ))
}

fn take_listener(listener: &SharedListener) -> Option<TcpListener> {
    listener
        .lock()
        .ok()
        .and_then(|mut listener| listener.take())
}

fn loopback_error() -> AppError {
    AppError::network("Could not allocate a loopback listener for the database tunnel.")
        .with_details(
            "ServerDesk binds database tunnels to 127.0.0.1 on an ephemeral port only. \
         Check local network resources and retry.",
        )
}

fn map_open_error(error: TransportError) -> AppError {
    match error {
        TransportError::Timeout => {
            AppError::timeout("The database tunnel could not be opened in time.")
        }
        TransportError::Network => {
            AppError::network("The SSH connection was lost while opening the database tunnel.")
        }
        TransportError::Authentication => AppError::authentication(
            "SSH authentication failed before the database tunnel could open.",
        ),
        TransportError::Remote(detail) => {
            AppError::remote("The SSH server refused to open the database tunnel.")
                .with_details(detail)
        }
        TransportError::Protocol => {
            AppError::remote("The SSH server rejected the database tunnel request.")
        }
        other => other.to_app_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_listener_is_loopback_only_with_an_ephemeral_port() {
        let listener = LoopbackTunnelListenerFactory
            .bind_loopback()
            .expect("loopback listener");
        let address = listener.local_addr().expect("local address");

        assert!(address.ip().is_loopback());
        assert_eq!(address.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(address.port(), 0);
    }

    #[test]
    fn rejects_invalid_remote_targets() {
        let base = TunnelRequest {
            profile_id: "profile-1".to_string(),
            server_id: "server-1".to_string(),
            remote_host: "127.0.0.1".to_string(),
            remote_port: 5432,
            socket: None,
        };

        for request in [
            TunnelRequest {
                profile_id: String::new(),
                ..base.clone()
            },
            TunnelRequest {
                server_id: String::new(),
                ..base.clone()
            },
            TunnelRequest {
                remote_host: String::new(),
                ..base.clone()
            },
            TunnelRequest {
                remote_host: "db host".to_string(),
                ..base.clone()
            },
            TunnelRequest {
                remote_host: "db\nhost".to_string(),
                ..base.clone()
            },
            TunnelRequest {
                remote_port: 0,
                ..base.clone()
            },
            TunnelRequest {
                socket: Some("/var/run\nmysql.sock".to_string()),
                ..base.clone()
            },
        ] {
            assert!(validate_request(request).is_err());
        }
    }

    #[test]
    fn accepts_ipv6_and_socket_metadata() {
        let (request, target) = validate_request(TunnelRequest {
            profile_id: "profile-1".to_string(),
            server_id: "server-1".to_string(),
            remote_host: "::1".to_string(),
            remote_port: 5432,
            socket: Some("/var/run/postgresql/.s.PGSQL.5432".to_string()),
        })
        .expect("valid request");

        assert_eq!(target.host, "::1");
        assert_eq!(
            request.socket.as_deref(),
            Some("/var/run/postgresql/.s.PGSQL.5432")
        );
    }
}
