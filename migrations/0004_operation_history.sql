-- Operation history keeps only bounded, already-sanitized metadata.
-- Raw command output, terminal input, environment files, passwords, keys,
-- passphrases, and tokens are never stored here.

ALTER TABLE operation_history ADD COLUMN category TEXT NOT NULL DEFAULT 'command'
    CHECK (category IN ('command', 'service', 'artisan', 'sftp', 'queue'));

ALTER TABLE operation_history ADD COLUMN result TEXT NOT NULL DEFAULT 'unknown'
    CHECK (result IN ('success', 'failure', 'cancelled', 'unknown'));

ALTER TABLE operation_history ADD COLUMN path TEXT;

ALTER TABLE operation_history ADD COLUMN output_summary TEXT;

-- Classify rows recorded before the result column existed from their exit status.
UPDATE operation_history
SET result = CASE
    WHEN exit_code = 0 THEN 'success'
    WHEN exit_code IS NULL THEN 'unknown'
    ELSE 'failure'
END;

CREATE INDEX IF NOT EXISTS idx_operation_history_category_started
    ON operation_history (category, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_operation_history_result_started
    ON operation_history (result, started_at DESC);
