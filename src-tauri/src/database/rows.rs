//! Bounded, parameterized row browsing.
//!
//! Table data is never read wholesale. A row request names one table, visible
//! columns, filters, search terms, sort columns, and one page. The adapter
//! resolves every identifier against live column metadata, quotes it with the
//! engine dialect, and binds every filter/search value as a parameter.
//!
//! The model also records whether a table has a stable row identity. A primary
//! key or a complete, non-null unique index makes row editing possible; a table
//! without one is returned as read-only for editing.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::storage::DatabaseEngine;

use super::{
    dialect::{Identifier, ParameterizedQuery, SqlValue},
    errors::DatabaseError,
    traits::{ColumnIdentity, MetadataValue, TableRef},
};

/// Default number of rows in one data page.
pub const DEFAULT_ROW_PAGE_LIMIT: u32 = 100;
/// Hard maximum number of rows in one data page. Larger requests are rejected.
pub const MAX_ROW_PAGE_LIMIT: u32 = 200;
/// Hard maximum offset accepted for a row page.
pub const MAX_ROW_OFFSET: u64 = 1_000_000;
/// Maximum number of filter predicates in one request.
pub const MAX_ROW_FILTERS: usize = 16;
/// Maximum number of values in one `in` filter.
pub const MAX_ROW_FILTER_VALUES: usize = 50;
/// Maximum characters in one bound filter value.
pub const MAX_ROW_FILTER_VALUE_CHARACTERS: usize = 512;
/// Maximum number of sort columns in one request.
pub const MAX_ROW_SORTS: usize = 8;
/// Maximum characters in a search term.
pub const MAX_ROW_SEARCH_CHARACTERS: usize = 256;
/// Maximum text columns searched by one request.
pub const MAX_ROW_SEARCH_COLUMNS: usize = 32;
/// Maximum number of visible columns requested for one page.
pub const MAX_ROW_VISIBLE_COLUMNS: usize = 256;
/// Maximum number of columns read from table metadata.
pub const MAX_ROW_COLUMNS: usize = 1_024;
/// Maximum characters returned for one non-identity cell. Longer values are
/// explicitly marked truncated; identity values are read in full because they
/// are bounded by the unique-index size.
pub const MAX_ROW_CELL_CHARACTERS: usize = 4_096;
/// Default server-side bound for one row request, including metadata and count.
pub const DEFAULT_ROW_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Hard maximum server-side bound for one row request.
pub const MAX_ROW_QUERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum length of a caller-generated cancellation id.
pub const MAX_ROW_REQUEST_ID_LENGTH: usize = 128;

/// One bounded data page request. Unlike metadata page requests, values above
/// the row-page bound are rejected rather than clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowPageRequest {
    pub limit: u32,
    pub offset: u64,
}

impl Default for RowPageRequest {
    fn default() -> Self {
        Self {
            limit: DEFAULT_ROW_PAGE_LIMIT,
            offset: 0,
        }
    }
}

impl RowPageRequest {
    pub fn try_new(limit: u32, offset: u64) -> Result<Self, DatabaseError> {
        if !(1..=MAX_ROW_PAGE_LIMIT).contains(&limit) {
            return Err(DatabaseError::validation(format!(
                "A row page limit must be between 1 and {MAX_ROW_PAGE_LIMIT}."
            )));
        }
        if offset > MAX_ROW_OFFSET {
            return Err(DatabaseError::validation(format!(
                "A row page offset cannot exceed {MAX_ROW_OFFSET}."
            )));
        }
        Ok(Self { limit, offset })
    }

    /// Builds the limit passed to SQL. One extra row is requested so the
    /// adapter can report `has_more` without an exact count.
    pub fn fetch_limit(self) -> u32 {
        self.limit.saturating_add(1)
    }
}

/// How a column renders and which operations it supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowColumnKind {
    Text,
    Integer,
    Float,
    Decimal,
    Boolean,
    Temporal,
    Uuid,
    Json,
    Binary,
    Other,
}

impl RowColumnKind {
    pub const fn is_sortable(self) -> bool {
        !matches!(self, Self::Json | Self::Binary | Self::Other)
    }

    pub const fn is_filterable(self) -> bool {
        self.is_sortable()
    }

    pub const fn is_searchable(self) -> bool {
        matches!(self, Self::Text)
    }
}

/// Classifies an engine-reported column type without trusting it as SQL text.
/// Only a closed set of type families can be sorted or filtered.
pub fn classify_column_kind(data_type: &str) -> RowColumnKind {
    let normalized = data_type.trim().to_ascii_lowercase();
    if normalized.ends_with("[]") {
        return RowColumnKind::Other;
    }
    if normalized.starts_with("tinyint(1)") || normalized == "bool" || normalized == "boolean" {
        return RowColumnKind::Boolean;
    }
    if normalized.starts_with("double precision") {
        return RowColumnKind::Float;
    }
    if normalized.starts_with("timestamp") || normalized.starts_with("datetime") {
        return RowColumnKind::Temporal;
    }
    if normalized.starts_with("time") || normalized.starts_with("date") {
        return RowColumnKind::Temporal;
    }

    let base = normalized.split(['(', ' ', '[']).next().unwrap_or_default();
    match base {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "serial"
        | "smallserial" | "bigserial" | "int2" | "int4" | "int8" | "oid" => RowColumnKind::Integer,
        "decimal" | "numeric" | "dec" | "fixed" => RowColumnKind::Decimal,
        "float" | "double" | "real" => RowColumnKind::Float,
        "boolean" => RowColumnKind::Boolean,
        "date" | "time" | "timestamp" | "datetime" | "year" | "interval" => RowColumnKind::Temporal,
        "uuid" => RowColumnKind::Uuid,
        "json" | "jsonb" => RowColumnKind::Json,
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" | "bytea" => {
            RowColumnKind::Binary
        }
        "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" | "enum" | "set"
        | "citext" | "name" | "character" | "bpchar" | "nvarchar" | "nchar" | "clob" | "xml" => {
            RowColumnKind::Text
        }
        _ => RowColumnKind::Other,
    }
}

