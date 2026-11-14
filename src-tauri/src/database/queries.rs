//! Arbitrary SQL execution for the SQL workspace.
//!
//! The adapter receives one SQL string from the editor, which may contain one
//! statement or a selection with several statements. Results are streamed from
//! the driver into a hard-bounded response: every result set is capped, cell
//! values are explicitly truncated, and rows beyond the limit are drained
//! without being stored. SQL errors are translated by the engine adapter into
//! the shared typed error taxonomy, including the engine code and, where the
//! server reports one, the statement position.
//!
//! Destructive-statement detection is intentionally conservative. It runs
//! before execution so a read-only session rejects writes and a destructive
//! script requires explicit confirmation. It is a safety aid, not a SQL parser
//! and not a replacement for database privileges.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::{core::AppError, storage::DatabaseEngine};

use super::{errors::DatabaseError, service::DatabaseConnectionRegistry, traits::DatabaseAdapter};

/// Default server-side bound for one SQL execution, including streaming.
pub const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Hard maximum server-side bound for one SQL execution.
pub const MAX_QUERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Default number of rows kept per result set.
pub const DEFAULT_QUERY_RESULT_LIMIT: u32 = 500;
/// Hard maximum number of rows kept per result set.
pub const MAX_QUERY_RESULT_LIMIT: u32 = 5_000;
/// Maximum number of result sets kept in one response.
pub const MAX_QUERY_RESULT_SETS: usize = 32;
/// Maximum columns kept per result set.
pub const MAX_QUERY_COLUMNS: usize = 512;
/// Maximum characters kept for one cell.
pub const MAX_QUERY_CELL_CHARACTERS: usize = 16_384;
/// Maximum characters kept across all stored cells in one response.
pub const MAX_QUERY_RESULT_CHARACTERS: usize = 4_000_000;
/// Maximum SQL text accepted by one execution request.
pub const MAX_QUERY_SQL_CHARACTERS: usize = 1_000_000;
/// Maximum length of a caller-generated cancellation id.
pub const MAX_QUERY_REQUEST_ID_LENGTH: usize = 128;

/// One validated SQL execution request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRequest {
    pub sql: String,
    pub max_rows: u32,
}

impl QueryRequest {
    /// Validates SQL and the per-result row limit before any driver call.
    pub fn try_new(sql: impl Into<String>, max_rows: Option<u32>) -> Result<Self, DatabaseError> {
        let sql = sql.into();
        if sql.trim().is_empty() {
            return Err(DatabaseError::validation(
                "Enter a SQL statement before executing it.",
            ));
        }
        if sql.chars().count() > MAX_QUERY_SQL_CHARACTERS {
            return Err(DatabaseError::validation(format!(
                "A SQL execution cannot exceed {MAX_QUERY_SQL_CHARACTERS} characters."
            )));
        }
        if sql.contains('\0') {
            return Err(DatabaseError::validation(
                "SQL text cannot contain NUL characters.",
            ));
        }

        let max_rows = max_rows.unwrap_or(DEFAULT_QUERY_RESULT_LIMIT);
        if !(1..=MAX_QUERY_RESULT_LIMIT).contains(&max_rows) {
            return Err(DatabaseError::validation(format!(
                "A query result limit must be between 1 and {MAX_QUERY_RESULT_LIMIT} rows."
            )));
        }

        Ok(Self { sql, max_rows })
    }

    /// Analyzes the request for the selected engine.
    pub fn analyze(&self, engine: DatabaseEngine) -> Result<SqlAnalysis, DatabaseError> {
        analyze_sql(engine, &self.sql)
    }
}

/// One column in a result set. Names are driver metadata, never SQL that is
/// executed again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryColumn {
    pub name: String,
    pub data_type: String,
}

/// One cell in a result grid. `None` is SQL NULL; `truncated` means the cell
/// was shortened at the configured character bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryCell {
    pub value: Option<String>,
    pub truncated: bool,
}

/// One statement result. A row-returning statement carries columns and rows; a
/// write or utility statement can carry only an affected-row count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResultSet {
    pub index: u32,
    pub columns: Vec<QueryColumn>,
    pub rows: Vec<Vec<QueryCell>>,
    pub returned_rows: u32,
    pub affected_rows: Option<u64>,
    pub last_insert_id: Option<u64>,
    /// More rows existed than the per-result limit allowed.
    pub truncated: bool,
    /// The statement returned more columns than the hard response bound.
    pub columns_truncated: bool,
}

/// Complete bounded result of one SQL execution request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryExecutionResult {
    pub engine: DatabaseEngine,
    pub read_only: bool,
    pub results: Vec<QueryResultSet>,
    pub warnings: Vec<SqlWarning>,
    pub destructive: bool,
    pub requires_confirmation: bool,
    /// At least one result set, column, cell, or row was bounded.
    pub truncated: bool,
    pub max_rows: u32,
    pub duration_ms: u64,
}

