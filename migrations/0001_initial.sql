-- ServerDesk initial schema.
-- Stores metadata and credential references only. Secrets never belong in this database.

CREATE TABLE IF NOT EXISTS servers (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22 CHECK (port BETWEEN 1 AND 65535),
    username TEXT NOT NULL,
    auth_type TEXT NOT NULL CHECK (auth_type IN ('password', 'private_key')),
    credential_ref TEXT,
    group_name TEXT,
    tags TEXT NOT NULL DEFAULT '[]',
    notes TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS known_hosts (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT NOT NULL,
    key_type TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    first_seen_at INTEGER NOT NULL,
    verified_at INTEGER,
    FOREIGN KEY (server_id) REFERENCES servers (id) ON DELETE CASCADE,
    UNIQUE (server_id, key_type)
);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT NOT NULL,
    name TEXT NOT NULL,
    remote_path TEXT NOT NULL,
    project_type TEXT NOT NULL DEFAULT 'laravel',
    detected_at INTEGER NOT NULL,
    FOREIGN KEY (server_id) REFERENCES servers (id) ON DELETE CASCADE,
    UNIQUE (server_id, remote_path)
);

CREATE TABLE IF NOT EXISTS saved_commands (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    command_template TEXT NOT NULL,
    requires_confirmation INTEGER NOT NULL DEFAULT 0 CHECK (requires_confirmation IN (0, 1))
);

CREATE TABLE IF NOT EXISTS operation_history (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT,
    project_id TEXT,
    operation TEXT NOT NULL,
    safe_summary TEXT NOT NULL,
    exit_code INTEGER,
    started_at INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (server_id) REFERENCES servers (id) ON DELETE SET NULL,
    FOREIGN KEY (project_id) REFERENCES projects (id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS preferences (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS port_forwards (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT NOT NULL,
    bind_host TEXT NOT NULL DEFAULT '127.0.0.1',
    local_port INTEGER NOT NULL CHECK (local_port BETWEEN 1 AND 65535),
    remote_host TEXT NOT NULL,
    remote_port INTEGER NOT NULL CHECK (remote_port BETWEEN 1 AND 65535),
    created_at INTEGER NOT NULL,
    FOREIGN KEY (server_id) REFERENCES servers (id) ON DELETE CASCADE,
    UNIQUE (server_id, bind_host, local_port)
);

CREATE INDEX IF NOT EXISTS idx_servers_name ON servers (name);
CREATE INDEX IF NOT EXISTS idx_known_hosts_server_id ON known_hosts (server_id);
CREATE INDEX IF NOT EXISTS idx_projects_server_id ON projects (server_id);
CREATE INDEX IF NOT EXISTS idx_operation_history_server_started
    ON operation_history (server_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_operation_history_started
    ON operation_history (started_at DESC);
CREATE INDEX IF NOT EXISTS idx_port_forwards_server_id ON port_forwards (server_id);
