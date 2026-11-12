-- Add the database lifecycle category to operation history.
--
-- SQLite cannot alter a CHECK constraint in place, so the table is rebuilt
-- with the same columns, foreign keys, and data. No raw SQL, password, or
-- engine driver text is introduced by this migration.

CREATE TABLE IF NOT EXISTS operation_history_new (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT,
    project_id TEXT,
    category TEXT NOT NULL DEFAULT 'command'
        CHECK (category IN ('command', 'service', 'artisan', 'sftp', 'queue', 'database')),
    operation TEXT NOT NULL,
    safe_summary TEXT NOT NULL,
    path TEXT,
    output_summary TEXT,
    result TEXT NOT NULL DEFAULT 'unknown'
        CHECK (result IN ('success', 'failure', 'cancelled', 'unknown')),
    exit_code INTEGER,
    started_at INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (server_id) REFERENCES servers (id) ON DELETE SET NULL,
    FOREIGN KEY (project_id) REFERENCES projects (id) ON DELETE SET NULL
);

INSERT INTO operation_history_new (
    id, server_id, project_id, category, operation, safe_summary,
    path, output_summary, result, exit_code, started_at, duration_ms
)
SELECT
    id, server_id, project_id, category, operation, safe_summary,
    path, output_summary, result, exit_code, started_at, duration_ms
FROM operation_history;

DROP TABLE operation_history;

ALTER TABLE operation_history_new RENAME TO operation_history;

CREATE INDEX IF NOT EXISTS idx_operation_history_server_started
    ON operation_history (server_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_operation_history_started
    ON operation_history (started_at DESC);
CREATE INDEX IF NOT EXISTS idx_operation_history_category_started
    ON operation_history (category, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_operation_history_result_started
    ON operation_history (result, started_at DESC);
