//! Fixed PostgreSQL metadata SQL.
//!
//! These statements are driver building blocks, not a command contract.
//! Dynamic values use `$n` placeholders and identifiers are quoted by
//! [`super::PostgresDialect`]. Tauri commands and React never receive them.

pub const PING: &str = "SELECT 1";

pub const SERVER_INFO: &str = "SELECT version() AS version, current_database() AS database_name";

/// Applies a session-wide read-only default before the connection is reported
/// as usable. PostgreSQL rejects writes to non-temporary objects in every
/// subsequent transaction, including autocommit statements.
pub const SET_READ_ONLY: &str = "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY";

pub const LIST_DATABASES: &str = r#"SELECT datname AS name,
    pg_catalog.pg_get_userbyid(datdba) AS owner,
    pg_catalog.pg_encoding_to_char(encoding) AS encoding,
    datcollate AS collation,
    CASE WHEN pg_catalog.has_database_privilege(datname, 'CONNECT')
         THEN pg_catalog.pg_database_size(datname)
    END AS size_bytes
FROM pg_catalog.pg_database
WHERE datallowconn
ORDER BY datname
LIMIT $1 OFFSET $2"#;

pub const DATABASE_SUMMARY: &str = r#"SELECT datname AS name,
    pg_catalog.pg_get_userbyid(datdba) AS owner,
    pg_catalog.pg_encoding_to_char(encoding) AS encoding,
    datcollate AS collation,
    CASE WHEN pg_catalog.has_database_privilege(datname, 'CONNECT')
         THEN pg_catalog.pg_database_size(datname)
    END AS size_bytes
FROM pg_catalog.pg_database
WHERE datname = $1
LIMIT 1"#;

pub const LIST_SCHEMAS: &str = r#"SELECT nspname AS name,
    pg_catalog.pg_get_userbyid(nspowner) AS owner,
    (nspname IN ('information_schema', 'pg_catalog')
        OR nspname LIKE 'pg_toast%'
        OR nspname LIKE 'pg_temp%') AS system
FROM pg_catalog.pg_namespace
ORDER BY nspname
LIMIT $1 OFFSET $2"#;

pub const LIST_TABLES: &str = r#"SELECT current_database() AS database_name,
    n.nspname AS schema_name,
    c.relname AS name,
    CASE c.relkind
        WHEN 'r' THEN 'BASE TABLE'
        WHEN 'p' THEN 'PARTITIONED TABLE'
        WHEN 'f' THEN 'FOREIGN TABLE'
        ELSE 'OTHER'
    END AS table_type,
    pg_catalog.pg_get_userbyid(c.relowner) AS owner,
    CASE WHEN pg_catalog.has_table_privilege(c.oid, 'SELECT')
         THEN pg_catalog.pg_total_relation_size(c.oid)
    END AS size_bytes,
    CASE WHEN c.reltuples >= 0 THEN c.reltuples::bigint END AS estimated_rows,
    am.amname AS engine
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_catalog.pg_am am ON am.oid = c.relam
WHERE c.relkind IN ('r', 'p', 'f')
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND ($1::text IS NULL OR n.nspname = $1)
ORDER BY n.nspname, c.relname
LIMIT $2 OFFSET $3"#;

pub const LIST_VIEWS: &str = r#"SELECT current_database() AS database_name,
    n.nspname AS schema_name,
    c.relname AS name,
    CASE c.relkind WHEN 'm' THEN 'MATERIALIZED VIEW' ELSE 'VIEW' END AS view_type,
    pg_catalog.pg_get_userbyid(c.relowner) AS owner,
    CASE WHEN pg_catalog.has_table_privilege(c.oid, 'SELECT')
         THEN pg_catalog.pg_total_relation_size(c.oid)
    END AS size_bytes
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
WHERE (c.relkind = 'v' OR (c.relkind = 'm' AND $4::boolean))
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND ($1::text IS NULL OR n.nspname = $1)
ORDER BY n.nspname, c.relname
LIMIT $2 OFFSET $3"#;

pub const LIST_MATERIALIZED_VIEWS: &str = "SELECT schemaname AS schema_name, \
    matviewname AS name, pg_catalog.pg_get_userbyid(matviewowner) AS owner \
    FROM pg_catalog.pg_matviews ORDER BY schemaname, matviewname";

pub const LIST_SEQUENCES: &str = r#"SELECT current_database() AS database_name,
    s.sequence_schema AS schema_name,
    s.sequence_name AS name,
    s.data_type AS data_type,
    s.start_value::text AS start_value,
    s.increment::text AS increment_by,
    pg_catalog.pg_get_userbyid(c.relowner) AS owner,
    CASE WHEN pg_catalog.has_sequence_privilege(c.oid, 'SELECT')
         THEN pg_catalog.pg_relation_size(c.oid)
    END AS size_bytes
FROM information_schema.sequences s
JOIN pg_catalog.pg_namespace n ON n.nspname = s.sequence_schema
JOIN pg_catalog.pg_class c
  ON c.relnamespace = n.oid AND c.relname = s.sequence_name AND c.relkind = 'S'
WHERE ($1::text IS NULL OR s.sequence_schema = $1)
ORDER BY s.sequence_schema, s.sequence_name
LIMIT $2 OFFSET $3"#;