/// One warning produced by destructive-statement detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlWarningCode {
    Drop,
    Truncate,
    DeleteWithoutWhere,
    UpdateWithoutWhere,
    DeleteBroadPredicate,
    UpdateBroadPredicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlWarning {
    pub code: SqlWarningCode,
    /// One-based statement position inside the submitted SQL.
    pub statement_index: u32,
    pub message: String,
}

/// Pre-execution analysis used by the SQL workspace to warn and confirm before
/// sending anything to the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlAnalysis {
    pub engine: DatabaseEngine,
    pub statement_count: u32,
    /// True when the text contains a statement that may write data, schema, or
    /// session state. Read-only sessions reject these before execution.
    pub has_writes: bool,
    /// True when at least one statement can remove or broadly rewrite data.
    pub destructive: bool,
    pub requires_confirmation: bool,
    pub warnings: Vec<SqlWarning>,
}

/// Per-request timeout, cancellation, and result bounds.
#[derive(Debug, Clone)]
pub struct QueryControl {
    pub timeout: Duration,
    pub cancellation: Option<QueryCancellation>,
    pub max_rows: u32,
    pub max_result_sets: usize,
    pub max_columns: usize,
    pub max_cell_characters: usize,
    pub max_total_characters: usize,
}

impl Default for QueryControl {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_QUERY_TIMEOUT,
            cancellation: None,
            max_rows: DEFAULT_QUERY_RESULT_LIMIT,
            max_result_sets: MAX_QUERY_RESULT_SETS,
            max_columns: MAX_QUERY_COLUMNS,
            max_cell_characters: MAX_QUERY_CELL_CHARACTERS,
            max_total_characters: MAX_QUERY_RESULT_CHARACTERS,
        }
    }
}

impl QueryControl {
    pub fn new(timeout: Duration, cancellation: Option<QueryCancellation>, max_rows: u32) -> Self {
        Self {
            timeout: timeout.clamp(Duration::from_millis(1), MAX_QUERY_TIMEOUT),
            cancellation,
            max_rows: max_rows.clamp(1, MAX_QUERY_RESULT_LIMIT),
            ..Self::default()
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(QueryCancellation::is_cancelled)
    }

    pub async fn cancelled(&self) {
        match &self.cancellation {
            Some(cancellation) => cancellation.cancelled().await,
            None => std::future::pending().await,
        }
    }
}

/// Cancellation shared by the command layer and one in-flight SQL execution.
#[derive(Debug, Clone, Default)]
pub struct QueryCancellation {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl QueryCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.notify.notified().await;
    }
}

/// Resolves an optional timeout in milliseconds with a hard upper bound.
pub fn query_timeout(timeout_ms: Option<u64>) -> Duration {
    let milliseconds = timeout_ms
        .unwrap_or(DEFAULT_QUERY_TIMEOUT.as_millis() as u64)
        .clamp(1_000, MAX_QUERY_TIMEOUT.as_millis() as u64);
    Duration::from_millis(milliseconds)
}

/// Validates a caller-generated query request id used only as an in-memory
/// cancellation key. It is never sent to a database.
pub fn validate_query_request_id(value: &str) -> Result<(), DatabaseError> {
    if value.is_empty() || value.len() > MAX_QUERY_REQUEST_ID_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A query request id must contain between 1 and \
             {MAX_QUERY_REQUEST_ID_LENGTH} characters."
        )));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(DatabaseError::validation(
            "A query request id may contain only letters, digits, hyphens, underscores, and dots.",
        ));
    }
    Ok(())
}

type CancellationEntries = Arc<Mutex<HashMap<String, Arc<QueryCancellation>>>>;

/// Tracks the cancellation handle for each in-flight SQL execution.
#[derive(Debug, Clone, Default)]
pub struct QueryRegistry {
    entries: CancellationEntries,
}

impl QueryRegistry {
    pub fn begin(&self, request_id: &str) -> Result<QueryGuard, DatabaseError> {
        validate_query_request_id(request_id)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.contains_key(request_id) {
            return Err(DatabaseError::validation(
                "A SQL execution with this request id is already running.",
            ));
        }
        let cancellation = Arc::new(QueryCancellation::default());
        entries.insert(request_id.to_string(), Arc::clone(&cancellation));
        Ok(QueryGuard {
            request_id: request_id.to_string(),
            cancellation,
            entries: Arc::clone(&self.entries),
        })
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        if validate_query_request_id(request_id).is_err() {
            return false;
        }
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match entries.get(request_id) {
            Some(cancellation) => {
                cancellation.cancel();
                true
            }
            None => false,
        }
    }

