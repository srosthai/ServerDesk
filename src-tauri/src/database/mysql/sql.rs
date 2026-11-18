//! Fixed MySQL metadata SQL.
//!
//! These statements are driver building blocks, not a command contract.
//! Dynamic values use `?` placeholders and identifiers are quoted by
//! [`super::MysqlDialect`]. The SQL strings are public so engine modules and
//! their tests can inspect them; Tauri commands and React never receive them.

pub const PING: &str = "SELECT 1";
pub const SERVER_VERSION: &str = "SELECT VERSION()";

/// Catalog-only size for the active schema. It never reads table rows or runs
/// statistics maintenance.
pub const HEALTH_DATABASE_SIZE: &str = "SELECT \
    CAST(COALESCE(SUM(COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)), 0) AS SIGNED) \
        AS size_bytes \
    FROM information_schema.TABLES \
    WHERE TABLE_SCHEMA = COALESCE(?, DATABASE())";

/// Largest tables in the active schema, read from catalog metadata only. The
/// caller asks for one extra row to disclose truncation.
pub const HEALTH_TABLE_SIZES: &str = "SELECT TABLE_SCHEMA AS database_name, \
    TABLE_NAME AS name, TABLE_TYPE AS table_type, \
    CAST(DATA_LENGTH + INDEX_LENGTH AS SIGNED) AS size_bytes, \
    CAST(TABLE_ROWS AS SIGNED) AS estimated_rows \
    FROM information_schema.TABLES \
    WHERE TABLE_SCHEMA = COALESCE(?, DATABASE()) \
    AND TABLE_TYPE = 'BASE TABLE' \
    ORDER BY COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0) DESC, TABLE_NAME \
    LIMIT ?";

pub const LIST_DATABASES: &str = "SELECT s.SCHEMA_NAME AS name, \
    s.DEFAULT_CHARACTER_SET_NAME AS encoding, s.DEFAULT_COLLATION_NAME AS collation, \
    COALESCE(t.size_bytes, 0) AS size_bytes \
    FROM information_schema.SCHEMATA s \
    LEFT JOIN ( \
        SELECT TABLE_SCHEMA, \
            CAST(SUM(COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)) AS SIGNED) AS size_bytes \
        FROM information_schema.TABLES \
        GROUP BY TABLE_SCHEMA \
    ) t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
    ORDER BY s.SCHEMA_NAME LIMIT ? OFFSET ?";

pub const DATABASE_SUMMARY: &str = "SELECT s.SCHEMA_NAME AS name, \
    s.DEFAULT_CHARACTER_SET_NAME AS encoding, s.DEFAULT_COLLATION_NAME AS collation, \
    COALESCE(t.size_bytes, 0) AS size_bytes \
    FROM information_schema.SCHEMATA s \
    LEFT JOIN ( \
        SELECT TABLE_SCHEMA, \
            CAST(SUM(COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)) AS SIGNED) AS size_bytes \
        FROM information_schema.TABLES \
        GROUP BY TABLE_SCHEMA \
    ) t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
    WHERE s.SCHEMA_NAME = ? LIMIT 1";

pub const CURRENT_DATABASE: &str = "SELECT DATABASE() AS database_name";

pub const LIST_SCHEMAS: &str = "SELECT s.SCHEMA_NAME AS name, \
    s.DEFAULT_CHARACTER_SET_NAME AS encoding, s.DEFAULT_COLLATION_NAME AS collation, \
    COALESCE(t.size_bytes, 0) AS size_bytes \
    FROM information_schema.SCHEMATA s \
    LEFT JOIN ( \
        SELECT TABLE_SCHEMA, \
            CAST(SUM(COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)) AS SIGNED) AS size_bytes \
        FROM information_schema.TABLES \
        GROUP BY TABLE_SCHEMA \
    ) t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
    ORDER BY s.SCHEMA_NAME LIMIT ? OFFSET ?";

pub const LIST_TABLES: &str = "SELECT TABLE_SCHEMA AS database_name, TABLE_NAME AS name, \
    TABLE_TYPE AS table_type, CAST(TABLE_ROWS AS SIGNED) AS estimated_rows, \
    CAST(COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0) AS SIGNED) AS size_bytes, \
    ENGINE AS engine \
    FROM information_schema.TABLES \
    WHERE TABLE_TYPE = 'BASE TABLE' \
    AND (? IS NULL OR TABLE_SCHEMA = ?) \
    ORDER BY TABLE_SCHEMA, TABLE_NAME LIMIT ? OFFSET ?";

