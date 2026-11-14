//! Typed, safe row insert, update, and delete.
//!
//! A write is only accepted for a table with a stable row identity. Values are
//! resolved against live column metadata, validated against the column kind, and
//! bound as parameters. The engine executes each write in a transaction,
//! verifies the affected row count, re-reads the row where the identity is
//! known, and commits only after verification. A failure rolls the transaction
//! back and reports that no rows were changed.
//!
//! `NULL`, an empty string, and `DEFAULT` are distinct modes. A `NULL` value
//! binds SQL NULL, an empty text value binds `''`, and `DEFAULT` lets the engine
//! apply the column default.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    dialect::{Identifier, SqlValue},
    errors::DatabaseError,
    rows::{
        RowColumn, RowColumnKind, RowIdentityValue, RowRecord, RowTableMetadata,
        postgres_parameter_cast,
    },
    traits::{ColumnIdentity, TableRef},
};

/// Maximum number of column values in one row write.
pub const MAX_ROW_WRITE_VALUES: usize = 256;
/// Maximum number of row identities in one scoped bulk delete.
pub const MAX_ROW_WRITE_TARGETS: usize = 200;
/// Maximum characters in one text-like row value.
pub const MAX_ROW_WRITE_TEXT_CHARACTERS: usize = 1_048_576;

/// How a field is supplied to a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowValueMode {
    /// Bind the supplied text value as a typed parameter.
    Value,
    /// Bind SQL NULL.
    Null,
    /// Let the engine apply the column default. Insert only.
    Default,
}

/// One typed field in an insert or update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowWriteValue {
    pub column: Identifier,
    pub mode: RowValueMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl RowWriteValue {
    fn validate_shape(&self) -> Result<(), DatabaseError> {
        match self.mode {
            RowValueMode::Value if self.value.is_none() => Err(DatabaseError::validation(
                "A value field requires a value. Use the null or default mode instead.",
            )),
            RowValueMode::Null | RowValueMode::Default if self.value.is_some() => Err(
                DatabaseError::validation("Null and default fields cannot also carry a value."),
            ),
            _ => Ok(()),
        }
    }
}

/// One insert request. Omitted columns use the engine default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowInsertRequest {
    pub table: TableRef,
    #[serde(default)]
    pub values: Vec<RowWriteValue>,
}

impl RowInsertRequest {
    pub fn validate(&self) -> Result<(), DatabaseError> {
        validate_values(&self.values, true)
    }
}

/// One update request addressed by a complete stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowUpdateRequest {
    pub table: TableRef,
    pub identity: Vec<RowIdentityValue>,
    pub values: Vec<RowWriteValue>,
    /// Original values for every changed column. A concurrent change makes the
    /// guarded update match zero rows and the transaction roll back.
    #[serde(default)]
    pub expected: Vec<RowIdentityValue>,
}

impl RowUpdateRequest {
    pub fn validate(&self) -> Result<(), DatabaseError> {
        validate_values(&self.values, false)?;
        if self.identity.is_empty() {
            return Err(DatabaseError::validation(
                "An update requires a complete row identity.",
            ));
        }
        Ok(())
    }
}

/// One scoped delete request. Every target is an explicit stable identity; a
/// table-wide or predicate-based delete is never accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowDeleteRequest {
    pub table: TableRef,
    pub targets: Vec<Vec<RowIdentityValue>>,
}

impl RowDeleteRequest {
    pub fn validate(&self) -> Result<(), DatabaseError> {
        if self.targets.is_empty() {
            return Err(DatabaseError::validation(
                "A delete requires at least one row identity.",
            ));
        }
        if self.targets.len() > MAX_ROW_WRITE_TARGETS {
            return Err(DatabaseError::validation(format!(
                "A delete cannot target more than {MAX_ROW_WRITE_TARGETS} rows."
            )));
        }
        for target in &self.targets {
            if target.is_empty() {
                return Err(DatabaseError::validation(
                    "Every delete target requires a complete row identity.",
                ));
            }
        }
        Ok(())
    }
}