    pub fn active_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// Removes the registry entry when the command finishes, including on error.
#[derive(Debug)]
pub struct QueryGuard {
    request_id: String,
    cancellation: Arc<QueryCancellation>,
    entries: CancellationEntries,
}

impl QueryGuard {
    pub fn cancellation(&self) -> QueryCancellation {
        QueryCancellation {
            cancelled: Arc::clone(&self.cancellation.cancelled),
            notify: Arc::clone(&self.cancellation.notify),
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

impl Drop for QueryGuard {
    fn drop(&mut self) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = entries.get(&self.request_id)
            && Arc::ptr_eq(current, &self.cancellation)
        {
            entries.remove(&self.request_id);
        }
    }
}

/// Stateless service that resolves one live adapter for a SQL execution.
pub struct DatabaseQueryService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseQueryService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn execute(
        &self,
        profile_id: &str,
        request: &QueryRequest,
        confirmed: bool,
        control: &QueryControl,
    ) -> Result<QueryExecutionResult, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        execute_query_with_adapter(adapter.as_ref(), request, confirmed, control)
            .await
            .map_err(AppError::from)
    }
}

/// Runs the shared safety gates and then asks the adapter for a bounded,
/// streamed execution. Tests use this path directly with the fake adapter.
pub async fn execute_query_with_adapter(
    adapter: &dyn DatabaseAdapter,
    request: &QueryRequest,
    confirmed: bool,
    control: &QueryControl,
) -> Result<QueryExecutionResult, DatabaseError> {
    let analysis = request.analyze(adapter.engine())?;

    if adapter.is_read_only() && analysis.has_writes {
        return Err(DatabaseError::permission(
            "The database connection profile is read-only; statements that write data, schema, \
             or session state are disabled.",
        )
        .with_details(
            "No statement was executed. Use a writable connection profile to run this SQL.",
        ));
    }

    if analysis.requires_confirmation && !confirmed {
        return Err(DatabaseError::cancelled(
            "The SQL contains a destructive statement and was cancelled because it was not \
             confirmed.",
        )
        .with_details(
            "No statement was executed. Review the warnings and confirm the destructive SQL \
             before running it.",
        ));
    }

    if control.is_cancelled() {
        return Err(DatabaseError::cancelled("The SQL execution was cancelled."));
    }

    let mut result = adapter.execute_query(request, control).await?;
    result.warnings = analysis.warnings;
    result.destructive = analysis.destructive;
    result.requires_confirmation = analysis.requires_confirmation;
    Ok(result)
}

/// Analyzes SQL text for the selected engine without executing it.
pub fn analyze_sql(engine: DatabaseEngine, sql: &str) -> Result<SqlAnalysis, DatabaseError> {
    if sql.trim().is_empty() {
        return Err(DatabaseError::validation(
            "Enter a SQL statement before analyzing it.",
        ));
    }
    if sql.chars().count() > MAX_QUERY_SQL_CHARACTERS {
        return Err(DatabaseError::validation(format!(
            "SQL text cannot exceed {MAX_QUERY_SQL_CHARACTERS} characters."
        )));
    }

    let statements = split_sql_statements(engine, sql)?;
    let mut warnings = Vec::new();
    let mut has_writes = false;
    let mut destructive = false;

    for (position, statement) in statements.iter().enumerate() {
        let statement_index = u32::try_from(position + 1).unwrap_or(u32::MAX);
        if statement_has_write(&statement.tokens) {
            has_writes = true;
        }

        let Some((write_position, keyword)) = first_top_level_write(&statement.tokens) else {
            continue;
        };
        match keyword {
            "drop" => {
                destructive = true;
                warnings.push(SqlWarning {
                    code: SqlWarningCode::Drop,
                    statement_index,
                    message: format!(
                        "Statement {statement_index} contains DROP and can permanently remove a \
                         database object."
                    ),
                });
            }
            "truncate" => {
                destructive = true;
                warnings.push(SqlWarning {
                    code: SqlWarningCode::Truncate,
                    statement_index,
                    message: format!(
                        "Statement {statement_index} contains TRUNCATE and removes every row from \
                         its target table."
                    ),
                });
            }
            "delete" => {
                if let Some(warning) = broad_data_change_warning(
                    &statement.tokens,
                    write_position,
                    statement_index,
                    true,
                ) {
                    destructive = true;
                    warnings.push(warning);
                }
            }
            "update" => {
                if let Some(warning) = broad_data_change_warning(
                    &statement.tokens,
                    write_position,
                    statement_index,
                    false,
                ) {
                    destructive = true;
                    warnings.push(warning);
                }
            }
            _ => {}
        }
    }

    Ok(SqlAnalysis {
        engine,
        statement_count: u32::try_from(statements.len()).unwrap_or(u32::MAX),
        has_writes,
        destructive,
        requires_confirmation: destructive,
        warnings,
    })
}

fn broad_data_change_warning(
    tokens: &[SqlToken],
    write_position: usize,
    statement_index: u32,
    delete: bool,
) -> Option<SqlWarning> {
    let where_position = find_top_level_word(tokens, "where", write_position + 1);
    match where_position {
        None => {
            let (code, verb) = if delete {
                (SqlWarningCode::DeleteWithoutWhere, "deletes")
            } else {
                (SqlWarningCode::UpdateWithoutWhere, "updates")
            };
            Some(SqlWarning {
                code,
                statement_index,
                message: format!(
                    "Statement {statement_index} {verb} rows without a WHERE clause and can affect \
                     every row in its target table."
                ),
            })
        }
        Some(position) if predicate_is_broad(tokens, position) => {
            let (code, verb) = if delete {
                (SqlWarningCode::DeleteBroadPredicate, "deletes")
            } else {
                (SqlWarningCode::UpdateBroadPredicate, "updates")
            };
            Some(SqlWarning {
                code,
                statement_index,
                message: format!(
                    "Statement {statement_index} {verb} rows with a predicate that is always true \
                     and can affect every row in its target table."
                ),
            })
        }
        Some(_) => None,
    }
}

fn predicate_is_broad(tokens: &[SqlToken], where_position: usize) -> bool {
    let predicate = predicate_tokens(tokens, where_position);
    matches!(
        predicate.as_slice(),
        [SqlToken::Word(value)] if value == "true"
    ) || matches!(
        predicate.as_slice(),
        [SqlToken::Word(left), SqlToken::Symbol('='), SqlToken::Word(right)]
            if (left == "1" && right == "1") || (left == "0" && right == "0")
    )
}

fn predicate_tokens(tokens: &[SqlToken], where_position: usize) -> Vec<SqlToken> {
    let mut depth = 0usize;
    let mut predicate = Vec::new();
    for token in tokens.iter().skip(where_position + 1) {
        if let SqlToken::Symbol(')') = token {
            depth = depth.saturating_sub(1);
        }
        if depth == 0 {
            if let SqlToken::Word(word) = token
                && matches!(
                    word.as_str(),
                    "returning" | "order" | "limit" | "offset" | "fetch" | "for"
                )
            {
                break;
            }
            predicate.push(token.clone());
        }
        if matches!(token, SqlToken::Symbol('(')) {
            depth += 1;
        }
    }
    predicate
}

fn first_top_level_write(tokens: &[SqlToken]) -> Option<(usize, &'static str)> {
    let mut depth = 0usize;
    for (position, token) in tokens.iter().enumerate() {
        if let SqlToken::Symbol(')') = token {
            depth = depth.saturating_sub(1);
        }
        if depth == 0
            && let SqlToken::Word(word) = token
            && let Some(keyword) = write_keyword(word)
        {
            return Some((position, keyword));
        }
        if matches!(token, SqlToken::Symbol('(')) {
            depth += 1;
        }
    }
    None
}

fn statement_has_write(tokens: &[SqlToken]) -> bool {
    if tokens
        .iter()
        .any(|token| matches!(token, SqlToken::Word(word) if write_keyword(word).is_some()))
    {
        return true;
    }
    matches!(
        tokens.first(),
        Some(SqlToken::Word(word)) if word == "select"
    ) && find_top_level_word(tokens, "into", 1).is_some()
}

fn find_top_level_word(tokens: &[SqlToken], needle: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (position, token) in tokens.iter().enumerate().skip(start) {
        if let SqlToken::Symbol(')') = token {
            depth = depth.saturating_sub(1);
        }
        if depth == 0 && matches!(token, SqlToken::Word(word) if word == needle) {
            return Some(position);
        }
        if matches!(token, SqlToken::Symbol('(')) {
            depth += 1;
        }
    }
    None
}

fn write_keyword(word: &str) -> Option<&'static str> {
    const WRITE_KEYWORDS: [&str; 30] = [
        "insert", "update", "delete", "merge", "create", "alter", "drop", "truncate", "grant",
        "revoke", "call", "do", "copy", "vacuum", "analyze", "reindex", "cluster", "refresh",
        "lock", "comment", "import", "security", "load", "rename", "set", "reset", "discard",
        "begin", "start", "commit",
    ];
    WRITE_KEYWORDS.into_iter().find(|keyword| *keyword == word)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SqlToken {
    Word(String),
    Symbol(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScannedStatement {
    pub(crate) text: String,
    pub(crate) tokens: Vec<SqlToken>,
}

/// Splits SQL into statements with comments and quoted text removed from the
/// token stream. Statement text is retained for engine-independent previews.
pub(crate) fn split_sql_statements(
    engine: DatabaseEngine,
    sql: &str,
) -> Result<Vec<ScannedStatement>, DatabaseError> {
    let characters: Vec<char> = sql.chars().collect();
    let mut statements = Vec::new();
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut statement_start = 0usize;
    let mut index = 0usize;

    #[derive(Debug)]
    enum Mode {
        Normal,
        SingleQuote,
        DoubleQuote,
        Backtick,
        LineComment,
        BlockComment(usize),
        DollarQuote(String),
    }

    let mut mode = Mode::Normal;

    while index < characters.len() {
        let character = characters[index];

        mode = match mode {
            Mode::Normal => {
                let next = characters.get(index + 1).copied();
                if character == '-' && next == Some('-') {
                    flush_word(&mut word, &mut tokens);
                    index += 2;
                    Mode::LineComment
                } else if engine == DatabaseEngine::Mysql && character == '#' {
                    flush_word(&mut word, &mut tokens);
                    index += 1;
                    Mode::LineComment
                } else if character == '/' && next == Some('*') {
                    flush_word(&mut word, &mut tokens);
                    index += 2;
                    Mode::BlockComment(1)
                } else if character == '\'' {
                    flush_word(&mut word, &mut tokens);
                    index += 1;
                    Mode::SingleQuote
                } else if character == '"' {
                    flush_word(&mut word, &mut tokens);
                    index += 1;
                    Mode::DoubleQuote
                } else if character == '`' {
                    flush_word(&mut word, &mut tokens);
                    index += 1;
                    Mode::Backtick
                } else if engine == DatabaseEngine::Postgresql
                    && character == '$'
                    && let Some((delimiter, next_index)) = dollar_quote(&characters, index)
                {
                    flush_word(&mut word, &mut tokens);
                    index = next_index;
                    Mode::DollarQuote(delimiter)
                } else if character == ';' {
                    flush_word(&mut word, &mut tokens);
                    if !tokens.is_empty() {
                        let text = characters[statement_start..index]
                            .iter()
                            .collect::<String>()
                            .trim()
                            .to_string();
                        statements.push(ScannedStatement {
                            text,
                            tokens: std::mem::take(&mut tokens),
                        });
                    }
                    statement_start = index + 1;
                    index += 1;
                    Mode::Normal
                } else if character.is_alphanumeric() || character == '_' {
                    word.push(character.to_ascii_lowercase());
                    index += 1;
                    Mode::Normal
                } else {
                    flush_word(&mut word, &mut tokens);
                    if !character.is_whitespace() {
                        tokens.push(SqlToken::Symbol(character));
                    }
                    index += 1;
                    Mode::Normal
                }
            }
            Mode::SingleQuote => {
                if character == '\\' && engine == DatabaseEngine::Mysql {
                    index = (index + 2).min(characters.len());
                    Mode::SingleQuote
                } else if character == '\'' {
                    if characters.get(index + 1) == Some(&'\'') {
                        index += 2;
                        Mode::SingleQuote
                    } else {
                        index += 1;
                        Mode::Normal
                    }
                } else {
                    index += 1;
                    Mode::SingleQuote
                }
            }
            Mode::DoubleQuote => {
                if character == '"' {
                    if characters.get(index + 1) == Some(&'"') {
                        index += 2;
                        Mode::DoubleQuote
                    } else {
                        index += 1;
                        Mode::Normal
                    }
                } else if character == '\\' && engine == DatabaseEngine::Mysql {
                    index = (index + 2).min(characters.len());
                    Mode::DoubleQuote
                } else {
                    index += 1;
                    Mode::DoubleQuote
                }
            }
            Mode::Backtick => {
                if character == '`' {
                    if characters.get(index + 1) == Some(&'`') {
                        index += 2;
                        Mode::Backtick
                    } else {
                        index += 1;
                        Mode::Normal
                    }
                } else {
                    index += 1;
                    Mode::Backtick
                }
            }
            Mode::LineComment => {
                if character == '\n' {
                    index += 1;
                    Mode::Normal
                } else {
                    index += 1;
                    Mode::LineComment
                }
            }
            Mode::BlockComment(mut depth) => {
                if character == '/'
                    && characters.get(index + 1) == Some(&'*')
                    && engine == DatabaseEngine::Postgresql
                {
                    depth += 1;
                    index += 2;
                    Mode::BlockComment(depth)
                } else if character == '*' && characters.get(index + 1) == Some(&'/') {
                    depth = depth.saturating_sub(1);
                    index += 2;
                    if depth == 0 {
                        Mode::Normal
                    } else {
                        Mode::BlockComment(depth)
                    }
                } else {
                    index += 1;
                    Mode::BlockComment(depth)
                }
            }
            Mode::DollarQuote(delimiter) => {
                if character == '$' && delimiter_matches(&characters, index, &delimiter) {
                    index += delimiter.chars().count();
                    Mode::Normal
                } else {
                    index += 1;
                    Mode::DollarQuote(delimiter)
                }
            }
        };
    }

    if !matches!(mode, Mode::Normal | Mode::LineComment) {
        return Err(DatabaseError::validation(
            "The SQL contains an unterminated quoted string or comment.",
        ));
    }

    flush_word(&mut word, &mut tokens);
    if !tokens.is_empty() {
        let text = characters[statement_start..]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        statements.push(ScannedStatement { text, tokens });
    }

    if statements.is_empty() {
        return Err(DatabaseError::validation(
            "Enter at least one SQL statement before executing it.",
        ));
    }

    Ok(statements)
}

fn flush_word(word: &mut String, tokens: &mut Vec<SqlToken>) {
    if !word.is_empty() {
        tokens.push(SqlToken::Word(std::mem::take(word)));
    }
}

fn dollar_quote(characters: &[char], start: usize) -> Option<(String, usize)> {
    let mut index = start + 1;
    while index < characters.len()
        && (characters[index].is_ascii_alphanumeric() || characters[index] == '_')
    {
        index += 1;
    }
    if characters.get(index) != Some(&'$') {
        return None;
    }
    let delimiter = characters[start..=index].iter().collect::<String>();
    Some((delimiter, index + 1))
}

fn delimiter_matches(characters: &[char], start: usize, delimiter: &str) -> bool {
    for (index, character) in (start..).zip(delimiter.chars()) {
        if characters.get(index) != Some(&character) {
            return false;
        }
    }
    true
}

/// Collects driver events into bounded result sets. Engines call it for both
/// MySQL and PostgreSQL so limits and truncation disclosure behave identically.
pub(crate) struct QueryResultCollector {
    results: Vec<QueryResultSet>,
    current: Option<QueryResultSet>,
    next_index: u32,
    current_discarded: bool,
    max_rows: u32,
    max_result_sets: usize,
    max_columns: usize,
    max_cell_characters: usize,
    max_total_characters: usize,
    total_characters: usize,
    truncated: bool,
}

impl QueryResultCollector {
    pub(crate) fn new(control: &QueryControl) -> Self {
        Self {
            results: Vec::new(),
            current: None,
            next_index: 0,
            current_discarded: false,
            max_rows: control.max_rows,
            max_result_sets: control.max_result_sets,
            max_columns: control.max_columns,
            max_cell_characters: control.max_cell_characters,
            max_total_characters: control.max_total_characters,
            total_characters: 0,
            truncated: false,
        }
    }

    pub(crate) fn push_row(
        &mut self,
        columns: Vec<QueryColumn>,
        values: Vec<Option<String>>,
    ) -> Result<(), DatabaseError> {
        if self.current_discarded {
            return Ok(());
        }

        if self.current.is_none() {
            if self.results.len() >= self.max_result_sets {
                self.truncated = true;
                self.current_discarded = true;
                return Ok(());
            }
            let mut columns = columns;
            let columns_truncated = columns.len() > self.max_columns;
            if columns_truncated {
                columns.truncate(self.max_columns);
                self.truncated = true;
            }
            self.current = Some(QueryResultSet {
                index: self.next_index,
                columns,
                rows: Vec::new(),
                returned_rows: 0,
                affected_rows: None,
                last_insert_id: None,
                truncated: false,
                columns_truncated,
            });
            self.next_index = self.next_index.saturating_add(1);
        }

        let set = self
            .current
            .as_mut()
            .ok_or_else(|| DatabaseError::internal("A result row arrived out of order."))?;

        if set.returned_rows >= self.max_rows {
            set.truncated = true;
            self.truncated = true;
            return Ok(());
        }
        if values.len() < set.columns.len() {
            return Err(DatabaseError::internal(
                "The SQL result returned fewer values than its column metadata.",
            ));
        }

        let mut cells = Vec::with_capacity(set.columns.len());
        let mut row_characters = 0usize;
        for value in values.into_iter().take(set.columns.len()) {
            let (value, truncated) = truncate_query_cell(value, self.max_cell_characters);
            if truncated {
                set.truncated = true;
                self.truncated = true;
            }
            row_characters = row_characters.saturating_add(
                value
                    .as_ref()
                    .map(|value| value.chars().count())
                    .unwrap_or(0),
            );
            cells.push(QueryCell { value, truncated });
        }

        if self.total_characters.saturating_add(row_characters) > self.max_total_characters {
            set.truncated = true;
            self.truncated = true;
            return Ok(());
        }
        self.total_characters = self.total_characters.saturating_add(row_characters);
        set.rows.push(cells);
        set.returned_rows = u32::try_from(set.rows.len()).unwrap_or(u32::MAX);
        Ok(())
    }

    pub(crate) fn push_command(&mut self, affected_rows: u64, last_insert_id: Option<u64>) {
        if self.current_discarded {
            self.current_discarded = false;
            return;
        }

        if let Some(mut set) = self.current.take() {
            set.affected_rows = Some(affected_rows);
            set.last_insert_id = last_insert_id.filter(|value| *value > 0);
            if set.truncated {
                self.truncated = true;
            }
            self.results.push(set);
            return;
        }

        if self.results.len() >= self.max_result_sets {
            self.truncated = true;
            return;
        }

        self.results.push(QueryResultSet {
            index: self.next_index,
            columns: Vec::new(),
            rows: Vec::new(),
            returned_rows: 0,
            affected_rows: Some(affected_rows),
            last_insert_id: last_insert_id.filter(|value| *value > 0),
            truncated: false,
            columns_truncated: false,
        });
        self.next_index = self.next_index.saturating_add(1);
    }

    /// Applies columns discovered by a describe call to the one empty read-only
    /// result set. This keeps an empty SELECT renderable as a grid.
    pub(crate) fn apply_described_columns(&mut self, columns: Vec<QueryColumn>) {
        if columns.is_empty() || self.results.len() != 1 {
            return;
        }
        let max_columns = self.max_columns;
        let Some(set) = self.results.first_mut() else {
            return;
        };
        if set.columns.is_empty() && set.rows.is_empty() {
            let mut columns = columns;
            if columns.len() > max_columns {
                columns.truncate(max_columns);
                set.columns_truncated = true;
                self.truncated = true;
            }
            set.columns = columns;
            set.affected_rows = None;
        }
    }

    pub(crate) fn finish(mut self) -> (Vec<QueryResultSet>, bool) {
        if let Some(set) = self.current.take() {
            if set.truncated {
                self.truncated = true;
            }
            self.results.push(set);
        }
        (self.results, self.truncated)
    }
}

fn truncate_query_cell(value: Option<String>, max_characters: usize) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    if value.chars().count() <= max_characters {
        return (Some(value), false);
    }
    (Some(value.chars().take(max_characters).collect()), true)
}

/// Encodes non-UTF-8 driver bytes for display without silently discarding data.
pub(crate) fn encode_binary(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(2 + bytes.len() * 2);
    encoded.push_str("\\x");
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(tokens: &[SqlToken]) -> Vec<&str> {
        tokens
            .iter()
            .filter_map(|token| match token {
                SqlToken::Word(word) => Some(word.as_str()),
                SqlToken::Symbol(_) => None,
            })
            .collect()
    }

    #[test]
    fn splitter_ignores_comments_strings_and_postgres_dollar_quotes() {
        let sql = "SELECT 'drop table users' AS text; -- DROP TABLE users\n\
                   SELECT 2; /* TRUNCATE users */ \
                   SELECT $tag$ DELETE FROM users $tag$;";
        let statements = split_sql_statements(DatabaseEngine::Postgresql, sql).expect("statements");
        assert_eq!(statements.len(), 3);
        assert_eq!(words(&statements[0].tokens), ["select", "as", "text"]);
        assert_eq!(words(&statements[1].tokens), ["select", "2"]);
        assert_eq!(words(&statements[2].tokens), ["select"]);
    }

    #[test]
    fn mysql_hash_comments_are_ignored_but_postgres_tokens_remain() {
        let mysql = split_sql_statements(DatabaseEngine::Mysql, "SELECT 1 # DROP TABLE users")
            .expect("mysql");
        assert_eq!(words(&mysql[0].tokens), ["select", "1"]);

        let postgres =
            split_sql_statements(DatabaseEngine::Postgresql, "SELECT 1 # DROP TABLE users")
                .expect("postgres");
        assert!(words(&postgres[0].tokens).contains(&"drop"));
    }

    #[test]
    fn detection_flags_drop_truncate_and_unqualified_dml() {
        let analysis = analyze_sql(
            DatabaseEngine::Postgresql,
            "DROP TABLE users; TRUNCATE users; DELETE FROM users; UPDATE users SET name = 'x';",
        )
        .expect("analysis");
        assert_eq!(analysis.statement_count, 4);
        assert!(analysis.has_writes);
        assert!(analysis.destructive);
        assert!(analysis.requires_confirmation);
        assert_eq!(analysis.warnings.len(), 4);
        assert_eq!(analysis.warnings[0].code, SqlWarningCode::Drop);
        assert_eq!(analysis.warnings[1].code, SqlWarningCode::Truncate);
        assert_eq!(
            analysis.warnings[2].code,
            SqlWarningCode::DeleteWithoutWhere
        );
        assert_eq!(
            analysis.warnings[3].code,
            SqlWarningCode::UpdateWithoutWhere
        );
    }

    #[test]
    fn detection_flags_tautological_predicates_but_not_scoped_changes() {
        let analysis = analyze_sql(
            DatabaseEngine::Mysql,
            "DELETE FROM users WHERE 1 = 1; UPDATE users SET active = 0 WHERE true; \
             DELETE FROM users WHERE id = 4; UPDATE users SET active = 1 WHERE id = 4;",
        )
        .expect("analysis");
        assert_eq!(analysis.warnings.len(), 2);
        assert_eq!(
            analysis.warnings[0].code,
            SqlWarningCode::DeleteBroadPredicate
        );
        assert_eq!(
            analysis.warnings[1].code,
            SqlWarningCode::UpdateBroadPredicate
        );
    }

    #[test]
    fn detection_distinguishes_reads_from_writes_and_select_into() {
        let read = analyze_sql(
            DatabaseEngine::Postgresql,
            "WITH x AS (SELECT 1) SELECT * FROM x; EXPLAIN SELECT * FROM users;",
        )
        .expect("read");
        assert!(!read.has_writes);
        assert!(!read.destructive);

        let write = analyze_sql(
            DatabaseEngine::Postgresql,
            "WITH x AS (INSERT INTO users(name) VALUES ('a') RETURNING id) SELECT * FROM x;",
        )
        .expect("write");
        assert!(write.has_writes);

        let select_into = analyze_sql(
            DatabaseEngine::Postgresql,
            "SELECT * INTO users_backup FROM users;",
        )
        .expect("select into");
        assert!(select_into.has_writes);

        let lock = analyze_sql(
            DatabaseEngine::Postgresql,
            "SELECT * FROM users FOR UPDATE;",
        )
        .expect("lock");
        assert!(lock.has_writes);
    }

    #[test]
    fn unterminated_quotes_are_rejected_before_execution() {
        let error = split_sql_statements(DatabaseEngine::Mysql, "SELECT 'unfinished")
            .expect_err("unterminated quote");
        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Validation);
    }

    #[test]
    fn result_collector_bounds_rows_columns_and_cells() {
        let control = QueryControl {
            max_rows: 2,
            max_columns: 2,
            max_cell_characters: 3,
            ..QueryControl::default()
        };
        let mut collector = QueryResultCollector::new(&control);
        let columns = vec![
            QueryColumn {
                name: "first".to_string(),
                data_type: "text".to_string(),
            },
            QueryColumn {
                name: "second".to_string(),
                data_type: "text".to_string(),
            },
            QueryColumn {
                name: "third".to_string(),
                data_type: "text".to_string(),
            },
        ];
        for value in ["abcdef", "ghijkl", "mnopqr"] {
            collector
                .push_row(
                    columns.clone(),
                    vec![
                        Some(value.to_string()),
                        Some("ok".to_string()),
                        Some("ignored".to_string()),
                    ],
                )
                .expect("row");
        }
        collector.push_command(3, Some(9));
        let (results, truncated) = collector.finish();

        assert!(truncated);
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.returned_rows, 2);
        assert!(result.truncated);
        assert!(result.columns_truncated);
        assert_eq!(result.affected_rows, Some(3));
        assert_eq!(result.last_insert_id, Some(9));
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.rows[0][0].value.as_deref(), Some("abc"));
        assert!(result.rows[0][0].truncated);
    }

    #[test]
    fn collector_keeps_command_results_and_multiple_result_sets() {
        let control = QueryControl::default();
        let mut collector = QueryResultCollector::new(&control);
        collector.push_command(2, None);
        collector.push_command(0, None);
        let (results, truncated) = collector.finish();

        assert!(!truncated);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 0);
        assert_eq!(results[0].affected_rows, Some(2));
        assert_eq!(results[1].index, 1);
        assert_eq!(results[1].affected_rows, Some(0));
    }

    #[test]
    fn request_limits_are_hard_bounds() {
        assert!(QueryRequest::try_new("SELECT 1", None).is_ok());
        assert!(QueryRequest::try_new("SELECT 1", Some(0)).is_err());
        assert!(QueryRequest::try_new("SELECT 1", Some(MAX_QUERY_RESULT_LIMIT + 1)).is_err());
        assert!(QueryRequest::try_new("  ", None).is_err());
        assert!(QueryRequest::try_new("SELECT 1\0", None).is_err());
    }

    #[test]
    fn timeout_and_request_ids_are_bounded() {
        assert_eq!(query_timeout(None), DEFAULT_QUERY_TIMEOUT);
        assert_eq!(query_timeout(Some(1)), Duration::from_secs(1));
        assert_eq!(query_timeout(Some(u64::MAX)), MAX_QUERY_TIMEOUT);
        assert!(validate_query_request_id("query-1").is_ok());
        assert!(validate_query_request_id("").is_err());
        assert!(validate_query_request_id("query;1").is_err());
    }

    #[test]
    fn registry_tracks_and_cleans_up_cancellation_handles() {
        let registry = QueryRegistry::default();
        let guard = registry.begin("query-1").expect("begin");
        assert_eq!(registry.active_count(), 1);
        assert!(registry.cancel("query-1"));
        assert!(guard.cancellation().is_cancelled());
        drop(guard);
        assert_eq!(registry.active_count(), 0);
        assert!(!registry.cancel("query-1"));
    }
}
