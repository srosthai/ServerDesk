//! Central identifier quoting and value parameterization.
//!
//! Every engine-specific SQL builder goes through these interfaces. Values are
//! always bound through driver placeholders; they are never concatenated into
//! SQL text. Identifiers are validated once and quoted by the selected engine
//! dialect.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::storage::DatabaseEngine;

use super::errors::DatabaseError;

/// Shared identifier bound. MySQL permits 64 characters and PostgreSQL 63
/// bytes, so the stricter 63-byte bound is used for both engines.
pub const MAX_IDENTIFIER_LENGTH: usize = 63;

/// Maximum number of positional parameters accepted by the shared builder.
pub const MAX_BIND_PARAMETERS: usize = u16::MAX as usize;

/// A single validated SQL identifier. The name is kept as written; callers
/// must never add engine-specific quote characters themselves.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    pub fn new(value: impl Into<String>) -> Result<Self, DatabaseError> {
        let value = value.into();

        if value.is_empty() || value.trim().is_empty() {
            return Err(DatabaseError::validation(
                "A database identifier cannot be empty.",
            ));
        }
        if value.len() > MAX_IDENTIFIER_LENGTH {
            return Err(DatabaseError::validation(format!(
                "A database identifier cannot exceed {MAX_IDENTIFIER_LENGTH} bytes."
            )));
        }
        if value.contains('.') {
            return Err(DatabaseError::validation(
                "Use QualifiedName for database, schema, and table segments.",
            ));
        }
        if value.contains('\0') || value.chars().any(char::is_control) {
            return Err(DatabaseError::validation(
                "A database identifier cannot contain NUL or control characters.",
            ));
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Identifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Identifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// One to three validated segments, for example `database.schema.table`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    parts: Vec<Identifier>,
}

impl QualifiedName {
    pub fn new(parts: impl IntoIterator<Item = Identifier>) -> Result<Self, DatabaseError> {
        let parts = parts.into_iter().collect::<Vec<_>>();
        if parts.is_empty() || parts.len() > 3 {
            return Err(DatabaseError::validation(
                "A qualified database name must have one to three segments.",
            ));
        }
        Ok(Self { parts })
    }

    pub fn from_segments(parts: &[&str]) -> Result<Self, DatabaseError> {
        Self::new(
            parts
                .iter()
                .map(|part| Identifier::new(*part))
                .collect::<Result<Vec<_>, _>>()?,
        )
    }

    pub fn parts(&self) -> &[Identifier] {
        &self.parts
    }
}

/// Engine-specific identifier quoting. Validation lives in [`Identifier`] so a
/// quoted value can never contain an unescaped delimiter.
pub trait IdentifierQuoter: Send + Sync {
    fn quote_identifier(&self, identifier: &Identifier) -> String;

    fn quote_qualified_name(&self, name: &QualifiedName) -> String {
        name.parts()
            .iter()
            .map(|part| self.quote_identifier(part))
            .collect::<Vec<_>>()
            .join(".")
    }
}

/// Engine-specific positional placeholder generation.
pub trait ValueParameterizer: Send + Sync {
    /// Returns the placeholder for a one-based parameter position.
    fn placeholder(&self, position: usize) -> Result<String, DatabaseError>;
}

/// The combined dialect used by adapters and shared query builders.
pub trait SqlDialect: IdentifierQuoter + ValueParameterizer + Send + Sync {
    fn engine(&self) -> DatabaseEngine;
}

pub(crate) fn validate_parameter_position(position: usize) -> Result<(), DatabaseError> {
    if !(1..=MAX_BIND_PARAMETERS).contains(&position) {
        return Err(DatabaseError::validation(format!(
            "A database parameter position must be between 1 and {MAX_BIND_PARAMETERS}."
        )));
    }
    Ok(())
}

/// A typed value bound to a query placeholder.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl From<bool> for SqlValue {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

impl From<i64> for SqlValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<i32> for SqlValue {
    fn from(value: i32) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<f64> for SqlValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<String> for SqlValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for SqlValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<Vec<u8>> for SqlValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

/// Builds SQL with engine-specific identifiers and placeholders while keeping
/// values in a separate parameter list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParameterizedQuery {
    sql: String,
    values: Vec<SqlValue>,
}

impl ParameterizedQuery {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends trusted SQL text. Engine modules use this for fixed keywords,
    /// column lists, and clauses; caller values must use [`Self::push_value`].
    pub fn push_sql(&mut self, fragment: &str) -> &mut Self {
        self.sql.push_str(fragment);
        self
    }

    pub fn push_identifier(
        &mut self,
        dialect: &dyn SqlDialect,
        identifier: &Identifier,
    ) -> &mut Self {
        self.sql.push_str(&dialect.quote_identifier(identifier));
        self
    }