fn validate_values(values: &[RowWriteValue], allow_default: bool) -> Result<(), DatabaseError> {
    if values.len() > MAX_ROW_WRITE_VALUES {
        return Err(DatabaseError::validation(format!(
            "A row write cannot contain more than {MAX_ROW_WRITE_VALUES} values."
        )));
    }
    let mut seen = HashSet::new();
    for value in values {
        value.validate_shape()?;
        if value.mode == RowValueMode::Default && !allow_default {
            return Err(DatabaseError::validation(
                "Update fields cannot use the default mode. Clear the field or set an explicit value.",
            ));
        }
        if !seen.insert(value.column.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Column {} appears more than once in the write.",
                value.column
            )));
        }
    }
    Ok(())
}

/// Which write produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowWriteOperation {
    Insert,
    Update,
    Delete,
}

impl RowWriteOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Insert => "insert_row",
            Self::Update => "update_row",
            Self::Delete => "delete_row",
        }
    }
}

/// Final transaction state reported to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowTransactionState {
    Committed,
    RolledBack,
}

/// A verified write result. A returned result is always committed; failures
/// return an error after an explicit rollback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowWriteResult {
    pub operation: RowWriteOperation,
    pub table: TableRef,
    pub affected_rows: u64,
    pub verified: bool,
    pub verification: String,
    pub transaction: RowTransactionState,
    /// Re-read row when the identity is known. It contains only the same
    /// bounded, display-safe cells the data grid already reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<RowRecord>,
}

impl RowWriteResult {
    pub fn committed(
        operation: RowWriteOperation,
        table: TableRef,
        affected_rows: u64,
        verification: String,
        row: Option<RowRecord>,
    ) -> Self {
        Self {
            operation,
            table,
            affected_rows,
            verified: true,
            verification,
            transaction: RowTransactionState::Committed,
            row,
        }
    }
}