pub const TABLE_SUMMARY: &str = "SELECT TABLE_SCHEMA AS database_name, TABLE_NAME AS name, \
    TABLE_TYPE AS table_type, CAST(TABLE_ROWS AS SIGNED) AS estimated_rows, \
    CAST(COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0) AS SIGNED) AS size_bytes, \
    ENGINE AS engine \
    FROM information_schema.TABLES \
    WHERE TABLE_TYPE = 'BASE TABLE' \
    AND TABLE_SCHEMA = COALESCE(?, DATABASE()) \
    AND TABLE_NAME = ? LIMIT 1";

pub const LIST_VIEWS: &str = "SELECT TABLE_SCHEMA AS database_name, TABLE_NAME AS name, \
    DEFINER AS owner \
    FROM information_schema.VIEWS \
    WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
    ORDER BY TABLE_SCHEMA, TABLE_NAME LIMIT ? OFFSET ?";

pub const VIEW_DEFINITION: &str = "SELECT TABLE_SCHEMA AS database_name, TABLE_NAME AS name, \
    VIEW_DEFINITION AS view_query, DEFINER AS owner, CHECK_OPTION AS check_option, \
    IS_UPDATABLE AS is_updatable \
    FROM information_schema.VIEWS \
    WHERE TABLE_SCHEMA = COALESCE(?, DATABASE()) AND TABLE_NAME = ? LIMIT 1";

pub const LIST_ROUTINES: &str = "SELECT ROUTINE_SCHEMA AS database_name, ROUTINE_NAME AS name, \
    ROUTINE_TYPE AS routine_type, DTD_IDENTIFIER AS return_type, \
    EXTERNAL_LANGUAGE AS language, DEFINER AS owner, ROUTINE_DEFINITION AS definition, \
    ROUTINE_BODY AS body_type \
    FROM information_schema.ROUTINES \
    WHERE (? IS NULL OR ROUTINE_SCHEMA = ?) \
    ORDER BY ROUTINE_SCHEMA, ROUTINE_NAME LIMIT ? OFFSET ?";

pub const ROUTINE_DEFINITION: &str = "SELECT ROUTINE_SCHEMA AS database_name, \
    ROUTINE_NAME AS name, ROUTINE_TYPE AS routine_type, DTD_IDENTIFIER AS return_type, \
    EXTERNAL_LANGUAGE AS language, DEFINER AS owner, ROUTINE_DEFINITION AS definition, \
    ROUTINE_BODY AS body_type \
    FROM information_schema.ROUTINES \
    WHERE ROUTINE_SCHEMA = COALESCE(?, DATABASE()) AND ROUTINE_NAME = ? \
    AND (? IS NULL OR ROUTINE_TYPE = ?) LIMIT 1";

pub const LIST_TRIGGERS: &str = "SELECT TRIGGER_SCHEMA AS database_name, TRIGGER_NAME AS name, \
    EVENT_OBJECT_TABLE AS table_name, ACTION_TIMING AS timing, \
    EVENT_MANIPULATION AS event_name, DEFINER AS owner, ACTION_STATEMENT AS statement \
    FROM information_schema.TRIGGERS \
    WHERE (? IS NULL OR TRIGGER_SCHEMA = ?) \
    ORDER BY TRIGGER_SCHEMA, TRIGGER_NAME LIMIT ? OFFSET ?";

pub const TRIGGER_DEFINITION: &str = "SELECT TRIGGER_SCHEMA AS database_name, \
    TRIGGER_NAME AS name, EVENT_OBJECT_TABLE AS table_name, ACTION_TIMING AS timing, \
    EVENT_MANIPULATION AS event_name, DEFINER AS owner, ACTION_STATEMENT AS statement, \
    ACTION_ORDER AS action_order, ACTION_ORIENTATION AS action_orientation \
    FROM information_schema.TRIGGERS \
    WHERE TRIGGER_SCHEMA = COALESCE(?, DATABASE()) AND TRIGGER_NAME = ? LIMIT 1";