pub const OBJECT_COUNTS: &str = r#"SELECT
    (SELECT count(*)
     FROM pg_catalog.pg_class c
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
     WHERE c.relkind IN ('r', 'p', 'f')
       AND n.nspname NOT IN ('pg_catalog', 'information_schema')
       AND ($1::text IS NULL OR n.nspname = $1)) AS table_count,
    (SELECT count(*)
     FROM pg_catalog.pg_class c
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
     WHERE c.relkind = 'v'
       AND n.nspname NOT IN ('pg_catalog', 'information_schema')
       AND ($1::text IS NULL OR n.nspname = $1)) AS view_count,
    (SELECT count(*)
     FROM pg_catalog.pg_class c
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
     WHERE c.relkind = 'm'
       AND n.nspname NOT IN ('pg_catalog', 'information_schema')
       AND ($1::text IS NULL OR n.nspname = $1)) AS materialized_view_count,
    (SELECT count(*)
     FROM pg_catalog.pg_class c
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
     WHERE c.relkind = 'S'
       AND n.nspname NOT IN ('pg_catalog', 'information_schema')
       AND ($1::text IS NULL OR n.nspname = $1)) AS sequence_count"#;

pub const LIST_COLUMNS: &str = r#"SELECT a.attname AS name,
    pg_catalog.format_type(a.atttypid, a.atttypmod) AS data_type,
    NOT a.attnotnull AS nullable,
    pg_catalog.pg_get_expr(ad.adbin, ad.adrelid) AS default_value,
    a.attnum::bigint AS ordinal_position,
    EXISTS (
        SELECT 1
        FROM pg_catalog.pg_index i
        WHERE i.indrelid = a.attrelid
          AND i.indisprimary
          AND a.attnum = ANY (i.indkey)
    ) AS primary_key,
    COALESCE(row_to_json(a)->>'attgenerated', '') <> '' AS generated,
    COALESCE(row_to_json(a)->>'attidentity', '') <> '' AS identity,
    pg_catalog.col_description(a.attrelid, a.attnum) AS comment
FROM pg_catalog.pg_attribute a
JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_catalog.pg_attrdef ad
       ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
WHERE a.attnum > 0
  AND NOT a.attisdropped
  AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND ($1::text IS NULL OR n.nspname = $1)
  AND ($2::text IS NULL OR c.relname = $2)
ORDER BY a.attnum
LIMIT $3 OFFSET $4"#;

pub const LIST_INDEXES: &str = r#"SELECT i.relname AS name,
    ix.indisunique AS unique_index,
    ix.indisprimary AS primary_index,
    am.amname AS method,
    ARRAY(
        SELECT pg_catalog.pg_get_indexdef(ix.indexrelid, position, true)
        FROM generate_series(1, ix.indnatts) AS position
    ) AS columns
FROM pg_catalog.pg_index ix
JOIN pg_catalog.pg_class i ON i.oid = ix.indexrelid
JOIN pg_catalog.pg_class t ON t.oid = ix.indrelid
JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
JOIN pg_catalog.pg_am am ON am.oid = i.relam
WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND ($1::text IS NULL OR n.nspname = $1)
  AND ($2::text IS NULL OR t.relname = $2)
ORDER BY n.nspname, t.relname, i.relname
LIMIT $3 OFFSET $4"#;

pub const LIST_CONSTRAINTS: &str = r#"SELECT c.conname AS name,
    CASE c.contype
        WHEN 'p' THEN 'PRIMARY KEY'
        WHEN 'u' THEN 'UNIQUE'
        WHEN 'f' THEN 'FOREIGN KEY'
        WHEN 'c' THEN 'CHECK'
        ELSE 'OTHER'
    END AS kind,
    n.nspname AS schema_name,
    t.relname AS table_name,
    ARRAY(
        SELECT a.attname
        FROM unnest(c.conkey) WITH ORDINALITY AS key(attnum, position)
        JOIN pg_catalog.pg_attribute a
          ON a.attrelid = c.conrelid AND a.attnum = key.attnum
        ORDER BY key.position
    ) AS columns,
    rn.nspname AS referenced_schema,
    rt.relname AS referenced_table,
    CASE WHEN c.contype = 'f' THEN ARRAY(
        SELECT a.attname
        FROM unnest(c.confkey) WITH ORDINALITY AS key(attnum, position)
        JOIN pg_catalog.pg_attribute a
          ON a.attrelid = c.confrelid AND a.attnum = key.attnum
        ORDER BY key.position
    ) ELSE ARRAY[]::text[] END AS referenced_columns,
    CASE c.confupdtype
        WHEN 'a' THEN 'NO ACTION'
        WHEN 'r' THEN 'RESTRICT'
        WHEN 'c' THEN 'CASCADE'
        WHEN 'n' THEN 'SET NULL'
        WHEN 'd' THEN 'SET DEFAULT'
    END AS on_update,
    CASE c.confdeltype
        WHEN 'a' THEN 'NO ACTION'
        WHEN 'r' THEN 'RESTRICT'
        WHEN 'c' THEN 'CASCADE'
        WHEN 'n' THEN 'SET NULL'
        WHEN 'd' THEN 'SET DEFAULT'
    END AS on_delete
FROM pg_catalog.pg_constraint c
JOIN pg_catalog.pg_class t ON t.oid = c.conrelid
JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
LEFT JOIN pg_catalog.pg_class rt ON rt.oid = c.confrelid
LEFT JOIN pg_catalog.pg_namespace rn ON rn.oid = rt.relnamespace
WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND ($1::text IS NULL OR n.nspname = $1)
  AND ($2::text IS NULL OR t.relname = $2)
ORDER BY n.nspname, t.relname, c.conname
LIMIT $3 OFFSET $4"#;

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