    pub fn push_qualified_name(
        &mut self,
        dialect: &dyn SqlDialect,
        name: &QualifiedName,
    ) -> &mut Self {
        self.sql.push_str(&dialect.quote_qualified_name(name));
        self
    }

    pub fn push_value(
        &mut self,
        dialect: &dyn SqlDialect,
        value: impl Into<SqlValue>,
    ) -> Result<&mut Self, DatabaseError> {
        let placeholder = dialect.placeholder(self.values.len() + 1)?;
        self.sql.push_str(&placeholder);
        self.values.push(value.into());
        Ok(self)
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn values(&self) -> &[SqlValue] {
        &self.values
    }

    pub fn into_parts(self) -> (String, Vec<SqlValue>) {
        (self.sql, self.values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{mysql::MysqlDialect, postgres::PostgresDialect};

    fn identifier(value: &str) -> Identifier {
        Identifier::new(value).expect("valid identifier")
    }

    #[test]
    fn quotes_and_escapes_mysql_identifiers() {
        let dialect = MysqlDialect;

        assert_eq!(dialect.quote_identifier(&identifier("users")), "`users`");
        assert_eq!(
            dialect.quote_identifier(&identifier("odd`name")),
            "`odd``name`"
        );

        let qualified = QualifiedName::from_segments(&["app", "users"]).expect("name");
        assert_eq!(dialect.quote_qualified_name(&qualified), "`app`.`users`");
    }

    #[test]
    fn quotes_and_escapes_postgresql_identifiers() {
        let dialect = PostgresDialect;

        assert_eq!(dialect.quote_identifier(&identifier("users")), "\"users\"");
        assert_eq!(
            dialect.quote_identifier(&identifier("odd\"name")),
            "\"odd\"\"name\""
        );

        let qualified = QualifiedName::from_segments(&["app", "public", "users"]).expect("name");
        assert_eq!(
            dialect.quote_qualified_name(&qualified),
            "\"app\".\"public\".\"users\""
        );
    }

    #[test]
    fn rejects_unsafe_identifier_construction() {
        for unsafe_value in [
            "",
            "   ",
            "app.users",
            "users\0name",
            "users\nname",
            &"a".repeat(MAX_IDENTIFIER_LENGTH + 1),
        ] {
            assert!(
                Identifier::new(unsafe_value).is_err(),
                "{unsafe_value:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_invalid_qualified_names() {
        assert!(QualifiedName::new([]).is_err());
        assert!(
            QualifiedName::new([
                identifier("a"),
                identifier("b"),
                identifier("c"),
                identifier("d"),
            ])
            .is_err()
        );
    }

    #[test]
    fn injection_text_stays_inside_one_quoted_identifier() {
        let dialect = MysqlDialect;
        let value = identifier("users; DROP TABLE audit; --");
        let quoted = dialect.quote_identifier(&value);

        assert_eq!(quoted, "`users; DROP TABLE audit; --`");
        assert!(quoted.starts_with('`'));
        assert!(quoted.ends_with('`'));
    }

    #[test]
    fn selects_engine_specific_placeholders() {
        let mysql = MysqlDialect;
        let postgres = PostgresDialect;

        assert_eq!(mysql.placeholder(1).expect("placeholder"), "?");
        assert_eq!(mysql.placeholder(99).expect("placeholder"), "?");
        assert_eq!(postgres.placeholder(1).expect("placeholder"), "$1");
        assert_eq!(postgres.placeholder(12).expect("placeholder"), "$12");

        for dialect in [&mysql as &dyn SqlDialect, &postgres as &dyn SqlDialect] {
            assert!(dialect.placeholder(0).is_err());
            assert!(dialect.placeholder(MAX_BIND_PARAMETERS + 1).is_err());
        }
    }

    #[test]
    fn parameterized_query_keeps_values_out_of_sql_text() {
        let dialect = PostgresDialect;
        let table = QualifiedName::from_segments(&["public", "users"]).expect("table");
        let mut query = ParameterizedQuery::new();

        query
            .push_sql("SELECT * FROM ")
            .push_qualified_name(&dialect, &table)
            .push_sql(" WHERE name = ")
            .push_value(&dialect, "Robert'; DROP TABLE users; --")
            .expect("value")
            .push_sql(" AND active = ")
            .push_value(&dialect, true)
            .expect("value");

        assert_eq!(
            query.sql(),
            "SELECT * FROM \"public\".\"users\" WHERE name = $1 AND active = $2"
        );
        assert_eq!(
            query.values(),
            [
                SqlValue::Text("Robert'; DROP TABLE users; --".to_string()),
                SqlValue::Boolean(true)
            ]
        );
        assert!(!query.sql().contains("DROP TABLE"));
    }
}