pub const OBJECT_COUNTS: &str = "SELECT \
    CAST(COALESCE(SUM(CASE WHEN TABLE_TYPE = 'BASE TABLE' THEN 1 ELSE 0 END), 0) AS SIGNED) \
        AS table_count, \
    CAST(COALESCE(SUM(CASE WHEN TABLE_TYPE = 'VIEW' THEN 1 ELSE 0 END), 0) AS SIGNED) \
        AS view_count, \
    (SELECT COUNT(*) FROM information_schema.ROUTINES r \
        WHERE (? IS NULL OR r.ROUTINE_SCHEMA = ?)) AS routine_count, \
    (SELECT COUNT(*) FROM information_schema.TRIGGERS tr \
        WHERE (? IS NULL OR tr.TRIGGER_SCHEMA = ?)) AS trigger_count \
    FROM information_schema.TABLES \
    WHERE (? IS NULL OR TABLE_SCHEMA = ?)";

/// Statistics timestamps are kept in a separate query because
/// `mysql.innodb_table_stats` is not readable by most application accounts.
/// The adapter turns a permission failure for that optional table into an
/// unavailable field instead of failing the table list.
pub const TABLE_STATISTICS_REFRESHED_AT: &str = "SELECT table_name AS name, \
    DATE_FORMAT(last_update, '%Y-%m-%dT%H:%i:%s') AS refreshed_at \
    FROM mysql.innodb_table_stats WHERE database_name = ?";

pub const LIST_COLUMNS: &str = "SELECT COLUMN_NAME AS name, COLUMN_TYPE AS data_type, \
    CAST(CHARACTER_MAXIMUM_LENGTH AS SIGNED) AS value_length, \
    CAST(COALESCE(NUMERIC_PRECISION, DATETIME_PRECISION) AS SIGNED) AS numeric_precision, \
    CAST(NUMERIC_SCALE AS SIGNED) AS numeric_scale, IS_NULLABLE = 'YES' AS nullable, \
    COLUMN_DEFAULT AS default_value, \
    CAST(ORDINAL_POSITION AS SIGNED) AS ordinal_position, \
    COLUMN_KEY = 'PRI' AS primary_key, \
    (EXTRA LIKE '%VIRTUAL GENERATED%' OR EXTRA LIKE '%STORED GENERATED%') AS `generated`, \
    GENERATION_EXPRESSION AS generation_expression, \
    EXTRA LIKE '%auto_increment%' AS auto_increment, COLUMN_COMMENT AS comment, \
    COLLATION_NAME AS collation \
    FROM information_schema.COLUMNS \
    WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
    AND (? IS NULL OR TABLE_NAME = ?) \
    ORDER BY ORDINAL_POSITION LIMIT ? OFFSET ?";

/// The derived table selects one bounded page of index names first, so a wide
/// index cannot make the adapter group an unbounded number of metadata rows in
/// memory.
pub const LIST_INDEXES: &str = "SELECT s.INDEX_NAME AS name, s.COLUMN_NAME AS column_name, \
    s.NON_UNIQUE = 0 AS unique_index, s.INDEX_NAME = 'PRIMARY' AS primary_index, \
    s.INDEX_TYPE AS method \
    FROM information_schema.STATISTICS s \
    JOIN ( \
        SELECT INDEX_NAME FROM information_schema.STATISTICS \
        WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
        AND (? IS NULL OR TABLE_NAME = ?) \
        GROUP BY INDEX_NAME ORDER BY INDEX_NAME LIMIT ? OFFSET ? \
    ) page ON page.INDEX_NAME = s.INDEX_NAME \
    WHERE (? IS NULL OR s.TABLE_SCHEMA = ?) \
    AND (? IS NULL OR s.TABLE_NAME = ?) \
    ORDER BY s.INDEX_NAME, s.SEQ_IN_INDEX";

