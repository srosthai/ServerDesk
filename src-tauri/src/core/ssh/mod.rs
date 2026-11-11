mod manager;
mod private_key;
mod russh_transport;
mod testing;
mod tunnel;
mod types;

pub use manager::ConnectionManager;
pub use private_key::validate_private_key;
pub use russh_transport::RusshTransport;
pub use testing::{ConnectionTestOutcome, test_connection};
pub use tunnel::{
    DEFAULT_MAX_TUNNELS, LoopbackTunnelListenerFactory, MAX_FORWARDED_CONNECTIONS,
    TunnelListenerFactory, TunnelManager, TunnelRequest, TunnelSnapshot, TunnelState,
    TunnelTimeouts,
};
pub use types::{
    Authentication, CONNECTION_STATE_EVENT, CommandOutput, ConnectionEventSink, ConnectionRequest,
    ConnectionSnapshot, ConnectionState, ConnectionTimeouts, HostKeyDetails, SftpBrowser,
    SftpDirEntry, SftpEntryKind, SftpError, SftpFile, SftpMetadata, SftpOpenMode, SshSession,
    SshTransport, TransportError, TrustedHostKey, TunnelStream, validate_host_key_metadata,
};
