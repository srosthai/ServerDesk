//! Fixed MySQL metadata SQL.
//!
//! These statements are driver building blocks, not a command contract.
//! Dynamic values use `?` placeholders and identifiers are quoted by
//! [`super::MysqlDialect`]. The SQL strings are public so engine modules and
//! their tests can inspect them; Tauri commands and React never receive them.

pub const PING: &str = "SELECT 1";
pub const SERVER_VERSION: &str = "SELECT VERSION()";

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
    TABLE_TYPE AS table_type, TABLE_ROWS AS estimated_rows, \
    CAST(COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0) AS SIGNED) AS size_bytes, \
    ENGINE AS engine \
    FROM information_schema.TABLES \
    WHERE TABLE_TYPE = 'BASE TABLE' \
    AND (? IS NULL OR TABLE_SCHEMA = ?) \
    ORDER BY TABLE_SCHEMA, TABLE_NAME LIMIT ? OFFSET ?";

pub const LIST_VIEWS: &str = "SELECT TABLE_SCHEMA AS database_name, TABLE_NAME AS name, \
    DEFINER AS owner \
    FROM information_schema.VIEWS \
    WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
    ORDER BY TABLE_SCHEMA, TABLE_NAME LIMIT ? OFFSET ?";

pub const OBJECT_COUNTS: &str = "SELECT \
    CAST(COALESCE(SUM(CASE WHEN TABLE_TYPE = 'BASE TABLE' THEN 1 ELSE 0 END), 0) AS SIGNED) \
        AS table_count, \
    CAST(COALESCE(SUM(CASE WHEN TABLE_TYPE = 'VIEW' THEN 1 ELSE 0 END), 0) AS SIGNED) \
        AS view_count \
    FROM information_schema.TABLES \
    WHERE (? IS NULL OR TABLE_SCHEMA = ?)";

pub const LIST_COLUMNS: &str = "SELECT COLUMN_NAME AS name, COLUMN_TYPE AS data_type, \
    IS_NULLABLE = 'YES' AS nullable, COLUMN_DEFAULT AS default_value, \
    ORDINAL_POSITION AS ordinal_position, COLUMN_KEY = 'PRI' AS primary_key, \
    EXTRA LIKE '%GENERATED%' AS `generated`, COLUMN_COMMENT AS comment \
    FROM information_schema.COLUMNS \
    WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
    AND (? IS NULL OR TABLE_NAME = ?) \
    ORDER BY ORDINAL_POSITION";

pub const LIST_INDEXES: &str = "SELECT INDEX_NAME AS name, COLUMN_NAME AS column_name, \
    NON_UNIQUE = 0 AS unique_index, INDEX_NAME = 'PRIMARY' AS primary_index, \
    INDEX_TYPE AS method \
    FROM information_schema.STATISTICS \
    WHERE (? IS NULL OR TABLE_SCHEMA = ?) \
    AND (? IS NULL OR TABLE_NAME = ?) \
    ORDER BY INDEX_NAME, SEQ_IN_INDEX";

pub const LIST_CONSTRAINTS: &str = "SELECT tc.CONSTRAINT_NAME AS name, \
    tc.CONSTRAINT_TYPE AS kind, tc.TABLE_SCHEMA AS database_name, \
    tc.TABLE_NAME AS table_name, kcu.COLUMN_NAME AS column_name, \
    kcu.REFERENCED_TABLE_NAME AS referenced_table, \
    kcu.REFERENCED_COLUMN_NAME AS referenced_column, \
    rc.UPDATE_RULE AS on_update, rc.DELETE_RULE AS on_delete \
    FROM information_schema.TABLE_CONSTRAINTS tc \
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

pub const LIST_ROUTINES: &str = "SELECT ROUTINE_SCHEMA AS database_name, ROUTINE_NAME AS name, \
    ROUTINE_TYPE AS routine_type \
    FROM information_schema.ROUTINES \
    WHERE (? IS NULL OR ROUTINE_SCHEMA = ?) \
    ORDER BY ROUTINE_SCHEMA, ROUTINE_NAME";

pub const LIST_TRIGGERS: &str = "SELECT TRIGGER_SCHEMA AS database_name, TRIGGER_NAME AS name, \
    EVENT_OBJECT_TABLE AS table_name, EVENT_MANIPULATION AS event, ACTION_TIMING AS timing \
    FROM information_schema.TRIGGERS \
    WHERE (? IS NULL OR TRIGGER_SCHEMA = ?) \
    ORDER BY TRIGGER_SCHEMA, TRIGGER_NAME";

pub const LIST_ROLES: &str = "SELECT DISTINCT FROM_USER AS name FROM mysql.role_edges \
    ORDER BY FROM_USER";

pub const LIST_SESSIONS: &str = "SELECT ID AS session_id, USER AS user, HOST AS host, \
    DB AS database_name, COMMAND AS command, TIME AS elapsed_seconds, STATE AS state, \
    INFO AS query_text FROM information_schema.PROCESSLIST ORDER BY ID";

pub const CANCEL_QUERY: &str = "KILL QUERY ?";
pub const KILL_SESSION: &str = "KILL ?";