/// MySQL 8.0.16 and MariaDB 10.2 expose `CHECK_CONSTRAINTS`. Older servers
/// use [`LIST_CONSTRAINTS_WITHOUT_CHECKS`] so the metadata query still works.
pub const LIST_CONSTRAINTS: &str = "SELECT tc.CONSTRAINT_NAME AS name, \
    tc.CONSTRAINT_TYPE AS kind, tc.TABLE_SCHEMA AS database_name, \
    tc.TABLE_NAME AS table_name, kcu.COLUMN_NAME AS column_name, \
    kcu.REFERENCED_TABLE_SCHEMA AS referenced_schema, \
    kcu.REFERENCED_TABLE_NAME AS referenced_table, \
    kcu.REFERENCED_COLUMN_NAME AS referenced_column, \
    rc.UPDATE_RULE AS on_update, rc.DELETE_RULE AS on_delete, \
    cc.CHECK_CLAUSE AS check_expression \
    FROM information_schema.TABLE_CONSTRAINTS tc \
    JOIN ( \
        SELECT CONSTRAINT_NAME FROM information_schema.TABLE_CONSTRAINTS \
        WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
        AND (? IS NULL OR TABLE_NAME = ?) \
        GROUP BY CONSTRAINT_NAME ORDER BY CONSTRAINT_NAME LIMIT ? OFFSET ? \
    ) page ON page.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    LEFT JOIN information_schema.KEY_COLUMN_USAGE kcu \
    ON kcu.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
    AND kcu.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    AND kcu.TABLE_SCHEMA = tc.TABLE_SCHEMA \
    AND kcu.TABLE_NAME = tc.TABLE_NAME \
    LEFT JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
    ON rc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
    AND rc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    AND rc.TABLE_NAME = tc.TABLE_NAME \
    LEFT JOIN information_schema.CHECK_CONSTRAINTS cc \
    ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
    AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    WHERE (? IS NULL OR tc.TABLE_SCHEMA = ?) \
    AND (? IS NULL OR tc.TABLE_NAME = ?) \
    ORDER BY tc.TABLE_SCHEMA, tc.TABLE_NAME, tc.CONSTRAINT_NAME, kcu.ORDINAL_POSITION";

pub const LIST_CONSTRAINTS_WITHOUT_CHECKS: &str = "SELECT tc.CONSTRAINT_NAME AS name, \
    tc.CONSTRAINT_TYPE AS kind, tc.TABLE_SCHEMA AS database_name, \
    tc.TABLE_NAME AS table_name, kcu.COLUMN_NAME AS column_name, \
    kcu.REFERENCED_TABLE_SCHEMA AS referenced_schema, \
    kcu.REFERENCED_TABLE_NAME AS referenced_table, \
    kcu.REFERENCED_COLUMN_NAME AS referenced_column, \
    rc.UPDATE_RULE AS on_update, rc.DELETE_RULE AS on_delete, \
    NULL AS check_expression \
    FROM information_schema.TABLE_CONSTRAINTS tc \
    JOIN ( \
        SELECT CONSTRAINT_NAME FROM information_schema.TABLE_CONSTRAINTS \
        WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
        AND (? IS NULL OR TABLE_NAME = ?) \
        GROUP BY CONSTRAINT_NAME ORDER BY CONSTRAINT_NAME LIMIT ? OFFSET ? \
    ) page ON page.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    LEFT JOIN information_schema.KEY_COLUMN_USAGE kcu \
    ON kcu.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
    AND kcu.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    AND kcu.TABLE_SCHEMA = tc.TABLE_SCHEMA \
    AND kcu.TABLE_NAME = tc.TABLE_NAME \
    LEFT JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
    ON rc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
    AND rc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
    AND rc.TABLE_NAME = tc.TABLE_NAME \
    WHERE (? IS NULL OR tc.TABLE_SCHEMA = ?) \
    AND (? IS NULL OR tc.TABLE_NAME = ?) \
    ORDER BY tc.TABLE_SCHEMA, tc.TABLE_NAME, tc.CONSTRAINT_NAME, kcu.ORDINAL_POSITION";

/// One table's row estimate plus the resolved database. `TABLE_ROWS` is an
/// engine estimate, never an exact count. The row browser labels it separately.
pub const ROW_TABLE_ESTIMATE: &str = "SELECT TABLE_SCHEMA AS database_name, \
    CAST(TABLE_ROWS AS SIGNED) AS estimated_rows \
    FROM information_schema.TABLES \
    WHERE TABLE_SCHEMA = COALESCE(?, DATABASE()) AND TABLE_NAME = ? \
    AND TABLE_TYPE = 'BASE TABLE' LIMIT 1";

/// Column metadata for one row-browser table, bounded by the caller's limit.
pub const ROW_TABLE_COLUMNS: &str = "SELECT TABLE_SCHEMA AS database_name, \
    COLUMN_NAME AS name, COLUMN_TYPE AS data_type, IS_NULLABLE = 'YES' AS nullable, \
    CAST(ORDINAL_POSITION AS SIGNED) AS ordinal_position, \
    COLUMN_KEY = 'PRI' AS primary_key, \
    EXTRA LIKE '%auto_increment%' AS auto_increment, \
    (EXTRA LIKE '%VIRTUAL GENERATED%' OR EXTRA LIKE '%STORED GENERATED%') AS `generated` \
    FROM information_schema.COLUMNS \
    WHERE TABLE_SCHEMA = COALESCE(?, DATABASE()) AND TABLE_NAME = ? \
    ORDER BY ORDINAL_POSITION LIMIT ?";