pub(crate) fn is_bit_type(data_type: &str) -> bool {
    data_type
        .trim()
        .to_ascii_lowercase()
        .split(['(', ' '])
        .next()
        .is_some_and(|base| base == "bit")
}

pub(crate) fn is_spatial_type(data_type: &str) -> bool {
    matches!(
        data_type
            .trim()
            .to_ascii_lowercase()
            .split(['(', ' '])
            .next()
            .unwrap_or_default(),
        "geometry"
            | "point"
            | "linestring"
            | "polygon"
            | "multipoint"
            | "multilinestring"
            | "multipolygon"
            | "geometrycollection"
    )
}

/// PostgreSQL needs an explicit cast when a text parameter is compared with a
/// temporal, numeric, or UUID column. The cast strings are fixed constants, not
/// metadata text.
pub(crate) fn postgres_parameter_cast(data_type: &str) -> Option<&'static str> {
    let normalized = data_type.trim().to_ascii_lowercase();
    if normalized.starts_with("timestamp with time zone") {
        Some("timestamptz")
    } else if normalized.starts_with("timestamp") {
        Some("timestamp")
    } else if normalized.starts_with("time with time zone") {
        Some("timetz")
    } else if normalized.starts_with("time") {
        Some("time")
    } else if normalized.starts_with("date") {
        Some("date")
    } else if normalized.starts_with("interval") {
        Some("interval")
    } else if normalized.starts_with("numeric") || normalized.starts_with("decimal") {
        Some("numeric")
    } else if normalized == "uuid" {
        Some("uuid")
    } else {
        None
    }
}

/// One column in a row-read response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowColumn {
    pub name: Identifier,
    pub data_type: String,
    pub kind: RowColumnKind,
    pub nullable: bool,
    pub ordinal_position: u32,
    pub primary_key: bool,
    pub identity: ColumnIdentity,
    pub generated: bool,
    pub sortable: bool,
    pub filterable: bool,
    pub searchable: bool,
}

