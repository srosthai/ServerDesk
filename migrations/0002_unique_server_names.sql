CREATE UNIQUE INDEX IF NOT EXISTS idx_servers_name_unique
    ON servers (name COLLATE NOCASE);