/// A resolved insert value. `text` is the canonical display text used for
/// verification; it is never written to history.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedWriteValue {
    Null,
    Default,
    Value(SqlValue),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedValue {
    pub column: usize,
    pub value: ResolvedWriteValue,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedInsert {
    pub values: Vec<ResolvedValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedExpected {
    pub column: usize,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedUpdate {
    pub identity: Vec<(usize, SqlValue, String)>,
    pub values: Vec<ResolvedValue>,
    pub expected: Vec<ResolvedExpected>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDelete {
    pub targets: Vec<Vec<(usize, SqlValue, String)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WritePhase {
    Insert,
    Update,
}

fn column_by_name<'a>(
    metadata: &'a RowTableMetadata,
    name: &Identifier,
) -> Result<(usize, &'a RowColumn), DatabaseError> {
    metadata
        .columns
        .iter()
        .position(|column| &column.name == name)
        .map(|index| (index, &metadata.columns[index]))
        .ok_or_else(|| {
            DatabaseError::validation(format!(
                "Column {name} does not exist in table {}.",
                metadata.table
            ))
        })
}

fn require_identity(metadata: &RowTableMetadata) -> Result<(), DatabaseError> {
    if !metadata.identity.stable || metadata.identity.columns.is_empty() {
        return Err(DatabaseError::validation(
            "This table has no stable primary key or complete unique index, so rows cannot be \
             edited safely.",
        ));
    }
    Ok(())
}

/// Resolves one insert request against live column metadata.
pub fn resolve_insert(
    request: &RowInsertRequest,
    metadata: &RowTableMetadata,
) -> Result<ResolvedInsert, DatabaseError> {
    require_identity(metadata)?;
    request.validate()?;

    let mut values = Vec::with_capacity(request.values.len());
    for requested in &request.values {
        let (index, column) = column_by_name(metadata, &requested.column)?;
        let resolved = resolve_write_value(column, requested, WritePhase::Insert)?;
        values.push(ResolvedValue {
            column: index,
            value: resolved.0,
            text: resolved.1,
        });
    }

    Ok(ResolvedInsert { values })
}

/// Resolves one update request, including its identity and conflict guards.
pub fn resolve_update(
    request: &RowUpdateRequest,
    metadata: &RowTableMetadata,
) -> Result<ResolvedUpdate, DatabaseError> {
    require_identity(metadata)?;
    request.validate()?;
    if request.values.is_empty() {
        return Err(DatabaseError::validation(
            "An update requires at least one changed field.",
        ));
    }

    let identity = resolve_identity(metadata, &request.identity)?;
    let identity_names = identity
        .iter()
        .map(|(index, _, _)| metadata.columns[*index].name.as_str())
        .collect::<HashSet<_>>();
    let identity_count = identity.len();

    let mut values = Vec::with_capacity(request.values.len());
    let mut value_names = HashSet::new();
    for requested in &request.values {
        let (index, column) = column_by_name(metadata, &requested.column)?;
        if identity_names.contains(column.name.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Column {} is part of the row identity and cannot be updated. Delete the row and \
                 insert a replacement instead.",
                column.name
            )));
        }
        if !value_names.insert(column.name.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Column {} appears more than once in the update.",
                column.name
            )));
        }
        let resolved = resolve_write_value(column, requested, WritePhase::Update)?;
        values.push(ResolvedValue {
            column: index,
            value: resolved.0,
            text: resolved.1,
        });
    }

    if request.expected.len() != values.len() {
        return Err(DatabaseError::validation(
            "Every changed column requires its original value for conflict detection.",
        ));
    }
    let mut expected = Vec::with_capacity(request.expected.len());
    let mut expected_names = HashSet::new();
    for requested in &request.expected {
        let (index, column) = column_by_name(metadata, &requested.column)?;
        if !expected_names.insert(column.name.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Column {} appears more than once in the conflict checks.",
                column.name
            )));
        }
        if !value_names.contains(column.name.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Column {} has a conflict check but is not being updated.",
                column.name
            )));
        }
        expected.push(ResolvedExpected {
            column: index,
            value: requested.value.clone(),
        });
    }
    if !value_names.iter().all(|name| expected_names.contains(name)) {
        return Err(DatabaseError::validation(
            "Every changed column requires its original value for conflict detection.",
        ));
    }
    debug_assert_eq!(identity_count, identity.len());
    debug_assert_eq!(values.len(), expected.len());

    Ok(ResolvedUpdate {
        identity,
        values,
        expected,
    })
}

/// Resolves one scoped delete request.
pub fn resolve_delete(
    request: &RowDeleteRequest,
    metadata: &RowTableMetadata,
) -> Result<ResolvedDelete, DatabaseError> {
    require_identity(metadata)?;
    request.validate()?;

    let mut targets = Vec::with_capacity(request.targets.len());
    let mut seen = HashSet::new();
    for target in &request.targets {
        let resolved = resolve_identity(metadata, target)?;
        let key = resolved
            .iter()
            .map(|(index, _, text)| format!("{index}:{text}"))
            .collect::<Vec<_>>()
            .join("|");
        if !seen.insert(key) {
            return Err(DatabaseError::validation(
                "A delete target appears more than once.",
            ));
        }
        targets.push(resolved);
    }

    Ok(ResolvedDelete { targets })
}

/// Resolves one raw identity value set for an internal lookup after an insert.
pub(crate) fn resolve_identity_for_lookup(
    metadata: &RowTableMetadata,
    values: &[RowIdentityValue],
) -> Result<Vec<(usize, SqlValue, String)>, DatabaseError> {
    resolve_identity(metadata, values)
}

/// Parses one display text value for a column. Used by engine builders that
/// need a typed expectation value for a conflict guard.
pub(crate) fn resolve_text_value(
    column: &RowColumn,
    value: &str,
) -> Result<(SqlValue, String), DatabaseError> {
    parse_value(column, value)
}

