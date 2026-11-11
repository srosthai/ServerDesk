//! Fixed PostgreSQL metadata SQL.
//!
//! These statements are driver building blocks, not a command contract.
//! Dynamic values use `$n` placeholders and identifiers are quoted by
//! [`super::PostgresDialect`]. Tauri commands and React never receive them.

pub const PING: &str = "SELECT 1";
pub const SERVER_VERSION: &str = "SELECT version()";

pub const LIST_DATABASES: &str = "SELECT datname AS name, \
    pg_catalog.pg_get_userbyid(datdba) AS owner, \
    pg_catalog.pg_encoding_to_char(encoding) AS encoding, datcollate AS collation, \
    pg_catalog.pg_database_size(datname) AS size_bytes \
    FROM pg_catalog.pg_database WHERE datallowconn ORDER BY datname";

pub const LIST_SCHEMAS: &str = "SELECT nspname AS name, \
    pg_catalog.pg_get_userbyid(nspowner) AS owner, \
    nspname IN ('pg_catalog', 'information_schema') OR nspname LIKE 'pg_toast%' AS system \
    FROM pg_catalog.pg_namespace ORDER BY nspname";

pub const LIST_TABLES: &str = "SELECT table_schema AS schema_name, table_name AS name, \
    table_type AS table_type \
    FROM information_schema.tables \
    WHERE ($1::text IS NULL OR table_schema = $1) \
    AND table_schema NOT IN ('pg_catalog', 'information_schema') \
    ORDER BY table_schema, table_name";

pub const LIST_VIEWS: &str = "SELECT table_schema AS schema_name, table_name AS name, \
    view_definition AS definition \
    FROM information_schema.views \
    WHERE ($1::text IS NULL OR table_schema = $1) \
    AND table_schema NOT IN ('pg_catalog', 'information_schema') \
    ORDER BY table_schema, table_name";

pub const LIST_MATERIALIZED_VIEWS: &str = "SELECT schemaname AS schema_name, \
    matviewname AS name, pg_catalog.pg_get_userbyid(matviewowner) AS owner \
    FROM pg_catalog.pg_matviews ORDER BY schemaname, matviewname";

pub const LIST_SEQUENCES: &str = "SELECT sequence_schema AS schema_name, sequence_name AS name, \
    data_type AS data_type, start_value AS start_value, increment AS increment_by \
    FROM information_schema.sequences \
    WHERE ($1::text IS NULL OR sequence_schema = $1) \
    ORDER BY sequence_schema, sequence_name";

pub const LIST_COLUMNS: &str = "SELECT column_name AS name, data_type AS data_type, \
    is_nullable = 'YES' AS nullable, column_default AS default_value, \
    ordinal_position AS ordinal_position, \
    EXISTS ( \
        SELECT 1 FROM information_schema.table_constraints tc \
        JOIN information_schema.key_column_usage kcu \
          ON tc.constraint_name = kcu.constraint_name \
         AND tc.table_schema = kcu.table_schema \
         AND tc.table_name = kcu.table_name \
        WHERE tc.constraint_type = 'PRIMARY KEY' \
          AND tc.table_schema = columns.table_schema \
          AND tc.table_name = columns.table_name \
          AND kcu.column_name = columns.column_name \
    ) AS primary_key, \
    is_generated = 'ALWAYS' AS generated, \
    is_identity = 'YES' AS identity \
    FROM information_schema.columns \
    WHERE ($1::text IS NULL OR table_schema = $1) \
    AND ($2::text IS NULL OR table_name = $2) \
    ORDER BY ordinal_position";

pub const LIST_INDEXES: &str = "SELECT indexname AS name, indexdef AS definition \
    FROM pg_catalog.pg_indexes \
    WHERE ($1::text IS NULL OR schemaname = $1) \
    AND ($2::text IS NULL OR tablename = $2) \
    ORDER BY schemaname, tablename, indexname";

pub const LIST_CONSTRAINTS: &str = "SELECT tc.constraint_name AS name, \
    tc.constraint_type AS kind, tc.table_schema AS schema_name, tc.table_name AS table_name \
    FROM information_schema.table_constraints tc \
    WHERE ($1::text IS NULL OR tc.table_schema = $1) \
    AND ($2::text IS NULL OR tc.table_name = $2) \
    ORDER BY tc.constraint_name";

pub const LIST_ROUTINES: &str = "SELECT routine_schema AS schema_name, routine_name AS name, \
    routine_type AS routine_type \
    FROM information_schema.routines \
    WHERE ($1::text IS NULL OR routine_schema = $1) \
    AND routine_schema NOT IN ('pg_catalog', 'information_schema') \
    ORDER BY routine_schema, routine_name";

pub const LIST_TRIGGERS: &str = "SELECT trigger_schema AS schema_name, trigger_name AS name, \
    event_object_table AS table_name, event_manipulation AS event, action_timing AS timing \
    FROM information_schema.triggers \
    WHERE ($1::text IS NULL OR trigger_schema = $1) \
    ORDER BY trigger_schema, trigger_name";

pub const LIST_ROLES: &str = "SELECT rolname AS name, rolsuper AS superuser, \
    rolcanlogin AS can_login, rolcreatedb AS create_database, rolcreaterole AS create_role \
    FROM pg_catalog.pg_roles ORDER BY rolname";

pub const LIST_SESSIONS: &str = "SELECT pid AS session_id, usename AS user, \
    client_addr::text AS host, datname AS database_name, state AS state, \
    EXTRACT(EPOCH FROM (now() - query_start))::bigint AS elapsed_seconds, \
    query AS query_text FROM pg_catalog.pg_stat_activity \
    WHERE pid <> pg_backend_pid() ORDER BY pid";

pub const CANCEL_QUERY: &str = "SELECT pg_catalog.pg_cancel_backend($1)";
pub const KILL_SESSION: &str = "SELECT pg_catalog.pg_terminate_backend($1)";
