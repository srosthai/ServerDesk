//! Connection lifecycle for saved database profiles.
//!
//! The service is the only backend component that combines a profile, its
//! credential-store password, and the loopback endpoint published by the SSH
//! tunnel. Commands added later call this service; driver objects and tunnel
//! details stay out of the Tauri payloads.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::Mutex;

use uuid::Uuid;

use crate::{
    core::{
        AppError,
        database_profiles::DatabaseTestResult,
        ssh::{TunnelManager, TunnelRequest, TunnelSnapshot},
    },
    storage::{DatabaseConnection, DatabaseEngine},
};

use super::{
    CapabilitySnapshot, DatabaseAdapter, DatabaseConnectRequest, DatabaseConnectionInfo,
    DatabasePassword, DatabasePing, MysqlDatabaseAdapter, PostgresDatabaseAdapter,
};

/// Default bound for opening a database session through its tunnel.
pub const DEFAULT_DATABASE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

struct ConnectedDatabase {
    adapter: Arc<Mutex<Box<dyn DatabaseAdapter>>>,
    info: DatabaseConnectionInfo,
}

/// Owns live database adapters keyed by saved profile id.
pub struct DatabaseConnectionRegistry {
    tunnels: Arc<TunnelManager>,
    connections: Mutex<HashMap<String, ConnectedDatabase>>,
    connect_timeout: Duration,
}

impl DatabaseConnectionRegistry {
    pub fn new(tunnels: Arc<TunnelManager>) -> Self {
        Self::with_timeout(tunnels, DEFAULT_DATABASE_CONNECT_TIMEOUT)
    }

    pub fn with_timeout(tunnels: Arc<TunnelManager>, connect_timeout: Duration) -> Self {
        Self {
            tunnels,
            connections: Mutex::new(HashMap::new()),
            connect_timeout,
        }
    }

    /// Connects a saved profile through its loopback tunnel.
    ///
    /// The password is supplied by the credential service and is only passed
    /// to the driver. A failed connect closes a tunnel that this call opened;
    /// an already-active tunnel is left for an explicit retry.
    pub async fn connect(
        &self,
        profile: &DatabaseConnection,
        password: DatabasePassword,
    ) -> Result<DatabaseConnectionInfo, AppError> {
        let adapter = adapter_for(profile.engine)?;

        // Reconnect replaces the previous adapter and tunnel deterministically.
        self.disconnect(&profile.id).await?;

        let tunnel = self
            .tunnels
            .open(TunnelRequest {
                profile_id: profile.id.clone(),
                server_id: profile.server_id.clone(),
                remote_host: profile.host.clone(),
                remote_port: profile.port,
                socket: profile.socket.clone(),
            })
            .await?;
        let request = match connect_request(profile, &tunnel, password, self.connect_timeout) {
            Ok(request) => request,
            Err(error) => {
                let _ = self.tunnels.close(&profile.id).await;
                return Err(error);
            }
        };

        let mut adapter = adapter;
        match adapter.connect(request).await {
            Ok(info) => {
                self.connections.lock().await.insert(
                    profile.id.clone(),
                    ConnectedDatabase {
                        adapter: Arc::new(Mutex::new(adapter)),
                        info: info.clone(),
                    },
                );
                Ok(info)
            }
            Err(error) => {
                let _ = self.tunnels.close(&profile.id).await;
                Err(error.into())
            }
        }
    }

    /// Closes one adapter and its tunnel. Missing sessions are idempotent.
    pub async fn disconnect(&self, profile_id: &str) -> Result<(), AppError> {
        let session = self.connections.lock().await.remove(profile_id);
        let adapter_result = match session {
            Some(session) => {
                let mut adapter = session.adapter.lock().await;
                adapter.close().await.map_err(AppError::from)
            }
            None => Ok(()),
        };

        let tunnel_result = self.tunnels.close(profile_id).await;
        adapter_result.and(tunnel_result)
    }