fn resolve_identity(
    metadata: &RowTableMetadata,
    values: &[RowIdentityValue],
) -> Result<Vec<(usize, SqlValue, String)>, DatabaseError> {
    if values.len() != metadata.identity.columns.len() {
        return Err(DatabaseError::validation(
            "The row identity does not match this table's stable identity.",
        ));
    }
    let expected_names = metadata
        .identity
        .columns
        .iter()
        .map(Identifier::as_str)
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut by_name = Vec::with_capacity(values.len());
    for value in values {
        if !seen.insert(value.column.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Identity column {} appears more than once.",
                value.column
            )));
        }
        if !expected_names.contains(value.column.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Column {} is not part of this table's row identity.",
                value.column
            )));
        }
        let (index, column) = column_by_name(metadata, &value.column)?;
        let Some(text) = value.value.as_deref() else {
            return Err(DatabaseError::validation(
                "A stable row identity cannot contain NULL.",
            ));
        };
        let (sql, canonical) = parse_value(column, text)?;
        by_name.push((index, sql, canonical));
    }

    // Return in the declared identity order regardless of request order.
    let mut ordered = Vec::with_capacity(by_name.len());
    for name in &metadata.identity.columns {
        let entry = by_name
            .iter()
            .find(|(index, _, _)| &metadata.columns[*index].name == name)
            .cloned()
            .ok_or_else(|| {
                DatabaseError::validation("The row identity is missing a stable column.")
            })?;
        ordered.push(entry);
    }
    Ok(ordered)
}

fn resolve_write_value(
    column: &RowColumn,
    requested: &RowWriteValue,
    phase: WritePhase,
) -> Result<(ResolvedWriteValue, Option<String>), DatabaseError> {
    if column.generated {
        return Err(DatabaseError::validation(format!(
            "Column {} is generated and cannot be written.",
            column.name
        )));
    }
    match requested.mode {
        RowValueMode::Default => {
            if phase == WritePhase::Update {
                return Err(DatabaseError::validation(
                    "Update fields cannot use the default mode.",
                ));
            }
            Ok((ResolvedWriteValue::Default, None))
        }
        RowValueMode::Null => {
            if column.identity == ColumnIdentity::Always {
                return Err(DatabaseError::validation(format!(
                    "Identity column {} cannot be set to NULL.",
                    column.name
                )));
            }
            if !column.nullable {
                return Err(DatabaseError::validation(format!(
                    "Column {} is not nullable.",
                    column.name
                )));
            }
            Ok((ResolvedWriteValue::Null, None))
        }
        RowValueMode::Value => {
            if column.identity == ColumnIdentity::Always {
                return Err(DatabaseError::validation(format!(
                    "Column {} is generated always and cannot receive an explicit value.",
                    column.name
                )));
            }
            let text = requested.value.as_deref().ok_or_else(|| {
                DatabaseError::validation(format!("Column {} requires a value.", column.name))
            })?;
            let (sql, canonical) = parse_value(column, text)?;
            Ok((ResolvedWriteValue::Value(sql), Some(canonical)))
        }
    }
}