/// Unique indexes that can identify one row for editing. Prefix indexes are
/// returned so the adapter can reject them as incomplete identities.
pub const ROW_UNIQUE_INDEXES: &str = "SELECT INDEX_NAME AS index_name, \
    COLUMN_NAME AS column_name, CAST(SEQ_IN_INDEX AS SIGNED) AS ordinal_position, \
    SUB_PART AS sub_part \
    FROM information_schema.STATISTICS \
    WHERE TABLE_SCHEMA = COALESCE(?, DATABASE()) AND TABLE_NAME = ? \
    AND NON_UNIQUE = 0 AND INDEX_NAME <> 'PRIMARY' \
    ORDER BY INDEX_NAME, SEQ_IN_INDEX LIMIT ?";

pub const TABLE_INCOMING_FOREIGN_KEYS: &str = "SELECT kcu.CONSTRAINT_NAME AS name, \
    kcu.TABLE_SCHEMA AS database_name, kcu.TABLE_NAME AS table_name, \
    kcu.COLUMN_NAME AS column_name, kcu.REFERENCED_COLUMN_NAME AS referenced_column, \
    rc.UPDATE_RULE AS on_update, rc.DELETE_RULE AS on_delete \
    FROM information_schema.KEY_COLUMN_USAGE kcu \
    JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
    ON rc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA \
    AND rc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
    AND rc.TABLE_NAME = kcu.TABLE_NAME \
    WHERE kcu.REFERENCED_TABLE_SCHEMA = ? AND kcu.REFERENCED_TABLE_NAME = ? \
    ORDER BY kcu.TABLE_SCHEMA, kcu.TABLE_NAME, kcu.CONSTRAINT_NAME, \
    kcu.ORDINAL_POSITION LIMIT ?";

pub const TABLE_TRIGGERS: &str = "SELECT TRIGGER_NAME AS name, \
    TRIGGER_SCHEMA AS database_name, EVENT_OBJECT_TABLE AS table_name, \
    ACTION_TIMING AS timing, EVENT_MANIPULATION AS event \
    FROM information_schema.TRIGGERS \
    WHERE TRIGGER_SCHEMA = ? AND EVENT_OBJECT_TABLE = ? \
    ORDER BY TRIGGER_NAME LIMIT ?";

/// Lists accounts without selecting authentication columns or password
/// hashes. A MySQL role is inferred from `mysql.role_edges`; a role that has
/// never been granted to a member is therefore shown as an account until it is
/// used. `can_login` reflects `account_locked`.
pub const LIST_ROLES: &str = "SELECT u.User AS name, u.Host AS host, \
    CASE WHEN EXISTS ( \
        SELECT 1 FROM mysql.role_edges e \
        WHERE e.FROM_USER = u.User AND e.FROM_HOST = u.Host \
    ) THEN 1 ELSE 0 END AS is_role, \
    u.account_locked = 'N' AS can_login, \
    u.Super_priv = 'Y' AS superuser, \
    u.Create_priv = 'Y' AS create_database \
    FROM mysql.user u \
    ORDER BY u.User, u.Host LIMIT ? OFFSET ?";

pub const ROLE_SUMMARY: &str = "SELECT u.User AS name, u.Host AS host, \
    CASE WHEN EXISTS ( \
        SELECT 1 FROM mysql.role_edges e \
        WHERE e.FROM_USER = u.User AND e.FROM_HOST = u.Host \
    ) THEN 1 ELSE 0 END AS is_role, \
    u.account_locked = 'N' AS can_login, \
    u.Super_priv = 'Y' AS superuser, \
    u.Create_priv = 'Y' AS create_database \
    FROM mysql.user u \
    WHERE u.User = ? AND u.Host = ? LIMIT 1";

