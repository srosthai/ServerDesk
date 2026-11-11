-- Database Manager metadata.
--
-- Secrets never belong in this database. Database passwords, private keys,
-- and passphrases are stored in the operating system credential store and
-- referenced by an opaque `credential_ref` only. There are deliberately no
-- password, secret, key, or passphrase columns in any table below.

CREATE TABLE IF NOT EXISTS database_connections (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT NOT NULL,
    name TEXT NOT NULL,
    engine TEXT NOT NULL CHECK (engine IN ('mysql', 'postgresql')),
    host TEXT NOT NULL,
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    socket TEXT,
    database_name TEXT,
    username TEXT NOT NULL,
    credential_ref TEXT,
    read_only INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1)),
    ssl_mode TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER,
    FOREIGN KEY (server_id) REFERENCES servers (id) ON DELETE CASCADE
);

-- Profile names are unique per server regardless of letter case.
CREATE UNIQUE INDEX IF NOT EXISTS idx_database_connections_server_name_unique
    ON database_connections (server_id, name COLLATE NOCASE);

CREATE INDEX IF NOT EXISTS idx_database_connections_server_id
    ON database_connections (server_id);

-- Query history keeps only bounded, already-sanitized SQL text. Raw result
-- sets, parameters, credentials, and environment files are never stored.
CREATE TABLE IF NOT EXISTS database_query_history (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    engine TEXT NOT NULL CHECK (engine IN ('mysql', 'postgresql')),
    database_name TEXT,
    sql_text TEXT NOT NULL CHECK (length(sql_text) BETWEEN 1 AND 65536),
    result TEXT NOT NULL CHECK (result IN ('success', 'failure', 'cancelled')),
    duration_ms INTEGER NOT NULL DEFAULT 0 CHECK (duration_ms >= 0),
    row_count INTEGER CHECK (row_count IS NULL OR row_count >= 0),
    error_summary TEXT CHECK (
        error_summary IS NULL OR length(error_summary) <= 4096
    ),
    started_at INTEGER NOT NULL,
    FOREIGN KEY (server_id) REFERENCES servers (id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES database_connections (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_database_query_history_connection_started
    ON database_query_history (connection_id, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_database_query_history_server_started
    ON database_query_history (server_id, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_database_query_history_result_started
    ON database_query_history (result, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_database_query_history_started
    ON database_query_history (started_at DESC);

-- Reusable query snippets are local, engine-tagged SQL bodies. They are not
-- tied to a server so the same snippet can be reused on any saved profile.
CREATE TABLE IF NOT EXISTS database_saved_queries (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    engine TEXT NOT NULL CHECK (engine IN ('mysql', 'postgresql')),
    sql_text TEXT NOT NULL CHECK (length(sql_text) BETWEEN 1 AND 65536),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_database_saved_queries_engine_name
    ON database_saved_queries (engine, name COLLATE NOCASE);