fn parse_value(column: &RowColumn, value: &str) -> Result<(SqlValue, String), DatabaseError> {
    if value.chars().count() > MAX_ROW_WRITE_TEXT_CHARACTERS {
        return Err(DatabaseError::validation(format!(
            "A value for column {} cannot exceed {MAX_ROW_WRITE_TEXT_CHARACTERS} characters.",
            column.name
        )));
    }
    if value.contains('\0') {
        return Err(DatabaseError::validation(format!(
            "A value for column {} cannot contain NUL characters.",
            column.name
        )));
    }

    let invalid = || {
        DatabaseError::validation(format!(
            "The value for column {} does not match its {} type.",
            column.name, column.data_type
        ))
    };

    match column.kind {
        RowColumnKind::Text | RowColumnKind::Temporal => {
            Ok((SqlValue::Text(value.to_string()), value.to_string()))
        }
        RowColumnKind::Integer => {
            let parsed = value.trim().parse::<i64>().map_err(|_| invalid())?;
            Ok((SqlValue::Integer(parsed), parsed.to_string()))
        }
        RowColumnKind::Float => {
            let parsed = value.trim().parse::<f64>().map_err(|_| invalid())?;
            if !parsed.is_finite() {
                return Err(invalid());
            }
            Ok((SqlValue::Float(parsed), parsed.to_string()))
        }
        RowColumnKind::Decimal => {
            if !value
                .trim()
                .parse::<f64>()
                .map(|parsed| parsed.is_finite())
                .unwrap_or(false)
            {
                return Err(invalid());
            }
            Ok((
                SqlValue::Text(value.trim().to_string()),
                value.trim().to_string(),
            ))
        }
        RowColumnKind::Boolean => {
            let parsed = match value.trim().to_ascii_lowercase().as_str() {
                "true" | "t" | "1" | "yes" => true,
                "false" | "f" | "0" | "no" => false,
                _ => return Err(invalid()),
            };
            Ok((SqlValue::Boolean(parsed), parsed.to_string()))
        }
        RowColumnKind::Uuid => {
            let parsed = uuid::Uuid::parse_str(value.trim()).map_err(|_| invalid())?;
            Ok((SqlValue::Text(parsed.to_string()), parsed.to_string()))
        }
        RowColumnKind::Json => {
            serde_json::from_str::<serde_json::Value>(value).map_err(|_| invalid())?;
            Ok((SqlValue::Text(value.to_string()), value.to_string()))
        }
        RowColumnKind::Binary => {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(value.trim())
                .map_err(|_| invalid())?;
            Ok((SqlValue::Bytes(bytes), value.trim().to_string()))
        }
        RowColumnKind::Other => Err(DatabaseError::validation(format!(
            "Column {} has a type that cannot be edited safely in the data grid.",
            column.name
        ))),
    }
}

/// Compares a resolved write value with a re-read display value. `None` means
/// the display representation is not comparable (for example temporal values
/// formatted by the engine), so the caller relies on the affected-row check.
pub fn text_matches_column(column: &RowColumn, expected: &str, actual: &str) -> Option<bool> {
    match column.kind {
        RowColumnKind::Text => Some(expected == actual),
        RowColumnKind::Integer => {
            Some(expected.parse::<i64>().ok()? == actual.parse::<i64>().ok()?)
        }
        RowColumnKind::Float | RowColumnKind::Decimal => {
            let left = expected.parse::<f64>().ok()?;
            let right = actual.parse::<f64>().ok()?;
            Some(left == right)
        }
        RowColumnKind::Boolean => {
            let parse = |value: &str| match value.trim().to_ascii_lowercase().as_str() {
                "true" | "t" | "1" | "yes" => Some(true),
                "false" | "f" | "0" | "no" => Some(false),
                _ => None,
            };
            Some(parse(expected)? == parse(actual)?)
        }
        RowColumnKind::Uuid => Some(expected.trim().eq_ignore_ascii_case(actual.trim())),
        RowColumnKind::Json => {
            let left = serde_json::from_str::<serde_json::Value>(expected).ok()?;
            let right = serde_json::from_str::<serde_json::Value>(actual).ok()?;
            Some(left == right)
        }
        RowColumnKind::Temporal | RowColumnKind::Binary | RowColumnKind::Other => None,
    }
}

/// PostgreSQL needs an explicit cast for typed write parameters. The cast is a
/// fixed constant selected from the engine-reported type family.
pub(crate) fn postgres_write_cast(column: &RowColumn) -> Option<&'static str> {
    if let Some(cast) = postgres_parameter_cast(&column.data_type) {
        return Some(cast);
    }
    match column.kind {
        RowColumnKind::Integer => Some("bigint"),
        RowColumnKind::Float => Some("double precision"),
        RowColumnKind::Boolean => Some("boolean"),
        RowColumnKind::Json => Some("jsonb"),
        _ => None,
    }
}