impl RowColumn {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: Identifier,
        data_type: String,
        nullable: bool,
        ordinal_position: u32,
        primary_key: bool,
        identity: ColumnIdentity,
        generated: bool,
    ) -> Self {
        let kind = classify_column_kind(&data_type);
        Self {
            name,
            data_type,
            kind,
            nullable,
            ordinal_position,
            primary_key,
            identity,
            generated,
            sortable: kind.is_sortable(),
            filterable: kind.is_filterable(),
            searchable: kind.is_searchable(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowIdentityKind {
    PrimaryKey,
    UniqueIndex,
    None,
}

/// The stable identity used to address one row for later editing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowIdentity {
    pub kind: RowIdentityKind,
    pub columns: Vec<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<Identifier>,
    pub stable: bool,
}

impl RowIdentity {
    pub fn primary_key(columns: Vec<Identifier>) -> Self {
        Self {
            kind: RowIdentityKind::PrimaryKey,
            columns,
            index: None,
            stable: true,
        }
    }

    pub fn unique_index(index: Identifier, columns: Vec<Identifier>) -> Self {
        Self {
            kind: RowIdentityKind::UniqueIndex,
            columns,
            index: Some(index),
            stable: true,
        }
    }

    pub fn none() -> Self {
        Self {
            kind: RowIdentityKind::None,
            columns: Vec::new(),
            index: None,
            stable: false,
        }
    }
}

/// One candidate unique index decoded by an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueIndexCandidate {
    pub name: Identifier,
    pub columns: Vec<String>,
    /// True only when every key column is a plain, non-null, non-prefix,
    /// non-expression column and all index parts were returned.
    pub complete: bool,
}

/// Chooses the primary key first, then the first complete unique index whose
/// columns are all non-null. A candidate with a nullable column is not a stable
/// row identity because the engine permits duplicate NULL values.
pub fn infer_row_identity(
    columns: &[RowColumn],
    candidates: Vec<UniqueIndexCandidate>,
) -> RowIdentity {
    let primary: Vec<Identifier> = columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.clone())
        .collect();
    if !primary.is_empty() {
        return RowIdentity::primary_key(primary);
    }

    for candidate in candidates {
        if !candidate.complete || candidate.columns.is_empty() {
            continue;
        }
        let mut resolved = Vec::with_capacity(candidate.columns.len());
        let mut valid = true;
        let mut seen = HashSet::new();
        for name in &candidate.columns {
            let Some(column) = columns
                .iter()
                .find(|column| column.name.as_str() == name.as_str())
            else {
                valid = false;
                break;
            };
            if column.nullable || !seen.insert(column.name.as_str()) {
                valid = false;
                break;
            }
            resolved.push(column.name.clone());
        }
        if valid {
            return RowIdentity::unique_index(candidate.name, resolved);
        }
    }

    RowIdentity::none()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowFilterOperator {
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    EndsWith,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    In,
    IsNull,
    IsNotNull,
}

/// A typed filter value. Text values are always bound, never interpolated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum RowFilterValue {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

impl RowFilterValue {
    fn validate(&self) -> Result<(), DatabaseError> {
        match self {
            Self::Text(value) => {
                if value.chars().count() > MAX_ROW_FILTER_VALUE_CHARACTERS {
                    return Err(DatabaseError::validation(format!(
                        "A filter value cannot exceed {MAX_ROW_FILTER_VALUE_CHARACTERS} characters."
                    )));
                }
                if value.contains('\0') {
                    return Err(DatabaseError::validation(
                        "A filter value cannot contain NUL characters.",
                    ));
                }
                Ok(())
            }
            Self::Float(value) if !value.is_finite() => Err(DatabaseError::validation(
                "A floating-point filter value must be finite.",
            )),
            Self::Integer(_) | Self::Float(_) | Self::Boolean(_) => Ok(()),
        }
    }

    fn matches_kind(&self, kind: RowColumnKind) -> bool {
        matches!(
            (self, kind),
            (
                Self::Text(_),
                RowColumnKind::Text | RowColumnKind::Temporal | RowColumnKind::Uuid,
            ) | (Self::Text(_), RowColumnKind::Decimal)
                | (Self::Integer(_), RowColumnKind::Integer)
                | (
                    Self::Integer(_),
                    RowColumnKind::Float | RowColumnKind::Decimal,
                )
                | (
                    Self::Float(_),
                    RowColumnKind::Float | RowColumnKind::Decimal
                )
                | (Self::Boolean(_), RowColumnKind::Boolean)
        )
    }

    fn sql_value(&self) -> SqlValue {
        match self {
            Self::Text(value) => SqlValue::Text(value.clone()),
            Self::Integer(value) => SqlValue::Integer(*value),
            Self::Float(value) => SqlValue::Float(*value),
            Self::Boolean(value) => SqlValue::Boolean(*value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowFilter {
    pub column: Identifier,
    pub operator: RowFilterOperator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<RowFilterValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<RowFilterValue>,
}

impl RowFilter {
    fn validate_shape(&self) -> Result<(), DatabaseError> {
        match self.operator {
            RowFilterOperator::IsNull | RowFilterOperator::IsNotNull => {
                if self.value.is_some() || !self.values.is_empty() {
                    return Err(DatabaseError::validation(
                        "A null-check filter cannot also carry a value.",
                    ));
                }
            }
            RowFilterOperator::In => {
                if self.value.is_some() || self.values.is_empty() {
                    return Err(DatabaseError::validation(
                        "An in filter requires at least one value.",
                    ));
                }
                if self.values.len() > MAX_ROW_FILTER_VALUES {
                    return Err(DatabaseError::validation(format!(
                        "An in filter cannot contain more than {MAX_ROW_FILTER_VALUES} values."
                    )));
                }
            }
            _ => {
                if self.value.is_none() || !self.values.is_empty() {
                    return Err(DatabaseError::validation(
                        "This filter requires exactly one value.",
                    ));
                }
            }
        }

        if let Some(value) = &self.value {
            value.validate()?;
        }
        for value in &self.values {
            value.validate()?;
        }
        Ok(())
    }

    fn validate_for_column(&self, column: &RowColumn) -> Result<(), DatabaseError> {
        // Null checks are safe for every type, including JSON and binary.
        if matches!(
            self.operator,
            RowFilterOperator::IsNull | RowFilterOperator::IsNotNull
        ) {
            return Ok(());
        }
        if !column.filterable {
            return Err(DatabaseError::validation(format!(
                "Column {} cannot be filtered.",
                column.name
            )));
        }
        match self.operator {
            RowFilterOperator::Contains
            | RowFilterOperator::StartsWith
            | RowFilterOperator::EndsWith => {
                if column.kind != RowColumnKind::Text {
                    return Err(DatabaseError::validation(format!(
                        "Column {} does not support text matching.",
                        column.name
                    )));
                }
                match &self.value {
                    Some(RowFilterValue::Text(_)) => Ok(()),
                    _ => Err(DatabaseError::validation(
                        "A text-match filter requires a text value.",
                    )),
                }
            }
            RowFilterOperator::IsNull | RowFilterOperator::IsNotNull => Ok(()),
            RowFilterOperator::In => {
                if self
                    .values
                    .iter()
                    .all(|value| value.matches_kind(column.kind))
                {
                    Ok(())
                } else {
                    Err(DatabaseError::validation(format!(
                        "An in filter value does not match the type of column {}.",
                        column.name
                    )))
                }
            }
            _ => match &self.value {
                Some(value) if value.matches_kind(column.kind) => Ok(()),
                _ => Err(DatabaseError::validation(format!(
                    "The filter value does not match the type of column {}.",
                    column.name
                ))),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowSortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowSort {
    pub column: Identifier,
    pub direction: RowSortDirection,
}

/// Search across selected text columns. An empty column list means all
/// searchable text columns, bounded by [`MAX_ROW_SEARCH_COLUMNS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowSearch {
    pub query: String,
    #[serde(default)]
    pub columns: Vec<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowReadRequest {
    pub table: TableRef,
    pub page: RowPageRequest,
    #[serde(default)]
    pub visible_columns: Vec<Identifier>,
    #[serde(default)]
    pub sort: Vec<RowSort>,
    #[serde(default)]
    pub filters: Vec<RowFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<RowSearch>,
    #[serde(default)]
    pub include_exact_count: bool,
}

impl RowReadRequest {
    /// Validates bounded request shape before any metadata query runs.
    pub fn validate(&self) -> Result<(), DatabaseError> {
        RowPageRequest::try_new(self.page.limit, self.page.offset)?;

        if self.visible_columns.len() > MAX_ROW_VISIBLE_COLUMNS {
            return Err(DatabaseError::validation(format!(
                "A row request cannot select more than {MAX_ROW_VISIBLE_COLUMNS} columns."
            )));
        }
        let mut visible = HashSet::new();
        for column in &self.visible_columns {
            if !visible.insert(column.as_str()) {
                return Err(DatabaseError::validation(format!(
                    "Column {} appears more than once in the visible column list.",
                    column
                )));
            }
        }

        if self.sort.len() > MAX_ROW_SORTS {
            return Err(DatabaseError::validation(format!(
                "A row request cannot sort by more than {MAX_ROW_SORTS} columns."
            )));
        }
        let mut sorted = HashSet::new();
        for sort in &self.sort {
            if !sorted.insert(sort.column.as_str()) {
                return Err(DatabaseError::validation(format!(
                    "Column {} appears more than once in the sort list.",
                    sort.column
                )));
            }
        }

        if self.filters.len() > MAX_ROW_FILTERS {
            return Err(DatabaseError::validation(format!(
                "A row request cannot contain more than {MAX_ROW_FILTERS} filters."
            )));
        }
        for filter in &self.filters {
            filter.validate_shape()?;
        }

        if let Some(search) = &self.search {
            if search.query.chars().count() > MAX_ROW_SEARCH_CHARACTERS {
                return Err(DatabaseError::validation(format!(
                    "A search term cannot exceed {MAX_ROW_SEARCH_CHARACTERS} characters."
                )));
            }
            if search.query.contains('\0') {
                return Err(DatabaseError::validation(
                    "A search term cannot contain NUL characters.",
                ));
            }
            if search.columns.len() > MAX_ROW_SEARCH_COLUMNS {
                return Err(DatabaseError::validation(format!(
                    "A search cannot target more than {MAX_ROW_SEARCH_COLUMNS} columns."
                )));
            }
            let mut searched = HashSet::new();
            for column in &search.columns {
                if !searched.insert(column.as_str()) {
                    return Err(DatabaseError::validation(format!(
                        "Column {} appears more than once in the search column list.",
                        column
                    )));
                }
            }
        }

        Ok(())
    }
}

/// A resolved request. Indices point into [`RowTableMetadata::columns`].
#[derive(Debug, Clone, PartialEq)]
pub struct RowReadPlan {
    pub visible: Vec<usize>,
    pub sort: Vec<(usize, RowSortDirection)>,
    pub filters: Vec<ResolvedFilter>,
    pub search: Option<ResolvedSearch>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFilter {
    pub column: usize,
    pub operator: RowFilterOperator,
    pub value: Option<RowFilterValue>,
    pub values: Vec<RowFilterValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSearch {
    pub columns: Vec<usize>,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowTableMetadata {
    pub engine: DatabaseEngine,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: Identifier,
    pub columns: Vec<RowColumn>,
    pub identity: RowIdentity,
    pub estimated_rows: MetadataValue<u64>,
}

impl RowTableMetadata {
    pub fn column(&self, index: usize) -> Result<&RowColumn, DatabaseError> {
        self.columns
            .get(index)
            .ok_or_else(|| DatabaseError::internal("A row query referenced an unknown column."))
    }

    /// Indices of stable identity columns, in identity order.
    pub fn identity_indices(&self) -> Vec<usize> {
        self.identity
            .columns
            .iter()
            .filter_map(|name| self.columns.iter().position(|column| &column.name == name))
            .collect()
    }
}

/// Resolves identifiers against live metadata and validates every operation.
/// This is the only path an engine SQL builder uses for request input.
pub fn resolve_read_plan(
    request: &RowReadRequest,
    metadata: &RowTableMetadata,
) -> Result<RowReadPlan, DatabaseError> {
    request.validate()?;
    if metadata.columns.is_empty() {
        return Err(DatabaseError::query(
            "The selected table does not expose any columns.",
        ));
    }

    let visible = if request.visible_columns.is_empty() {
        if metadata.columns.len() > MAX_ROW_VISIBLE_COLUMNS {
            return Err(DatabaseError::validation(format!(
                "This table has more than {MAX_ROW_VISIBLE_COLUMNS} columns. Choose the visible \
                 columns explicitly."
            )));
        }
        (0..metadata.columns.len()).collect::<Vec<_>>()
    } else {
        let mut indices = Vec::with_capacity(request.visible_columns.len());
        for name in &request.visible_columns {
            let index = metadata
                .columns
                .iter()
                .position(|column| &column.name == name)
                .ok_or_else(|| {
                    DatabaseError::validation(format!(
                        "Column {name} does not exist in table {}.",
                        metadata.table
                    ))
                })?;
            indices.push(index);
        }
        indices
    };

    let mut sort = Vec::with_capacity(request.sort.len());
    for requested in &request.sort {
        let index = metadata
            .columns
            .iter()
            .position(|column| column.name == requested.column)
            .ok_or_else(|| {
                DatabaseError::validation(format!(
                    "Sort column {} does not exist in table {}.",
                    requested.column, metadata.table
                ))
            })?;
        let column = metadata.column(index)?;
        if !column.sortable {
            return Err(DatabaseError::validation(format!(
                "Column {} cannot be sorted.",
                column.name
            )));
        }
        sort.push((index, requested.direction));
    }

    let mut filters = Vec::with_capacity(request.filters.len());
    for requested in &request.filters {
        let index = metadata
            .columns
            .iter()
            .position(|column| column.name == requested.column)
            .ok_or_else(|| {
                DatabaseError::validation(format!(
                    "Filter column {} does not exist in table {}.",
                    requested.column, metadata.table
                ))
            })?;
        let column = metadata.column(index)?;
        requested.validate_for_column(column)?;
        filters.push(ResolvedFilter {
            column: index,
            operator: requested.operator,
            value: requested.value.clone(),
            values: requested.values.clone(),
        });
    }

    let search = match &request.search {
        Some(search) if !search.query.trim().is_empty() => {
            let columns = if search.columns.is_empty() {
                metadata
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(_, column)| column.searchable)
                    .map(|(index, _)| index)
                    .take(MAX_ROW_SEARCH_COLUMNS)
                    .collect::<Vec<_>>()
            } else {
                let mut indices = Vec::with_capacity(search.columns.len());
                for name in &search.columns {
                    let index = metadata
                        .columns
                        .iter()
                        .position(|column| &column.name == name)
                        .ok_or_else(|| {
                            DatabaseError::validation(format!(
                                "Search column {name} does not exist in table {}.",
                                metadata.table
                            ))
                        })?;
                    if !metadata.column(index)?.searchable {
                        return Err(DatabaseError::validation(format!(
                            "Column {name} cannot be searched."
                        )));
                    }
                    indices.push(index);
                }
                indices
            };
            if columns.is_empty() {
                return Err(DatabaseError::validation(
                    "The table has no searchable text columns.",
                ));
            }
            Some(ResolvedSearch {
                columns,
                query: search.query.clone(),
            })
        }
        _ => None,
    };

    Ok(RowReadPlan {
        visible,
        sort,
        filters,
        search,
    })
}

/// A projected output column in a generated SELECT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowOutputColumn {
    pub column: usize,
    /// True for a visible cell that may be truncated for display.
    pub visible: bool,
    pub truncate: bool,
}

/// A generated row page query and the column layout needed to decode it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowSelectQuery {
    pub query: ParameterizedQuery,
    pub outputs: Vec<RowOutputColumn>,
    pub visible_columns: Vec<usize>,
    pub identity_columns: Vec<usize>,
}

/// Builds the output column list shared by both engines. Visible columns keep
/// their requested order; identity columns that are not visible are appended so
/// every row carries its stable address.
pub fn projected_outputs(plan: &RowReadPlan, metadata: &RowTableMetadata) -> Vec<RowOutputColumn> {
    let identity = metadata.identity_indices();
    let mut outputs = Vec::with_capacity(plan.visible.len() + identity.len());
    for index in &plan.visible {
        outputs.push(RowOutputColumn {
            column: *index,
            visible: true,
            truncate: !identity.contains(index),
        });
    }
    for index in identity {
        if !plan.visible.contains(&index) {
            outputs.push(RowOutputColumn {
                column: index,
                visible: false,
                truncate: false,
            });
        }
    }
    outputs
}

/// One table cell in the response. `None` is SQL NULL; `truncated` is explicit
/// and never used to silently claim a value is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowCell {
    pub value: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowIdentityValue {
    pub column: Identifier,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowRecord {
    pub cells: Vec<RowCell>,
    pub identity: Vec<RowIdentityValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowCounts {
    /// Engine statistics, explicitly labelled as an estimate. Never used as an
    /// exact total and never substituted with zero when unavailable.
    pub estimated_table_rows: MetadataValue<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_rows: Option<RowExactCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowExactCount {
    pub value: u64,
    /// True when filters or search reduce the counted set.
    pub filtered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowPageInfo {
    pub limit: u32,
    pub offset: u64,
    pub returned: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowReadOnlyReason {
    NoStableIdentity,
    SessionReadOnly,
}

/// One bounded page of table data plus everything the data grid needs to
/// render and page it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowReadResult {
    pub engine: DatabaseEngine,
    pub table: TableRef,
    pub columns: Vec<RowColumn>,
    pub visible_columns: Vec<Identifier>,
    pub identity: RowIdentity,
    pub editable: bool,
    pub read_only_reasons: Vec<RowReadOnlyReason>,
    pub rows: Vec<RowRecord>,
    pub page: RowPageInfo,
    pub counts: RowCounts,
}

impl RowReadResult {
    pub fn new(
        metadata: RowTableMetadata,
        plan: &RowReadPlan,
        rows: Vec<RowRecord>,
        page: RowPageRequest,
        has_more: bool,
        exact_rows: Option<RowExactCount>,
        session_read_only: bool,
    ) -> Self {
        let returned = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        let mut read_only_reasons = Vec::new();
        if !metadata.identity.stable {
            read_only_reasons.push(RowReadOnlyReason::NoStableIdentity);
        }
        if session_read_only {
            read_only_reasons.push(RowReadOnlyReason::SessionReadOnly);
        }
        let visible_columns = plan
            .visible
            .iter()
            .filter_map(|index| metadata.column(*index).ok())
            .map(|column| column.name.clone())
            .collect();

        Self {
            engine: metadata.engine,
            table: TableRef {
                database: metadata
                    .database
                    .as_deref()
                    .and_then(|value| Identifier::new(value).ok()),
                schema: metadata
                    .schema
                    .as_deref()
                    .and_then(|value| Identifier::new(value).ok()),
                name: metadata.table.clone(),
            },
            columns: metadata.columns,
            visible_columns,
            identity: metadata.identity,
            editable: read_only_reasons.is_empty(),
            read_only_reasons,
            rows,
            page: RowPageInfo {
                limit: page.limit,
                offset: page.offset,
                returned,
                has_more,
            },
            counts: RowCounts {
                estimated_table_rows: metadata.estimated_rows,
                exact_rows,
            },
        }
    }
}

/// Decodes one driver row from the output layout produced by an engine
/// builder. Values are already text; truncation is applied here so the marker
/// and value can never disagree.
pub fn row_from_values(
    metadata: &RowTableMetadata,
    outputs: &[RowOutputColumn],
    values: Vec<Option<String>>,
) -> Result<RowRecord, DatabaseError> {
    if values.len() != outputs.len() {
        return Err(DatabaseError::internal(
            "The row query returned an unexpected number of values.",
        ));
    }

    let mut processed: HashMap<usize, (Option<String>, bool)> = HashMap::new();
    for (output, value) in outputs.iter().zip(values) {
        let (value, truncated) = if output.truncate {
            truncate_cell(value)
        } else {
            (value, false)
        };
        processed.insert(output.column, (value, truncated));
    }

    let cells = outputs
        .iter()
        .filter(|output| output.visible)
        .map(|output| {
            let (value, truncated) = processed.get(&output.column).cloned().unwrap_or_default();
            RowCell { value, truncated }
        })
        .collect();

    let identity = metadata
        .identity_indices()
        .into_iter()
        .map(|index| {
            let column = metadata.columns[index].name.clone();
            let (value, _) = processed.get(&index).cloned().unwrap_or_default();
            RowIdentityValue { column, value }
        })
        .collect();

    Ok(RowRecord { cells, identity })
}

fn truncate_cell(value: Option<String>) -> (Option<String>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    if value.chars().count() <= MAX_ROW_CELL_CHARACTERS {
        return (Some(value), false);
    }
    let truncated = value.chars().take(MAX_ROW_CELL_CHARACTERS).collect();
    (Some(truncated), true)
}

/// Which values can appear in a filter. Used by tests and adapters to convert a
/// typed filter value into one driver parameter without string interpolation.
pub(crate) fn sql_value_for_column(column: &RowColumn, value: &RowFilterValue) -> SqlValue {
    match (column.kind, value) {
        (RowColumnKind::Float, RowFilterValue::Integer(number)) => SqlValue::Float(*number as f64),
        _ => value.sql_value(),
    }
}

/// Escape text for a `LIKE` pattern using `=` as the escape character. Both
/// supported engines accept `ESCAPE '='`, avoiding backslash-mode differences.
pub(crate) fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' | '_' | '=' => {
                escaped.push('=');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

pub(crate) fn contains_pattern(value: &str) -> String {
    format!("%{}%", escape_like(value))
}

pub(crate) fn starts_with_pattern(value: &str) -> String {
    format!("{}%", escape_like(value))
}

pub(crate) fn ends_with_pattern(value: &str) -> String {
    format!("%{}", escape_like(value))
}

/// Cancellation shared by the command layer and one in-flight adapter call.
#[derive(Debug, Clone, Default)]
pub struct RowQueryCancellation {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl RowQueryCancellation {
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

/// Per-request timeout and optional cancellation.
#[derive(Debug, Clone)]
pub struct RowQueryControl {
    pub timeout: Duration,
    pub cancellation: Option<RowQueryCancellation>,
}

impl Default for RowQueryControl {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_ROW_QUERY_TIMEOUT,
            cancellation: None,
        }
    }
}

impl RowQueryControl {
    pub fn new(timeout: Duration, cancellation: Option<RowQueryCancellation>) -> Self {
        Self {
            timeout: timeout.clamp(Duration::from_millis(1), MAX_ROW_QUERY_TIMEOUT),
            cancellation,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(RowQueryCancellation::is_cancelled)
    }

    pub async fn cancelled(&self) {
        match &self.cancellation {
            Some(cancellation) => cancellation.cancelled().await,
            None => std::future::pending().await,
        }
    }
}

/// Validates a caller-generated request id used to cancel one read. Ids are not
/// SQL and never reach a driver as text; this bound keeps registry keys safe.
pub fn validate_row_query_id(value: &str) -> Result<(), DatabaseError> {
    if value.is_empty() || value.len() > MAX_ROW_REQUEST_ID_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A row request id must contain between 1 and {MAX_ROW_REQUEST_ID_LENGTH} characters."
        )));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(DatabaseError::validation(
            "A row request id may contain only letters, digits, hyphens, underscores, and dots.",
        ));
    }
    Ok(())
}

/// Resolves an optional timeout in milliseconds with a hard upper bound.
pub fn row_query_timeout(timeout_ms: Option<u64>) -> Duration {
    let milliseconds = timeout_ms
        .unwrap_or(DEFAULT_ROW_QUERY_TIMEOUT.as_millis() as u64)
        .clamp(1_000, MAX_ROW_QUERY_TIMEOUT.as_millis() as u64);
    Duration::from_millis(milliseconds)
}

type CancellationEntries = Arc<Mutex<HashMap<String, Arc<RowQueryCancellation>>>>;

/// Tracks the cancellation handle for each in-flight row read.
#[derive(Debug, Clone, Default)]
pub struct RowQueryRegistry {
    entries: CancellationEntries,
}

impl RowQueryRegistry {
    pub fn begin(&self, request_id: &str) -> Result<RowQueryGuard, DatabaseError> {
        validate_row_query_id(request_id)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.contains_key(request_id) {
            return Err(DatabaseError::validation(
                "A row read with this request id is already running.",
            ));
        }
        let cancellation = Arc::new(RowQueryCancellation::default());
        entries.insert(request_id.to_string(), Arc::clone(&cancellation));
        Ok(RowQueryGuard {
            request_id: request_id.to_string(),
            cancellation,
            entries: Arc::clone(&self.entries),
        })
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        if validate_row_query_id(request_id).is_err() {
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
pub struct RowQueryGuard {
    request_id: String,
    cancellation: Arc<RowQueryCancellation>,
    entries: CancellationEntries,
}

impl RowQueryGuard {
    pub fn cancellation(&self) -> RowQueryCancellation {
        RowQueryCancellation {
            cancelled: Arc::clone(&self.cancellation.cancelled),
            notify: Arc::clone(&self.cancellation.notify),
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

impl Drop for RowQueryGuard {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn identifier(value: &str) -> Identifier {
        Identifier::new(value).expect("identifier")
    }

    fn column(name: &str, data_type: &str, nullable: bool, primary: bool) -> RowColumn {
        RowColumn::new(
            identifier(name),
            data_type.to_string(),
            nullable,
            1,
            primary,
            ColumnIdentity::None,
            false,
        )
    }

    fn base_metadata(columns: Vec<RowColumn>, identity: RowIdentity) -> RowTableMetadata {
        RowTableMetadata {
            engine: DatabaseEngine::Postgresql,
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: identifier("users"),
            columns,
            identity,
            estimated_rows: MetadataValue::Available(42),
        }
    }

    fn base_request() -> RowReadRequest {
        RowReadRequest {
            table: TableRef::new(Some("app"), Some("public"), "users").expect("table"),
            page: RowPageRequest::try_new(10, 0).expect("page"),
            visible_columns: Vec::new(),
            sort: Vec::new(),
            filters: Vec::new(),
            search: None,
            include_exact_count: false,
        }
    }

    #[test]
    fn row_pages_are_hard_bounded() {
        assert_eq!(RowPageRequest::default().limit, DEFAULT_ROW_PAGE_LIMIT);
        assert!(RowPageRequest::try_new(0, 0).is_err());
        assert!(RowPageRequest::try_new(MAX_ROW_PAGE_LIMIT + 1, 0).is_err());
        assert!(RowPageRequest::try_new(1, MAX_ROW_OFFSET + 1).is_err());
        assert_eq!(
            RowPageRequest::try_new(MAX_ROW_PAGE_LIMIT, MAX_ROW_OFFSET)
                .expect("max page")
                .fetch_limit(),
            MAX_ROW_PAGE_LIMIT + 1
        );
    }

    #[test]
    fn classifies_engine_types_without_treating_arrays_as_scalars() {
        assert_eq!(
            classify_column_kind("bigint unsigned"),
            RowColumnKind::Integer
        );
        assert_eq!(classify_column_kind("tinyint(1)"), RowColumnKind::Boolean);
        assert_eq!(
            classify_column_kind("character varying(255)"),
            RowColumnKind::Text
        );
        assert_eq!(
            classify_column_kind("timestamp with time zone"),
            RowColumnKind::Temporal
        );
        assert_eq!(
            classify_column_kind("numeric(10,2)"),
            RowColumnKind::Decimal
        );
        assert_eq!(classify_column_kind("integer[]"), RowColumnKind::Other);
        assert_eq!(classify_column_kind("jsonb"), RowColumnKind::Json);
        assert_eq!(classify_column_kind("bytea"), RowColumnKind::Binary);
        assert_eq!(classify_column_kind("uuid"), RowColumnKind::Uuid);
        assert_eq!(classify_column_kind("point"), RowColumnKind::Other);
    }

    #[test]
    fn identity_prefers_primary_key_then_complete_non_null_unique_indexes() {
        let columns = vec![
            column("id", "bigint", false, true),
            column("email", "text", false, false),
            column("tenant", "text", true, false),
        ];
        assert_eq!(
            infer_row_identity(
                &columns,
                vec![UniqueIndexCandidate {
                    name: identifier("users_email_key"),
                    columns: vec!["email".to_string()],
                    complete: true,
                }]
            ),
            RowIdentity::primary_key(vec![identifier("id")])
        );

        let columns = vec![
            column("email", "text", false, false),
            column("tenant", "text", true, false),
        ];
        assert_eq!(
            infer_row_identity(
                &columns,
                vec![
                    UniqueIndexCandidate {
                        name: identifier("users_tenant_email_key"),
                        columns: vec!["tenant".to_string(), "email".to_string()],
                        complete: true,
                    },
                    UniqueIndexCandidate {
                        name: identifier("users_email_key"),
                        columns: vec!["email".to_string()],
                        complete: true,
                    },
                ]
            ),
            RowIdentity::unique_index(identifier("users_email_key"), vec![identifier("email")])
        );

        assert_eq!(
            infer_row_identity(&columns, Vec::new()).kind,
            RowIdentityKind::None
        );
        assert!(!infer_row_identity(&columns, Vec::new()).stable);
    }

    #[test]
    fn plan_rejects_injection_columns_and_validates_filter_types() {
        let columns = vec![
            column("id", "bigint", false, true),
            column("name", "text", false, false),
            column("payload", "jsonb", false, false),
        ];
        let metadata = base_metadata(columns, RowIdentity::primary_key(vec![identifier("id")]));
        let mut request = base_request();
        request.sort.push(RowSort {
            column: identifier("name` DESC; DROP TABLE users; --"),
            direction: RowSortDirection::Asc,
        });
        assert_eq!(
            resolve_read_plan(&request, &metadata)
                .expect_err("unknown sort column")
                .kind,
            crate::database::DatabaseErrorKind::Validation
        );

        let mut request = base_request();
        request.filters.push(RowFilter {
            column: identifier("id"),
            operator: RowFilterOperator::Equals,
            value: Some(RowFilterValue::Text("1; DROP TABLE users".to_string())),
            values: Vec::new(),
        });
        assert!(resolve_read_plan(&request, &metadata).is_err());

        let mut request = base_request();
        request.filters.push(RowFilter {
            column: identifier("payload"),
            operator: RowFilterOperator::Equals,
            value: Some(RowFilterValue::Text("{}".to_string())),
            values: Vec::new(),
        });
        assert!(resolve_read_plan(&request, &metadata).is_err());
    }

    #[test]
    fn plan_bounds_search_columns_and_rejects_empty_text_tables() {
        let columns: Vec<RowColumn> = (0..40)
            .map(|index| column(&format!("name_{index}"), "text", false, false))
            .collect();
        let metadata = base_metadata(columns, RowIdentity::none());
        let mut request = base_request();
        request.search = Some(RowSearch {
            query: "ann".to_string(),
            columns: Vec::new(),
        });
        let plan = resolve_read_plan(&request, &metadata).expect("plan");
        assert_eq!(
            plan.search.expect("search").columns.len(),
            MAX_ROW_SEARCH_COLUMNS
        );

        let metadata = base_metadata(
            vec![column("id", "bigint", false, false)],
            RowIdentity::none(),
        );
        let mut request = base_request();
        request.search = Some(RowSearch {
            query: "ann".to_string(),
            columns: Vec::new(),
        });
        assert!(resolve_read_plan(&request, &metadata).is_err());
    }

    #[test]
    fn table_without_identity_is_read_only_for_editing() {
        let metadata = base_metadata(
            vec![column("name", "text", false, false)],
            RowIdentity::none(),
        );
        let request = base_request();
        let plan = resolve_read_plan(&request, &metadata).expect("plan");
        let result = RowReadResult::new(
            metadata,
            &plan,
            Vec::new(),
            request.page,
            false,
            None,
            false,
        );
        assert!(!result.editable);
        assert_eq!(
            result.read_only_reasons,
            [RowReadOnlyReason::NoStableIdentity]
        );
        assert!(!result.identity.stable);
    }

    #[test]
    fn identity_columns_are_appended_even_when_not_visible() {
        let metadata = base_metadata(
            vec![
                column("id", "bigint", false, true),
                column("name", "text", false, false),
            ],
            RowIdentity::primary_key(vec![identifier("id")]),
        );
        let mut request = base_request();
        request.visible_columns = vec![identifier("name")];
        let plan = resolve_read_plan(&request, &metadata).expect("plan");
        let outputs = projected_outputs(&plan, &metadata);

        assert_eq!(outputs.len(), 2);
        assert!(outputs[0].visible);
        assert_eq!(outputs[0].column, 1);
        assert!(!outputs[1].visible);
        assert_eq!(outputs[1].column, 0);
        assert!(!outputs[1].truncate);
    }

    #[test]
    fn row_values_mark_truncation_and_keep_full_identity_values() {
        let metadata = base_metadata(
            vec![
                column("id", "text", false, true),
                column("name", "text", false, false),
            ],
            RowIdentity::primary_key(vec![identifier("id")]),
        );
        let mut request = base_request();
        request.visible_columns = vec![identifier("name")];
        let plan = resolve_read_plan(&request, &metadata).expect("plan");
        let outputs = projected_outputs(&plan, &metadata);
        let long = "x".repeat(MAX_ROW_CELL_CHARACTERS + 5);
        let row = row_from_values(
            &metadata,
            &outputs,
            vec![Some(long), Some("identity-value".to_string())],
        )
        .expect("row");

        assert_eq!(row.cells.len(), 1);
        assert!(row.cells[0].truncated);
        assert_eq!(
            row.cells[0].value.as_ref().map(String::len),
            Some(MAX_ROW_CELL_CHARACTERS)
        );
        assert_eq!(row.identity[0].column, identifier("id"));
        assert_eq!(row.identity[0].value.as_deref(), Some("identity-value"));
    }

    #[test]
    fn like_patterns_escape_wildcards_and_the_escape_character() {
        assert_eq!(escape_like("a%b_c=d"), "a=%b=_c==d");
        assert_eq!(contains_pattern("50%"), "%50=%%");
        assert_eq!(starts_with_pattern("a_b"), "a=_b%");
        assert_eq!(ends_with_pattern("a=b"), "%a==b");
    }

    #[test]
    fn cancellation_registry_rejects_bad_ids_and_is_dropped_with_the_guard() {
        assert!(validate_row_query_id("read-1").is_ok());
        for value in [
            "",
            "bad id",
            "bad;id",
            &"a".repeat(MAX_ROW_REQUEST_ID_LENGTH + 1),
        ] {
            assert!(validate_row_query_id(value).is_err(), "{value:?}");
        }

        let registry = RowQueryRegistry::default();
        let guard = registry.begin("read-1").expect("begin");
        assert_eq!(registry.active_count(), 1);
        assert!(registry.begin("read-1").is_err());
        assert!(registry.cancel("read-1"));
        assert!(guard.cancellation().is_cancelled());
        assert!(!registry.cancel("missing"));
        drop(guard);
        assert_eq!(registry.active_count(), 0);
    }

    #[test]
    fn timeouts_are_clamped_to_the_hard_bound() {
        assert_eq!(row_query_timeout(None), DEFAULT_ROW_QUERY_TIMEOUT);
        assert_eq!(row_query_timeout(Some(0)), Duration::from_millis(1_000));
        assert_eq!(row_query_timeout(Some(u64::MAX)), MAX_ROW_QUERY_TIMEOUT);
    }

    #[test]
    fn read_result_never_serializes_unrelated_metadata() {
        let metadata = base_metadata(
            vec![column("id", "bigint", false, true)],
            RowIdentity::primary_key(vec![identifier("id")]),
        );
        let request = base_request();
        let plan = resolve_read_plan(&request, &metadata).expect("plan");
        let result = RowReadResult::new(
            metadata,
            &plan,
            Vec::new(),
            request.page,
            false,
            Some(RowExactCount {
                value: 0,
                filtered: false,
            }),
            false,
        );
        let json = serde_json::to_string(&result).expect("result json");
        assert!(json.contains("\"estimatedTableRows\":{\"status\":\"available\",\"value\":42}"));
        assert!(json.contains("\"exactRows\":{\"value\":0,\"filtered\":false}"));
        assert!(!json.contains("password"));
        assert!(!json.contains("mysql://"));
    }
}