    /// Probes one saved profile through a transient tunnel and adapter.
    ///
    /// The test never changes the active session for `profile.id`: it uses a
    /// separate tunnel key and closes both resources before returning. The
    /// returned document contains only version, capabilities, read-only state,
    /// and latency.
    pub async fn test(
        &self,
        profile: &DatabaseConnection,
        password: DatabasePassword,
    ) -> Result<DatabaseTestResult, AppError> {
        let adapter = adapter_for(profile.engine)?;
        let tunnel_profile_id = format!("database-test:{}", Uuid::new_v4());
        let tunnel = self
            .tunnels
            .open(TunnelRequest {
                profile_id: tunnel_profile_id.clone(),
                server_id: profile.server_id.clone(),
                remote_host: profile.host.clone(),
                remote_port: profile.port,
                socket: profile.socket.clone(),
            })
            .await?;

        let request = match connect_request(profile, &tunnel, password, self.connect_timeout) {
            Ok(request) => request,
            Err(error) => {
                let _ = self.tunnels.close(&tunnel_profile_id).await;
                return Err(error);
            }
        };

        let mut adapter = adapter;
        let result = match adapter.connect(request).await {
            Ok(info) => match adapter.ping().await {
                Ok(ping) => Ok(DatabaseTestResult::new(profile, info, ping.latency_ms)),
                Err(error) => Err(error.into()),
            },
            Err(error) => Err(error.into()),
        };

        let _ = adapter.close().await;
        let _ = self.tunnels.close(&tunnel_profile_id).await;
        result
    }

    /// Pings the live adapter without exposing driver details.
    pub async fn ping(&self, profile_id: &str) -> Result<DatabasePing, AppError> {
        let adapter = self.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter.ping().await.map_err(AppError::from)
    }

    /// Returns the last successful connection metadata.
    pub async fn info(&self, profile_id: &str) -> Result<DatabaseConnectionInfo, AppError> {
        self.connections
            .lock()
            .await
            .get(profile_id)
            .map(|session| session.info.clone())
            .ok_or_else(|| AppError::connection("The database connection is not active."))
    }

    /// Returns the connected server's capability document.
    pub async fn capabilities(&self, profile_id: &str) -> Result<CapabilitySnapshot, AppError> {
        Ok(self.info(profile_id).await?.capabilities)
    }

    /// Lists active profile ids in deterministic order.
    pub async fn connected_profiles(&self) -> Vec<String> {
        let mut profiles = self
            .connections
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        profiles.sort();
        profiles
    }

    pub async fn is_connected(&self, profile_id: &str) -> bool {
        self.connections.lock().await.contains_key(profile_id)
    }

    /// Closes every adapter and tunnel. Called during application exit.
    pub async fn close_all(&self) {
        let profile_ids = self.connected_profiles().await;
        for profile_id in profile_ids {
            let _ = self.disconnect(&profile_id).await;
        }
    }

    async fn adapter(
        &self,
        profile_id: &str,
    ) -> Result<Arc<Mutex<Box<dyn DatabaseAdapter>>>, AppError> {
        self.connections
            .lock()
            .await
            .get(profile_id)
            .map(|session| Arc::clone(&session.adapter))
            .ok_or_else(|| AppError::connection("The database connection is not active."))
    }
}

fn connect_request(
    profile: &DatabaseConnection,
    tunnel: &TunnelSnapshot,
    password: DatabasePassword,
    connect_timeout: Duration,
) -> Result<DatabaseConnectRequest, AppError> {
    let local_port = tunnel.local_port.ok_or_else(|| {
        AppError::internal("The database tunnel did not publish a loopback port.")
    })?;

    Ok(DatabaseConnectRequest {
        host: tunnel.local_host.clone(),
        port: local_port,
        socket: profile.socket.clone(),
        database: profile.database_name.clone(),
        username: profile.username.clone(),
        password,
        read_only: profile.read_only,
        ssl_mode: profile.ssl_mode.clone(),
        connect_timeout,
    })
}

fn adapter_for(engine: DatabaseEngine) -> Result<Box<dyn DatabaseAdapter>, AppError> {
    match engine {
        DatabaseEngine::Mysql => Ok(Box::new(MysqlDatabaseAdapter::new())),
        DatabaseEngine::Postgresql => Ok(Box::new(PostgresDatabaseAdapter::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_profiles_select_the_engine_adapter() {
        let mysql = adapter_for(DatabaseEngine::Mysql).expect("mysql adapter");
        assert_eq!(mysql.engine(), DatabaseEngine::Mysql);
        assert_eq!(mysql.dialect().engine(), DatabaseEngine::Mysql);

        let postgres = adapter_for(DatabaseEngine::Postgresql).expect("postgres adapter");
        assert_eq!(postgres.engine(), DatabaseEngine::Postgresql);
        assert_eq!(postgres.dialect().engine(), DatabaseEngine::Postgresql);
        assert_eq!(
            postgres.capabilities().snapshot().engine,
            DatabaseEngine::Postgresql
        );
    }
}