/// Adds explicit rollback context to an execution failure. The original error
/// kind is preserved so the UI can still distinguish permission, conflict, and
/// timeout failures.
pub(crate) fn rolled_back(error: DatabaseError) -> DatabaseError {
    let details = match error.details {
        Some(details) if !details.trim().is_empty() => {
            format!("{details} The transaction was rolled back; no rows were changed.")
        }
        _ => "The transaction was rolled back; no rows were changed.".to_string(),
    };
    DatabaseError {
        details: Some(details),
        ..error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{
        rows::{RowIdentity, RowTableMetadata},
        traits::{ColumnIdentity, MetadataValue},
    };
    use crate::storage::DatabaseEngine;

    fn identifier(value: &str) -> Identifier {
        Identifier::new(value).expect("identifier")
    }

    fn column(
        name: &str,
        data_type: &str,
        nullable: bool,
        primary: bool,
        identity: ColumnIdentity,
    ) -> RowColumn {
        RowColumn::new(
            identifier(name),
            data_type.to_string(),
            nullable,
            1,
            primary,
            identity,
            false,
        )
    }

    fn metadata() -> RowTableMetadata {
        RowTableMetadata {
            engine: DatabaseEngine::Postgresql,
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: identifier("users"),
            columns: vec![
                column("id", "bigint", false, true, ColumnIdentity::ByDefault),
                column("name", "text", false, false, ColumnIdentity::None),
                column("note", "text", true, false, ColumnIdentity::None),
                column(
                    "created_at",
                    "timestamp with time zone",
                    false,
                    false,
                    ColumnIdentity::None,
                ),
            ],
            identity: RowIdentity::primary_key(vec![identifier("id")]),
            estimated_rows: MetadataValue::available(1),
        }
    }

    fn table() -> TableRef {
        TableRef::new(Some("app"), Some("public"), "users").expect("table")
    }

    fn value(column: &str, mode: RowValueMode, value: Option<&str>) -> RowWriteValue {
        RowWriteValue {
            column: identifier(column),
            mode,
            value: value.map(ToString::to_string),
        }
    }

    #[test]
    fn insert_distinguishes_null_empty_and_default() {
        let request = RowInsertRequest {
            table: table(),
            values: vec![
                value("name", RowValueMode::Value, Some("")),
                value("note", RowValueMode::Null, None),
                value("created_at", RowValueMode::Default, None),
            ],
        };
        let resolved = resolve_insert(&request, &metadata()).expect("resolved insert");
        assert_eq!(
            resolved.values[0].value,
            ResolvedWriteValue::Value(SqlValue::Text(String::new()))
        );
        assert_eq!(resolved.values[1].value, ResolvedWriteValue::Null);
        assert_eq!(resolved.values[2].value, ResolvedWriteValue::Default);
    }

    #[test]
    fn insert_rejects_generated_identity_values_and_unknown_columns() {
        let mut always_identity = metadata();
        always_identity.columns[0].identity = ColumnIdentity::Always;
        let request = RowInsertRequest {
            table: table(),
            values: vec![value("id", RowValueMode::Value, Some("7"))],
        };
        assert!(resolve_insert(&request, &always_identity).is_err());

        let request = RowInsertRequest {
            table: table(),
            values: vec![value("missing", RowValueMode::Value, Some("x"))],
        };
        assert!(resolve_insert(&request, &metadata()).is_err());
    }

    #[test]
    fn update_requires_a_complete_identity_and_original_values() {
        let identity = vec![RowIdentityValue {
            column: identifier("id"),
            value: Some("7".to_string()),
        }];
        let request = RowUpdateRequest {
            table: table(),
            identity: identity.clone(),
            values: vec![value("name", RowValueMode::Value, Some("Ann"))],
            expected: vec![RowIdentityValue {
                column: identifier("name"),
                value: Some("ann".to_string()),
            }],
        };
        let resolved = resolve_update(&request, &metadata()).expect("resolved update");
        assert_eq!(resolved.identity[0].1, SqlValue::Integer(7));
        assert_eq!(resolved.expected[0].value.as_deref(), Some("ann"));

        let missing_expected = RowUpdateRequest {
            table: table(),
            identity: identity.clone(),
            values: vec![value("name", RowValueMode::Value, Some("Ann"))],
            expected: Vec::new(),
        };
        assert!(resolve_update(&missing_expected, &metadata()).is_err());

        let identity_change = RowUpdateRequest {
            table: table(),
            identity,
            values: vec![value("id", RowValueMode::Value, Some("8"))],
            expected: vec![RowIdentityValue {
                column: identifier("id"),
                value: Some("7".to_string()),
            }],
        };
        assert!(resolve_update(&identity_change, &metadata()).is_err());
    }

    #[test]
    fn update_rejects_default_mode_and_null_for_non_nullable_columns() {
        let identity = vec![RowIdentityValue {
            column: identifier("id"),
            value: Some("7".to_string()),
        }];
        let default = RowUpdateRequest {
            table: table(),
            identity: identity.clone(),
            values: vec![value("name", RowValueMode::Default, None)],
            expected: vec![RowIdentityValue {
                column: identifier("name"),
                value: Some("ann".to_string()),
            }],
        };
        assert!(resolve_update(&default, &metadata()).is_err());

        let null = RowUpdateRequest {
            table: table(),
            identity,
            values: vec![value("name", RowValueMode::Null, None)],
            expected: vec![RowIdentityValue {
                column: identifier("name"),
                value: Some("ann".to_string()),
            }],
        };
        assert!(resolve_update(&null, &metadata()).is_err());
    }

    #[test]
    fn delete_uses_only_complete_unique_targets() {
        let target = vec![RowIdentityValue {
            column: identifier("id"),
            value: Some("7".to_string()),
        }];
        let request = RowDeleteRequest {
            table: table(),
            targets: vec![target.clone(), target.clone()],
        };
        assert!(resolve_delete(&request, &metadata()).is_err());

        let request = RowDeleteRequest {
            table: table(),
            targets: vec![target],
        };
        let resolved = resolve_delete(&request, &metadata()).expect("resolved delete");
        assert_eq!(resolved.targets.len(), 1);
    }

    #[test]
    fn typed_values_are_validated_before_they_reach_sql() {
        let mut metadata = metadata();
        metadata.columns.push(column(
            "score",
            "integer",
            false,
            false,
            ColumnIdentity::None,
        ));
        metadata.columns.push(column(
            "active",
            "boolean",
            false,
            false,
            ColumnIdentity::None,
        ));
        metadata.columns.push(column(
            "payload",
            "jsonb",
            true,
            false,
            ColumnIdentity::None,
        ));

        let bad_int = RowInsertRequest {
            table: table(),
            values: vec![value(
                "score",
                RowValueMode::Value,
                Some("1; DROP TABLE users"),
            )],
        };
        assert!(resolve_insert(&bad_int, &metadata).is_err());

        let bad_bool = RowInsertRequest {
            table: table(),
            values: vec![value("active", RowValueMode::Value, Some("maybe"))],
        };
        assert!(resolve_insert(&bad_bool, &metadata).is_err());

        let bad_json = RowInsertRequest {
            table: table(),
            values: vec![value("payload", RowValueMode::Value, Some("{"))],
        };
        assert!(resolve_insert(&bad_json, &metadata).is_err());
    }

    #[test]
    fn a_table_without_stable_identity_rejects_every_write() {
        let mut metadata = metadata();
        metadata.identity = RowIdentity::none();
        let request = RowInsertRequest {
            table: table(),
            values: vec![value("name", RowValueMode::Value, Some("Ann"))],
        };
        assert!(resolve_insert(&request, &metadata).is_err());
    }

    #[test]
    fn rollback_context_preserves_the_typed_error_kind() {
        let error = rolled_back(DatabaseError::permission("denied"));
        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Permission);
        assert!(
            error
                .details
                .as_deref()
                .is_some_and(|details| details.contains("rolled back"))
        );
    }
}