/// Global, database, and table privileges for one account. Column-level
/// privileges are deliberately excluded until the typed target model supports
/// columns; this query never reads password or authentication columns.
pub const ROLE_DIRECT_GRANTS: &str = "SELECT 'global' AS target_kind, \
    NULL AS database_name, NULL AS schema_name, NULL AS object_name, \
    PRIVILEGE_TYPE AS privilege, IS_GRANTABLE = 'YES' AS grantable \
    FROM information_schema.USER_PRIVILEGES \
    WHERE GRANTEE = CONCAT('''', ?, '''@''', ?, '''') \
    AND PRIVILEGE_TYPE <> 'GRANT OPTION' \
    UNION ALL \
    SELECT 'database', TABLE_SCHEMA, NULL, NULL, PRIVILEGE_TYPE, \
    IS_GRANTABLE = 'YES' \
    FROM information_schema.SCHEMA_PRIVILEGES \
    WHERE GRANTEE = CONCAT('''', ?, '''@''', ?, '''') \
    AND PRIVILEGE_TYPE <> 'GRANT OPTION' \
    UNION ALL \
    SELECT 'table', TABLE_SCHEMA, NULL, TABLE_NAME, PRIVILEGE_TYPE, \
    IS_GRANTABLE = 'YES' \
    FROM information_schema.TABLE_PRIVILEGES \
    WHERE GRANTEE = CONCAT('''', ?, '''@''', ?, '''') \
    AND PRIVILEGE_TYPE <> 'GRANT OPTION'";

pub const ROLE_MEMBERSHIPS: &str = "SELECT FROM_USER AS role_name, \
    FROM_HOST AS role_host FROM mysql.role_edges \
    WHERE TO_USER = ? AND TO_HOST = ? ORDER BY FROM_USER, FROM_HOST";

/// Active connections from the process list. `TIME` is the number of seconds
/// in the current command state; it is only used as query elapsed time when a
/// query is running. `INFO` can be NULL when the account lacks PROCESS or the
/// session is idle.
pub const LIST_SESSIONS: &str = "SELECT p.ID AS session_id, p.USER AS user_name, \
    p.HOST AS host, p.DB AS database_name, p.COMMAND AS command, \
    p.TIME AS elapsed_seconds, p.STATE AS state, p.INFO AS query_text, \
    (p.ID = CONNECTION_ID()) AS is_current \
    FROM information_schema.PROCESSLIST p \
    WHERE (? = 0 OR (p.COMMAND <> 'Sleep' AND p.INFO IS NOT NULL AND p.INFO <> '')) \
    ORDER BY p.ID LIMIT ? OFFSET ?";

pub const SESSION_SUMMARY: &str = "SELECT p.ID AS session_id, p.USER AS user_name, \
    p.HOST AS host, p.DB AS database_name, p.COMMAND AS command, \
    p.TIME AS elapsed_seconds, p.STATE AS state, p.INFO AS query_text, \
    (p.ID = CONNECTION_ID()) AS is_current \
    FROM information_schema.PROCESSLIST p WHERE p.ID = ? LIMIT 1";

/// MySQL does not accept a placeholder for the process id in KILL, so the
/// adapter appends a validated integer to this fixed prefix.
pub const CANCEL_QUERY: &str = "KILL QUERY";
pub const KILL_SESSION: &str = "KILL CONNECTION";

#[cfg(test)]
mod tests {
    use super::*;

    /// MySQL reports `DEFAULT_GENERATED` in EXTRA for timestamp and expression
    /// defaults. Those are writable default values, not generated columns, so a
    /// broad `%GENERATED%` match would make the row editor reject them.
    #[test]
    fn column_metadata_distinguishes_generated_columns_from_default_generated_values() {
        assert!(!LIST_COLUMNS.contains("EXTRA LIKE '%GENERATED%'"));
        assert!(!ROW_TABLE_COLUMNS.contains("EXTRA LIKE '%GENERATED%'"));
        assert!(LIST_COLUMNS.contains("VIRTUAL GENERATED"));
        assert!(LIST_COLUMNS.contains("STORED GENERATED"));
        assert!(ROW_TABLE_COLUMNS.contains("VIRTUAL GENERATED"));
        assert!(ROW_TABLE_COLUMNS.contains("STORED GENERATED"));
    }

    /// MySQL uses the database name for both database and table grants. The
    /// adapter must not publish a separate schema field: the request contract
    /// rejects one, and a synthetic schema made every live grant verification
    /// compare two different target shapes.
    #[test]
    fn table_grant_metadata_does_not_synthesize_a_schema() {
        assert!(ROLE_DIRECT_GRANTS.contains("SELECT 'table', TABLE_SCHEMA, NULL, TABLE_NAME"));
    }
}
