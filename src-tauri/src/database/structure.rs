//! Typed table, column, index, and constraint changes with DDL previews.
//!
//! Requests cross the Tauri boundary as untrusted JSON. [`StructureActionRequest::validate`]
//! turns them into [`ValidatedStructureAction`] values: every identifier has been
//! checked by the shared [`Identifier`] rules, every type name and expression is
//! bounded, and every user value is represented by a typed value instead of raw
//! SQL. Engine modules generate the DDL from the validated action.
//!
//! Execution always asks an adapter for the same preview first, so capability
//! checks, read-only checks, and destructive confirmations cannot be bypassed.
//! Column execution re-reads column metadata before reporting success.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{core::AppError, storage::DatabaseEngine};

use super::{
    capabilities::{DatabaseAction as CapabilityAction, DatabaseCapabilities},
    dialect::Identifier,
    errors::{DatabaseError, DatabaseErrorKind},
    pagination::{Bounded, MAX_PAGE_LIMIT, PageRequest},
    service::DatabaseConnectionRegistry,
    traits::{
        ColumnIdentity, ColumnSummary, ConstraintKind, DatabaseAdapter, DependencySummary,
        MetadataValue, TableRef, TableSummary,
    },
};

/// Longest accepted data type spelling, including length/precision modifiers.
pub const MAX_DATA_TYPE_LENGTH: usize = 128;
/// Longest accepted generated-column, check, or partial-index expression.
pub const MAX_COLUMN_EXPRESSION_LENGTH: usize = 2_000;
/// Longest accepted column comment.
pub const MAX_COLUMN_COMMENT_LENGTH: usize = 1_024;
/// Longest accepted string default.
pub const MAX_DEFAULT_STRING_LENGTH: usize = 4_000;
/// Maximum number of columns in a table, index, or constraint definition.
pub const MAX_COLUMN_LIST_LENGTH: usize = 64;
/// Maximum length accepted for an index access method.
pub const MAX_INDEX_METHOD_LENGTH: usize = 32;

/// One structure action requested by a future editor UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructureActionRequest {
    CreateTable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        columns: Vec<ColumnDefinitionRequest>,
        #[serde(default, rename = "primaryKey", skip_serializing_if = "Vec::is_empty")]
        primary_key: Vec<String>,
        #[serde(default, rename = "uniqueKeys", skip_serializing_if = "Vec::is_empty")]
        unique_keys: Vec<UniqueKeyRequest>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        options: Option<TableOptionsRequest>,
    },
    RenameTable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(rename = "newName")]
        new_name: String,
    },
    DuplicateTable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(rename = "newName")]
        new_name: String,
    },
    DropTable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
    },
    TruncateTable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
    },
    AddColumn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        column: ColumnDefinitionRequest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<ColumnPositionRequest>,
    },
    AlterColumn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        name: String,
        #[serde(default, rename = "dataType", skip_serializing_if = "Option::is_none")]
        data_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nullable: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<ColumnDefaultRequest>,
        #[serde(default, rename = "dropDefault")]
        drop_default: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        comment: Option<String>,
        #[serde(default, rename = "dropComment")]
        drop_comment: bool,
    },
    RenameColumn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        name: String,
        #[serde(rename = "newName")]
        new_name: String,
    },
    DropColumn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        name: String,
    },
    ReorderColumn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        column: ColumnDefinitionRequest,
        position: ColumnPositionRequest,
    },
    CreateIndex {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        name: String,
        columns: Vec<String>,
        #[serde(default)]
        unique: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        method: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        predicate: Option<String>,
    },
    RenameIndex {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        name: String,
        #[serde(rename = "newName")]
        new_name: String,
    },
    DropIndex {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        name: String,
    },
    AddConstraint {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        constraint: ConstraintRequest,
    },
    DropConstraint {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        table: String,
        name: String,
        #[serde(rename = "constraintKind")]
        kind: ConstraintKind,
    },
}

impl StructureActionRequest {
    pub const fn kind(&self) -> StructureActionKind {
        match self {
            Self::CreateTable { .. } => StructureActionKind::CreateTable,
            Self::RenameTable { .. } => StructureActionKind::RenameTable,
            Self::DuplicateTable { .. } => StructureActionKind::DuplicateTable,
            Self::DropTable { .. } => StructureActionKind::DropTable,
            Self::TruncateTable { .. } => StructureActionKind::TruncateTable,
            Self::AddColumn { .. } => StructureActionKind::AddColumn,
            Self::AlterColumn { .. } => StructureActionKind::AlterColumn,
            Self::RenameColumn { .. } => StructureActionKind::RenameColumn,
            Self::DropColumn { .. } => StructureActionKind::DropColumn,
            Self::ReorderColumn { .. } => StructureActionKind::ReorderColumn,
            Self::CreateIndex { .. } => StructureActionKind::CreateIndex,
            Self::RenameIndex { .. } => StructureActionKind::RenameIndex,
            Self::DropIndex { .. } => StructureActionKind::DropIndex,
            Self::AddConstraint { .. } => StructureActionKind::AddConstraint,
            Self::DropConstraint { .. } => StructureActionKind::DropConstraint,
        }
    }

    /// The table name carried by this request, before identifier validation.
    pub fn table_name(&self) -> Option<&str> {
        match self {
            Self::CreateTable { name, .. }
            | Self::RenameTable { name, .. }
            | Self::DuplicateTable { name, .. }
            | Self::DropTable { name, .. }
            | Self::TruncateTable { name, .. } => Some(name),
            Self::AddColumn { table, .. }
            | Self::AlterColumn { table, .. }
            | Self::RenameColumn { table, .. }
            | Self::DropColumn { table, .. }
            | Self::ReorderColumn { table, .. }
            | Self::CreateIndex { table, .. }
            | Self::RenameIndex { table, .. }
            | Self::DropIndex { table, .. }
            | Self::AddConstraint { table, .. }
            | Self::DropConstraint { table, .. } => Some(table),
        }
    }

    /// Validates the complete request before an engine planner sees it.
    pub fn validate(&self) -> Result<ValidatedStructureAction, DatabaseError> {
        match self {
            Self::CreateTable {
                database,
                schema,
                name,
                columns,
                primary_key,
                unique_keys,
                options,
            } => {
                let table = table_ref(database, schema, name, "table")?;
                let columns = validate_columns(columns)?;
                let primary_key = validate_column_list("primary key", primary_key)?;
                if !primary_key.is_empty() {
                    for key in &primary_key {
                        let Some(column) = columns
                            .iter()
                            .find(|column| column.name.as_str() == key.as_str())
                        else {
                            return Err(DatabaseError::validation(format!(
                                "The primary key column {} is not part of the new table.",
                                key
                            )));
                        };
                        if column.nullable {
                            return Err(DatabaseError::validation(format!(
                                "Primary key column {} must be NOT NULL.",
                                key
                            )));
                        }
                    }
                }
                let unique_keys = validate_unique_keys(unique_keys, &columns)?;
                Ok(ValidatedStructureAction::CreateTable(
                    ValidatedCreateTable {
                        table,
                        columns,
                        primary_key,
                        unique_keys,
                        options: validate_table_options(options)?,
                    },
                ))
            }
            Self::RenameTable {
                database,
                schema,
                name,
                new_name,
            } => {
                let table = table_ref(database, schema, name, "table")?;
                let new_name = validate_identifier("new table name", new_name)?;
                ensure_distinct_name(&table.name, &new_name, "table")?;
                Ok(ValidatedStructureAction::RenameTable { table, new_name })
            }
            Self::DuplicateTable {
                database,
                schema,
                name,
                new_name,
            } => {
                let table = table_ref(database, schema, name, "table")?;
                let new_name = validate_identifier("new table name", new_name)?;
                ensure_distinct_name(&table.name, &new_name, "table")?;
                Ok(ValidatedStructureAction::DuplicateTable { table, new_name })
            }
            Self::DropTable {
                database,
                schema,
                name,
            } => Ok(ValidatedStructureAction::DropTable {
                table: table_ref(database, schema, name, "table")?,
            }),
            Self::TruncateTable {
                database,
                schema,
                name,
            } => Ok(ValidatedStructureAction::TruncateTable {
                table: table_ref(database, schema, name, "table")?,
            }),
            Self::AddColumn {
                database,
                schema,
                table,
                column,
                position,
            } => Ok(ValidatedStructureAction::AddColumn {
                table: table_ref(database, schema, table, "table")?,
                column: validate_column(column)?,
                position: position.as_ref().map(validate_position).transpose()?,
            }),
            Self::AlterColumn {
                database,
                schema,
                table,
                name,
                data_type,
                nullable,
                default,
                drop_default,
                comment,
                drop_comment,
            } => {
                if *drop_default && default.is_some() {
                    return Err(DatabaseError::validation(
                        "A column change cannot set and drop a default at the same time.",
                    ));
                }
                if *drop_comment && comment.is_some() {
                    return Err(DatabaseError::validation(
                        "A column change cannot set and drop a comment at the same time.",
                    ));
                }
                let changes = ValidatedColumnChanges {
                    data_type: data_type
                        .as_deref()
                        .map(|value| validate_data_type("column type", value))
                        .transpose()?,
                    nullable: *nullable,
                    default: default.as_ref().map(validate_default).transpose()?,
                    drop_default: *drop_default,
                    comment: comment
                        .as_deref()
                        .map(|value| validate_comment("column comment", value))
                        .transpose()?,
                    drop_comment: *drop_comment,
                };
                if changes.is_empty() {
                    return Err(DatabaseError::validation(
                        "A column change must set a type, nullability, default, or comment.",
                    ));
                }
                Ok(ValidatedStructureAction::AlterColumn {
                    table: table_ref(database, schema, table, "table")?,
                    name: validate_identifier("column name", name)?,
                    changes,
                })
            }
            Self::RenameColumn {
                database,
                schema,
                table,
                name,
                new_name,
            } => {
                let name = validate_identifier("column name", name)?;
                let new_name = validate_identifier("new column name", new_name)?;
                ensure_distinct_name(&name, &new_name, "column")?;
                Ok(ValidatedStructureAction::RenameColumn {
                    table: table_ref(database, schema, table, "table")?,
                    name,
                    new_name,
                })
            }
            Self::DropColumn {
                database,
                schema,
                table,
                name,
            } => Ok(ValidatedStructureAction::DropColumn {
                table: table_ref(database, schema, table, "table")?,
                name: validate_identifier("column name", name)?,
            }),
            Self::ReorderColumn {
                database,
                schema,
                table,
                column,
                position,
            } => {
                let column = validate_column(column)?;
                let position = validate_position(position)?;
                if let ColumnPosition::After { column: after } = &position
                    && after.as_str() == column.name.as_str()
                {
                    return Err(DatabaseError::validation(
                        "A column cannot be positioned after itself.",
                    ));
                }
                Ok(ValidatedStructureAction::ReorderColumn {
                    table: table_ref(database, schema, table, "table")?,
                    column,
                    position,
                })
            }
            Self::CreateIndex {
                database,
                schema,
                table,
                name,
                columns,
                unique,
                method,
                predicate,
            } => {
                let columns = validate_column_list("index columns", columns)?;
                if columns.is_empty() {
                    return Err(DatabaseError::validation(
                        "An index must include at least one column.",
                    ));
                }
                Ok(ValidatedStructureAction::CreateIndex(
                    ValidatedCreateIndex {
                        table: table_ref(database, schema, table, "table")?,
                        name: validate_identifier("index name", name)?,
                        columns,
                        unique: *unique,
                        method: method
                            .as_deref()
                            .map(|value| validate_index_method("index method", value))
                            .transpose()?,
                        predicate: predicate
                            .as_deref()
                            .map(|value| validate_expression("index predicate", value))
                            .transpose()?,
                    },
                ))
            }
            Self::RenameIndex {
                database,
                schema,
                table,
                name,
                new_name,
            } => {
                let name = validate_identifier("index name", name)?;
                let new_name = validate_identifier("new index name", new_name)?;
                ensure_distinct_name(&name, &new_name, "index")?;
                Ok(ValidatedStructureAction::RenameIndex {
                    table: table_ref(database, schema, table, "table")?,
                    name,
                    new_name,
                })
            }
            Self::DropIndex {
                database,
                schema,
                table,
                name,
            } => Ok(ValidatedStructureAction::DropIndex {
                table: table_ref(database, schema, table, "table")?,
                name: validate_identifier("index name", name)?,
            }),
            Self::AddConstraint {
                database,
                schema,
                table,
                constraint,
            } => Ok(ValidatedStructureAction::AddConstraint {
                table: table_ref(database, schema, table, "table")?,
                constraint: validate_constraint(constraint)?,
            }),
            Self::DropConstraint {
                database,
                schema,
                table,
                name,
                kind,
            } => Ok(ValidatedStructureAction::DropConstraint {
                table: table_ref(database, schema, table, "table")?,
                name: validate_identifier("constraint name", name)?,
                kind: *kind,
            }),
        }
    }
}

/// A validated change. Engine planners rely on every identifier and value here
/// already being safe to quote or render.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidatedStructureAction {
    CreateTable(ValidatedCreateTable),
    RenameTable {
        table: TableRef,
        new_name: Identifier,
    },
    DuplicateTable {
        table: TableRef,
        new_name: Identifier,
    },
    DropTable {
        table: TableRef,
    },
    TruncateTable {
        table: TableRef,
    },
    AddColumn {
        table: TableRef,
        column: ValidatedColumnDefinition,
        position: Option<ColumnPosition>,
    },
    AlterColumn {
        table: TableRef,
        name: Identifier,
        changes: ValidatedColumnChanges,
    },
    RenameColumn {
        table: TableRef,
        name: Identifier,
        new_name: Identifier,
    },
    DropColumn {
        table: TableRef,
        name: Identifier,
    },
    ReorderColumn {
        table: TableRef,
        column: ValidatedColumnDefinition,
        position: ColumnPosition,
    },
    CreateIndex(ValidatedCreateIndex),
    RenameIndex {
        table: TableRef,
        name: Identifier,
        new_name: Identifier,
    },
    DropIndex {
        table: TableRef,
        name: Identifier,
    },
    AddConstraint {
        table: TableRef,
        constraint: ValidatedConstraint,
    },
    DropConstraint {
        table: TableRef,
        name: Identifier,
        kind: ConstraintKind,
    },
}

impl ValidatedStructureAction {
    pub const fn kind(&self) -> StructureActionKind {
        match self {
            Self::CreateTable(_) => StructureActionKind::CreateTable,
            Self::RenameTable { .. } => StructureActionKind::RenameTable,
            Self::DuplicateTable { .. } => StructureActionKind::DuplicateTable,
            Self::DropTable { .. } => StructureActionKind::DropTable,
            Self::TruncateTable { .. } => StructureActionKind::TruncateTable,
            Self::AddColumn { .. } => StructureActionKind::AddColumn,
            Self::AlterColumn { .. } => StructureActionKind::AlterColumn,
            Self::RenameColumn { .. } => StructureActionKind::RenameColumn,
            Self::DropColumn { .. } => StructureActionKind::DropColumn,
            Self::ReorderColumn { .. } => StructureActionKind::ReorderColumn,
            Self::CreateIndex(_) => StructureActionKind::CreateIndex,
            Self::RenameIndex { .. } => StructureActionKind::RenameIndex,
            Self::DropIndex { .. } => StructureActionKind::DropIndex,
            Self::AddConstraint { .. } => StructureActionKind::AddConstraint,
            Self::DropConstraint { .. } => StructureActionKind::DropConstraint,
        }
    }

    pub fn table(&self) -> &TableRef {
        match self {
            Self::CreateTable(options) => &options.table,
            Self::RenameTable { table, .. }
            | Self::DuplicateTable { table, .. }
            | Self::DropTable { table }
            | Self::TruncateTable { table }
            | Self::AddColumn { table, .. }
            | Self::AlterColumn { table, .. }
            | Self::RenameColumn { table, .. }
            | Self::DropColumn { table, .. }
            | Self::ReorderColumn { table, .. }
            | Self::CreateIndex(ValidatedCreateIndex { table, .. })
            | Self::RenameIndex { table, .. }
            | Self::DropIndex { table, .. }
            | Self::AddConstraint { table, .. }
            | Self::DropConstraint { table, .. } => table,
        }
    }

    /// The object a preview is about. Table-level actions have no child name.
    pub fn object(&self) -> AffectedObject {
        let table = self.table();
        let mut object = AffectedObject {
            kind: StructureObjectKind::Table,
            database: table.database.as_ref().map(ToString::to_string),
            schema: table.schema.as_ref().map(ToString::to_string),
            table: table.name.as_str().to_string(),
            name: None,
        };
        match self {
            Self::CreateTable(_)
            | Self::RenameTable { .. }
            | Self::DuplicateTable { .. }
            | Self::DropTable { .. }
            | Self::TruncateTable { .. } => {}
            Self::AddColumn { column, .. } | Self::ReorderColumn { column, .. } => {
                object.kind = StructureObjectKind::Column;
                object.name = Some(column.name.as_str().to_string());
            }
            Self::AlterColumn { name, .. }
            | Self::RenameColumn { name, .. }
            | Self::DropColumn { name, .. } => {
                object.kind = StructureObjectKind::Column;
                object.name = Some(name.as_str().to_string());
            }
            Self::CreateIndex(options) => {
                object.kind = StructureObjectKind::Index;
                object.name = Some(options.name.as_str().to_string());
            }
            Self::RenameIndex { name, .. } | Self::DropIndex { name, .. } => {
                object.kind = StructureObjectKind::Index;
                object.name = Some(name.as_str().to_string());
            }
            Self::AddConstraint { constraint, .. } => {
                object.kind = StructureObjectKind::Constraint;
                object.name = Some(constraint.name().as_str().to_string());
            }
            Self::DropConstraint { name, .. } => {
                object.kind = StructureObjectKind::Constraint;
                object.name = Some(name.as_str().to_string());
            }
        }
        object
    }

    /// Capability this action requires, if it maps to one shared action. Some
    /// actions (constraints, generated/identity options) add checks in
    /// [`Self::ensure_supported`].
    pub const fn capability(&self) -> Option<CapabilityAction> {
        match self {
            Self::CreateTable(_) => Some(CapabilityAction::CreateTable),
            Self::RenameTable { .. } => Some(CapabilityAction::RenameTable),
            Self::DuplicateTable { .. } => Some(CapabilityAction::DuplicateTable),
            Self::DropTable { .. } => Some(CapabilityAction::DropTable),
            Self::TruncateTable { .. } => Some(CapabilityAction::TruncateTable),
            Self::AddColumn { .. } => Some(CapabilityAction::AddColumn),
            Self::AlterColumn { .. } => Some(CapabilityAction::EditColumn),
            Self::RenameColumn { .. } => Some(CapabilityAction::RenameColumn),
            Self::DropColumn { .. } => Some(CapabilityAction::DropColumn),
            Self::ReorderColumn { .. } => Some(CapabilityAction::ReorderColumns),
            Self::CreateIndex(_) => Some(CapabilityAction::CreateIndex),
            Self::RenameIndex { .. } => Some(CapabilityAction::RenameIndex),
            Self::DropIndex { .. } => Some(CapabilityAction::DropIndex),
            Self::AddConstraint { .. } | Self::DropConstraint { .. } => None,
        }
    }

    /// Rejects an unsupported engine/version before any DDL is generated.
    pub fn ensure_supported(
        &self,
        capabilities: &DatabaseCapabilities,
    ) -> Result<(), DatabaseError> {
        if let Some(action) = self.capability() {
            capabilities.ensure(action)?;
        }

        match self {
            Self::CreateTable(options) => {
                for column in &options.columns {
                    ensure_column_supported(column, capabilities)?;
                }
            }
            Self::AddColumn {
                column, position, ..
            } => {
                ensure_column_supported(column, capabilities)?;
                if position.is_some() {
                    capabilities.ensure(CapabilityAction::ReorderColumns)?;
                }
            }
            Self::AlterColumn { changes, .. } => {
                if changes.data_type.is_some() {
                    capabilities.ensure(CapabilityAction::EditColumn)?;
                }
            }
            Self::ReorderColumn { .. } => {
                capabilities.ensure(CapabilityAction::ReorderColumns)?;
            }
            Self::CreateIndex(options) => {
                if options.predicate.is_some() {
                    capabilities.ensure(CapabilityAction::CreatePartialIndex)?;
                }
            }
            Self::AddConstraint { constraint, .. } => {
                if matches!(constraint, ValidatedConstraint::Check { .. }) {
                    capabilities.ensure(CapabilityAction::CreateCheckConstraint)?;
                }
            }
            Self::DropConstraint { kind, .. } if *kind == ConstraintKind::Check => {
                capabilities.ensure(CapabilityAction::CreateCheckConstraint)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub const fn destructive(&self) -> bool {
        match self {
            Self::DropTable { .. } | Self::TruncateTable { .. } | Self::DropColumn { .. } => true,
            Self::AlterColumn { changes, .. } => changes.data_type.is_some(),
            _ => false,
        }
    }

    /// Converts a planner result into the shared preview shape.
    pub fn preview(
        &self,
        engine: DatabaseEngine,
        statements: Vec<String>,
        warnings: Vec<String>,
    ) -> StructureDdlPreview {
        let ddl = statements.join(";\n");
        let destructive = self.destructive();
        StructureDdlPreview {
            action: self.kind(),
            engine,
            object: self.object(),
            statements,
            ddl,
            warnings,
            destructive,
            requires_confirmation: destructive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCreateTable {
    pub table: TableRef,
    pub columns: Vec<ValidatedColumnDefinition>,
    pub primary_key: Vec<Identifier>,
    pub unique_keys: Vec<ValidatedUniqueKey>,
    pub options: ValidatedTableOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedUniqueKey {
    pub name: Identifier,
    pub columns: Vec<Identifier>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidatedTableOptions {
    /// MySQL storage engine, for example `InnoDB`.
    pub engine: Option<String>,
    /// MySQL database character set.
    pub charset: Option<String>,
    /// MySQL database collation.
    pub collation: Option<String>,
    /// PostgreSQL tablespace.
    pub tablespace: Option<Identifier>,
    /// PostgreSQL unlogged-table option.
    pub unlogged: bool,
    /// Optional table comment, rendered with engine-specific syntax.
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedColumnDefinition {
    pub name: Identifier,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<ColumnDefault>,
    pub comment: Option<String>,
    pub generated: Option<ValidatedGeneratedColumn>,
    pub identity: ColumnIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedGeneratedColumn {
    pub expression: String,
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnDefault {
    Null,
    String(String),
    Number(String),
    Boolean(bool),
    CurrentTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedColumnChanges {
    pub data_type: Option<String>,
    pub nullable: Option<bool>,
    pub default: Option<ColumnDefault>,
    pub drop_default: bool,
    pub comment: Option<String>,
    pub drop_comment: bool,
}

impl ValidatedColumnChanges {
    pub const fn is_empty(&self) -> bool {
        self.data_type.is_none()
            && self.nullable.is_none()
            && self.default.is_none()
            && !self.drop_default
            && self.comment.is_none()
            && !self.drop_comment
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCreateIndex {
    pub table: TableRef,
    pub name: Identifier,
    pub columns: Vec<Identifier>,
    pub unique: bool,
    pub method: Option<String>,
    pub predicate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedConstraint {
    PrimaryKey {
        name: Identifier,
        columns: Vec<Identifier>,
    },
    Unique {
        name: Identifier,
        columns: Vec<Identifier>,
    },
    ForeignKey {
        name: Identifier,
        columns: Vec<Identifier>,
        referenced_schema: Option<Identifier>,
        referenced_table: Identifier,
        referenced_columns: Vec<Identifier>,
        on_delete: Option<ReferentialAction>,
        on_update: Option<ReferentialAction>,
    },
    Check {
        name: Identifier,
        expression: String,
    },
}

impl ValidatedConstraint {
    pub fn name(&self) -> &Identifier {
        match self {
            Self::PrimaryKey { name, .. }
            | Self::Unique { name, .. }
            | Self::ForeignKey { name, .. }
            | Self::Check { name, .. } => name,
        }
    }

    pub fn columns(&self) -> &[Identifier] {
        match self {
            Self::PrimaryKey { columns, .. }
            | Self::Unique { columns, .. }
            | Self::ForeignKey { columns, .. } => columns,
            Self::Check { .. } => &[],
        }
    }

    pub const fn kind(&self) -> ConstraintKind {
        match self {
            Self::PrimaryKey { .. } => ConstraintKind::PrimaryKey,
            Self::Unique { .. } => ConstraintKind::Unique,
            Self::ForeignKey { .. } => ConstraintKind::ForeignKey,
            Self::Check { .. } => ConstraintKind::Check,
        }
    }
}

/// A typed default value. Expressions are limited to the fixed engine defaults
/// the editor can offer; arbitrary SQL expressions are not accepted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ColumnDefaultRequest {
    Null,
    String { value: String },
    Number { value: String },
    Boolean { value: bool },
    CurrentTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDefinitionRequest {
    pub name: String,
    #[serde(rename = "dataType")]
    pub data_type: String,
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ColumnDefaultRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<GeneratedColumnRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ColumnIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedColumnRequest {
    pub expression: String,
    #[serde(default)]
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniqueKeyRequest {
    pub name: String,
    pub columns: Vec<String>,
}

/// Optional table-level options. Each engine supports a different subset; the
/// engine planner rejects an option the selected server cannot express.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOptionsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tablespace: Option<String>,
    #[serde(default)]
    pub unlogged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ColumnPositionRequest {
    First,
    After { column: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnPosition {
    First,
    After { column: Identifier },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConstraintRequest {
    PrimaryKey {
        name: String,
        columns: Vec<String>,
    },
    Unique {
        name: String,
        columns: Vec<String>,
    },
    ForeignKey {
        name: String,
        columns: Vec<String>,
        #[serde(default, rename = "referencedSchema")]
        referenced_schema: Option<String>,
        #[serde(rename = "referencedTable")]
        referenced_table: String,
        #[serde(default, rename = "referencedColumns")]
        referenced_columns: Vec<String>,
        #[serde(default, rename = "onDelete")]
        on_delete: Option<ReferentialAction>,
        #[serde(default, rename = "onUpdate")]
        on_update: Option<ReferentialAction>,
    },
    Check {
        name: String,
        expression: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferentialAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

/// Structure action names used by previews and future history entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureActionKind {
    CreateTable,
    RenameTable,
    DuplicateTable,
    DropTable,
    TruncateTable,
    AddColumn,
    AlterColumn,
    RenameColumn,
    DropColumn,
    ReorderColumn,
    CreateIndex,
    RenameIndex,
    DropIndex,
    AddConstraint,
    DropConstraint,
}

impl StructureActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateTable => "create_table",
            Self::RenameTable => "rename_table",
            Self::DuplicateTable => "duplicate_table",
            Self::DropTable => "drop_table",
            Self::TruncateTable => "truncate_table",
            Self::AddColumn => "add_column",
            Self::AlterColumn => "alter_column",
            Self::RenameColumn => "rename_column",
            Self::DropColumn => "drop_column",
            Self::ReorderColumn => "reorder_column",
            Self::CreateIndex => "create_index",
            Self::RenameIndex => "rename_index",
            Self::DropIndex => "drop_index",
            Self::AddConstraint => "add_constraint",
            Self::DropConstraint => "drop_constraint",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureObjectKind {
    Table,
    Column,
    Index,
    Constraint,
}

/// The exact object a preview affects. `table` is always present; `name` is the
/// column, index, or constraint name for child-object actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedObject {
    pub kind: StructureObjectKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Generated DDL for one structure action. `statements` is the stable contract;
/// `ddl` is the same statements joined with `;` for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureDdlPreview {
    pub action: StructureActionKind,
    pub engine: DatabaseEngine,
    pub object: AffectedObject,
    pub statements: Vec<String>,
    pub ddl: String,
    pub warnings: Vec<String>,
    pub destructive: bool,
    pub requires_confirmation: bool,
}

/// Verified outcome of one structure action. The statements are safe to show
/// because they contain only validated identifiers and typed values; row data is
/// never selected or returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureActionResult {
    pub action: StructureActionKind,
    pub engine: DatabaseEngine,
    pub object: AffectedObject,
    /// New table name for rename and duplicate actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_table: Option<String>,
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
    pub verified: bool,
    pub verification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TableSummary>,
    pub duration_ms: u64,
}

/// Stateless preview/execute service used by the structure commands.
pub struct DatabaseStructureService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseStructureService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn preview(
        &self,
        profile_id: &str,
        request: StructureActionRequest,
    ) -> Result<StructureDdlPreview, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        preview_structure_with_adapter(adapter.as_ref(), &request)
            .await
            .map_err(AppError::from)
    }

    pub async fn execute(
        &self,
        profile_id: &str,
        request: StructureActionRequest,
        confirmed: bool,
    ) -> Result<StructureActionResult, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        execute_structure_with_adapter(adapter.as_mut(), request, confirmed)
            .await
            .map_err(AppError::from)
    }
}

/// Validates a request and asks the adapter for a preview. Execution always
/// calls this same path first so a caller cannot skip capability checks, the
/// read-only gate, or confirmation.
pub async fn preview_structure_with_adapter(
    adapter: &dyn DatabaseAdapter,
    request: &StructureActionRequest,
) -> Result<StructureDdlPreview, DatabaseError> {
    let validated = request.validate()?;
    let mut preview = adapter.preview_structure_action(&validated).await?;
    append_dependency_warnings(adapter, &validated, &mut preview).await;
    Ok(preview)
}

/// Executes one validated structure action and re-reads metadata before
/// reporting success.
pub async fn execute_structure_with_adapter(
    adapter: &mut dyn DatabaseAdapter,
    request: StructureActionRequest,
    confirmed: bool,
) -> Result<StructureActionResult, DatabaseError> {
    let started = Instant::now();
    let validated = request.validate()?;
    let preview = preview_structure_with_adapter(adapter, &request).await?;

    if !matches!(
        validated,
        ValidatedStructureAction::CreateTable(_)
            | ValidatedStructureAction::RenameTable { .. }
            | ValidatedStructureAction::DuplicateTable { .. }
            | ValidatedStructureAction::TruncateTable { .. }
            | ValidatedStructureAction::DropTable { .. }
            | ValidatedStructureAction::AddColumn { .. }
            | ValidatedStructureAction::AlterColumn { .. }
            | ValidatedStructureAction::RenameColumn { .. }
            | ValidatedStructureAction::DropColumn { .. }
            | ValidatedStructureAction::ReorderColumn { .. }
    ) {
        return Err(DatabaseError::capability(
            "This build executes only table lifecycle and column add, edit, rename, drop, and \
             reorder actions. Index and constraint execution arrives with the structure editor.",
        ));
    }

    if preview.requires_confirmation && !confirmed {
        return Err(DatabaseError::cancelled(format!(
            "The {} action for table {} was cancelled because it was not confirmed.",
            validated.kind().as_str(),
            validated.table().name
        ))
        .with_details(
            "No statement was executed. Review the server, engine, table, dependencies, and the \
             exact preview, then confirm the destructive action.",
        ));
    }

    if adapter.is_read_only() {
        return Err(DatabaseError::permission(
            "The database connection profile is read-only; database structure changes require a \
             writable connection.",
        )
        .with_scope("table", validated.table().name.as_str()));
    }

    ensure_no_blocking_dependencies(adapter, &validated).await?;
    adapter.execute_structure_action(&validated).await?;

    let verification = verify_structure_action(adapter, &validated).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(StructureActionResult {
        action: validated.kind(),
        engine: adapter.engine(),
        object: preview.object,
        target_table: validated.target_table_name().map(str::to_string),
        statements: preview.statements,
        warnings: preview.warnings,
        verified: verification.verified,
        verification: verification.summary,
        verification_error: verification.error,
        metadata: verification.metadata,
        duration_ms,
    })
}

impl ValidatedStructureAction {
    /// New table name for rename and duplicate actions.
    pub fn target_table_name(&self) -> Option<&str> {
        match self {
            Self::RenameTable { new_name, .. } | Self::DuplicateTable { new_name, .. } => {
                Some(new_name.as_str())
            }
            _ => None,
        }
    }
}

async fn ensure_no_blocking_dependencies(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedStructureAction,
) -> Result<(), DatabaseError> {
    let (check_foreign_keys, check_views) = match action {
        ValidatedStructureAction::DropTable { .. } => (true, true),
        ValidatedStructureAction::TruncateTable { .. } => (true, false),
        ValidatedStructureAction::DropColumn { .. } => (true, false),
        _ => return Ok(()),
    };

    let dependencies = match adapter.table_dependencies(action.table(), 50).await {
        Ok(dependencies) => dependencies,
        // Unqualified names resolve through the connection's active
        // database/search_path. The adapter cannot inventory dependencies
        // without an explicit scope, so the engine's own dependency error
        // remains the fallback.
        Err(error)
            if error.kind == DatabaseErrorKind::Validation && action.table().schema.is_none() =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let mut blockers = Vec::new();
    if check_foreign_keys {
        match action {
            ValidatedStructureAction::DropColumn { name, .. } => collect_column_dependency_names(
                &mut blockers,
                "incoming foreign key",
                &dependencies.incoming_foreign_keys,
                name,
            ),
            _ => collect_dependency_names(
                &mut blockers,
                "incoming foreign key",
                &dependencies.incoming_foreign_keys,
            ),
        }
    }
    if check_views {
        collect_dependency_names(
            &mut blockers,
            "dependent view",
            &dependencies.dependent_views,
        );
    }

    if blockers.is_empty() {
        return Ok(());
    }

    let (action_label, object_label) = match action {
        ValidatedStructureAction::DropTable { .. } => ("drop", "table"),
        ValidatedStructureAction::DropColumn { .. } => ("drop", "column"),
        _ => ("truncate", "table"),
    };
    let object_name = match action {
        ValidatedStructureAction::DropColumn { name, .. } => name.as_str(),
        _ => action.table().name.as_str(),
    };
    Err(DatabaseError::dependency(format!(
        "Cannot {action_label} {object_label} {object_name} because {} object(s) depend on it.",
        blockers.len()
    ))
    .with_details(format!(
        "Remove or retarget these dependencies first: {}.",
        blockers.join(", ")
    ))
    .with_scope(object_label, object_name))
}

fn collect_column_dependency_names(
    target: &mut Vec<String>,
    label: &str,
    group: &MetadataValue<Bounded<DependencySummary>>,
    column: &Identifier,
) {
    let Some(group) = group.value() else {
        return;
    };
    for dependency in &group.items {
        if !dependency_references_column(dependency, column) {
            continue;
        }
        let mut name = format!("{label} {}", dependency.name);
        if let Some(table) = dependency.table.as_deref()
            && table != dependency.name
        {
            name.push_str(&format!(" on {table}"));
        }
        target.push(name);
    }
}

fn dependency_references_column(dependency: &DependencySummary, column: &Identifier) -> bool {
    dependency
        .referenced_columns
        .iter()
        .any(|referenced| referenced.eq_ignore_ascii_case(column.as_str()))
}

/// Adds dependency notices to a preview without turning a preview-only request
/// into an execution. Unqualified PostgreSQL names cannot be inspected without
/// a schema; the engine's own dependency error remains the fallback on execute.
async fn append_dependency_warnings(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedStructureAction,
    preview: &mut StructureDdlPreview,
) {
    if !matches!(
        action,
        ValidatedStructureAction::DropTable { .. }
            | ValidatedStructureAction::TruncateTable { .. }
            | ValidatedStructureAction::DropColumn { .. }
    ) {
        return;
    }

    let dependencies = match adapter.table_dependencies(action.table(), 20).await {
        Ok(dependencies) => dependencies,
        Err(error)
            if error.kind == DatabaseErrorKind::Validation && action.table().schema.is_none() =>
        {
            return;
        }
        Err(_) => {
            preview.warnings.push(
                "Dependencies could not be inspected in this preview; the engine may still block \
                 the change or invalidate a dependent object."
                    .to_string(),
            );
            return;
        }
    };

    match action {
        ValidatedStructureAction::DropColumn { name, .. } => {
            if let Some(group) = dependencies.incoming_foreign_keys.value() {
                for dependency in &group.items {
                    if dependency_references_column(dependency, name) {
                        preview.warnings.push(format!(
                            "Dependent foreign key {} on table {} references column {}; the drop \
                             is blocked until that relationship is removed.",
                            dependency.name,
                            dependency.table.as_deref().unwrap_or("another table"),
                            name
                        ));
                    }
                }
            }
            if let Some(group) = dependencies.dependent_views.value() {
                for dependency in &group.items {
                    preview.warnings.push(format!(
                        "Dependent view {} may reference column {}; the engine can block dropping \
                         a column used by a view.",
                        dependency.name, name
                    ));
                }
            }
            if let Some(group) = dependencies.owned_sequences.value() {
                for dependency in &group.items {
                    preview.warnings.push(format!(
                        "Column {} owns sequence {}; dropping the column also drops or detaches \
                         the sequence.",
                        name, dependency.name
                    ));
                }
            }
            if let Some(group) = dependencies.triggers.value() {
                for dependency in &group.items {
                    preview.warnings.push(format!(
                        "Trigger {} is defined on this table and may reference column {}.",
                        dependency.name, name
                    ));
                }
            }
        }
        _ => {
            let mut names = Vec::new();
            collect_dependency_names(
                &mut names,
                "incoming foreign key",
                &dependencies.incoming_foreign_keys,
            );
            collect_dependency_names(&mut names, "dependent view", &dependencies.dependent_views);
            if !names.is_empty() {
                preview.warnings.push(format!(
                    "Dependency check also found: {}. The engine can block the change or leave \
                     these objects invalid.",
                    names.join(", ")
                ));
            }
        }
    }
}

fn collect_dependency_names(
    target: &mut Vec<String>,
    label: &str,
    group: &MetadataValue<Bounded<DependencySummary>>,
) {
    let Some(group) = group.value() else {
        return;
    };
    for dependency in &group.items {
        let mut name = format!("{label} {}", dependency.name);
        if let Some(table) = dependency.table.as_deref()
            && table != dependency.name
        {
            name.push_str(&format!(" on {table}"));
        }
        target.push(name);
    }
}

struct StructureVerification {
    verified: bool,
    summary: String,
    error: Option<String>,
    metadata: Option<TableSummary>,
}

impl StructureVerification {
    fn success(summary: String, metadata: Option<TableSummary>) -> Self {
        Self {
            verified: true,
            summary,
            error: None,
            metadata,
        }
    }

    fn failure(summary: String, error: String) -> Self {
        Self {
            verified: false,
            summary,
            error: Some(error),
            metadata: None,
        }
    }
}

async fn verify_structure_action(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedStructureAction,
) -> StructureVerification {
    match action {
        ValidatedStructureAction::CreateTable(options) => {
            let table = &options.table;
            let expected = options
                .columns
                .iter()
                .map(|column| column.name.as_str().to_string())
                .collect::<Vec<_>>();
            match adapter.table_summary(table).await {
                Ok(Some(metadata)) => {
                    let target = resolved_table_ref(table, &metadata);
                    match verify_table_columns(adapter, &target, &expected).await {
                        Ok(None) => StructureVerification::success(
                            format!(
                                "Metadata re-read confirms table {} exists with the expected {} column(s){}.",
                                table.name,
                                expected.len(),
                                table_metadata_detail(&metadata)
                            ),
                            Some(metadata),
                        ),
                        Ok(Some(problem)) => StructureVerification::failure(
                            format!(
                                "The create action finished, but table {} does not match the expected \
                                 definition.",
                                table.name
                            ),
                            problem,
                        ),
                        Err(error) => verification_error(&table.name, &error),
                    }
                }
                Ok(None) => StructureVerification::failure(
                    format!(
                        "The create action finished, but metadata does not list table {}.",
                        table.name
                    ),
                    "The table was not present in metadata after the create action.".to_string(),
                ),
                Err(error) => verification_error(&table.name, &error),
            }
        }
        ValidatedStructureAction::RenameTable { table, new_name } => {
            let renamed = table_ref_with_name(table, new_name);
            let renamed_result = adapter.table_summary(&renamed).await;
            let previous_result = adapter.table_summary(table).await;
            match (renamed_result, previous_result) {
                (Ok(Some(metadata)), Ok(None)) => StructureVerification::success(
                    format!(
                        "Metadata re-read confirms table {} was renamed to {}.",
                        table.name, new_name
                    ),
                    Some(metadata),
                ),
                (Ok(None), _) => StructureVerification::failure(
                    format!(
                        "The rename action finished, but metadata does not list table {new_name}."
                    ),
                    "The new table name was not present in metadata after the rename action."
                        .to_string(),
                ),
                (Ok(Some(_)), Ok(Some(_))) => StructureVerification::failure(
                    format!(
                        "The rename action finished, but metadata still lists both {} and {}.",
                        table.name, new_name
                    ),
                    "Both the previous and new table names are present after the rename."
                        .to_string(),
                ),
                (Err(error), _) | (_, Err(error)) => verification_error(&table.name, &error),
            }
        }
        ValidatedStructureAction::DuplicateTable { table, new_name } => {
            let duplicate = table_ref_with_name(table, new_name);
            match adapter.table_summary(&duplicate).await {
                Ok(Some(metadata)) => {
                    let target_duplicate = resolved_table_ref(&duplicate, &metadata);
                    let source = match adapter.table_summary(table).await {
                        Ok(Some(source)) => resolved_table_ref(table, &source),
                        Ok(None) => table.clone(),
                        Err(error) => return verification_error(&table.name, &error),
                    };
                    match verify_duplicate_columns(adapter, &source, &target_duplicate).await {
                        Ok(None) => StructureVerification::success(
                            format!(
                                "Metadata re-read confirms table {} was duplicated as {} with the \
                                 same column definition.",
                                table.name, new_name
                            ),
                            Some(metadata),
                        ),
                        Ok(Some(problem)) => StructureVerification::failure(
                            format!(
                                "The duplicate action finished, but table {new_name} does not match \
                                 its source definition."
                            ),
                            problem,
                        ),
                        Err(error) => verification_error(new_name, &error),
                    }
                }
                Ok(None) => StructureVerification::failure(
                    format!(
                        "The duplicate action finished, but metadata does not list table {new_name}."
                    ),
                    "The duplicate table was not present in metadata.".to_string(),
                ),
                Err(error) => verification_error(new_name, &error),
            }
        }
        ValidatedStructureAction::TruncateTable { table } => {
            match adapter.table_summary(table).await {
                Ok(Some(metadata)) => {
                    let rows = metadata.estimated_rows.value().copied();
                    if rows.is_some_and(|rows| rows > 0) {
                        StructureVerification::failure(
                            format!(
                                "The truncate action finished, but metadata still reports {} \
                                 estimated row(s) for table {}.",
                                rows.unwrap_or_default(),
                                table.name
                            ),
                            "The table appears to contain rows after the truncate action."
                                .to_string(),
                        )
                    } else if rows == Some(0) {
                        StructureVerification::success(
                            format!(
                                "Metadata re-read confirms table {} exists and reports no rows{}.",
                                table.name,
                                table_metadata_detail(&metadata)
                            ),
                            Some(metadata),
                        )
                    } else {
                        StructureVerification::success(
                            format!(
                                "Metadata re-read confirms table {} still exists. The engine does \
                                 not report an exact row estimate, so emptiness could not be \
                                 checked.",
                                table.name
                            ),
                            Some(metadata),
                        )
                    }
                }
                Ok(None) => StructureVerification::failure(
                    format!(
                        "The truncate action finished, but metadata does not list table {}.",
                        table.name
                    ),
                    "The table was not present in metadata after the truncate action.".to_string(),
                ),
                Err(error) => verification_error(&table.name, &error),
            }
        }
        ValidatedStructureAction::DropTable { table } => match adapter.table_summary(table).await {
            Ok(None) => StructureVerification::success(
                format!(
                    "Metadata re-read confirms table {} is no longer present.",
                    table.name
                ),
                None,
            ),
            Ok(Some(_)) => StructureVerification::failure(
                format!(
                    "The drop action finished, but metadata still lists table {}.",
                    table.name
                ),
                "The table was still present in metadata after the drop action.".to_string(),
            ),
            Err(error) => verification_error(&table.name, &error),
        },
        ValidatedStructureAction::AddColumn { table, column, .. } => {
            verify_added_column(adapter, table, column).await
        }
        ValidatedStructureAction::AlterColumn {
            table,
            name,
            changes,
        } => verify_altered_column(adapter, table, name, changes).await,
        ValidatedStructureAction::RenameColumn {
            table,
            name,
            new_name,
        } => verify_renamed_column(adapter, table, name, new_name).await,
        ValidatedStructureAction::DropColumn { table, name } => {
            verify_dropped_column(adapter, table, name).await
        }
        ValidatedStructureAction::ReorderColumn {
            table,
            column,
            position,
        } => verify_reordered_column(adapter, table, column, position).await,
        // Index and constraint actions keep the preview-only contract from #170.
        _ => StructureVerification::success(
            "The structure action finished; no automatic metadata verification applies."
                .to_string(),
            None,
        ),
    }
}

async fn read_column_metadata(
    adapter: &dyn DatabaseAdapter,
    table: &TableRef,
) -> Result<Vec<ColumnSummary>, DatabaseError> {
    let page = adapter
        .list_columns(table, PageRequest::new(MAX_PAGE_LIMIT, 0))
        .await?;
    if page.has_more {
        return Err(DatabaseError::validation(format!(
            "The table has more than {MAX_PAGE_LIMIT} columns, so the complete column list cannot \
             be verified."
        )));
    }
    Ok(page.items)
}

fn find_column<'a>(
    columns: &'a [ColumnSummary],
    name: &Identifier,
    engine: DatabaseEngine,
) -> Option<&'a ColumnSummary> {
    columns
        .iter()
        .find(|column| column.name == name.as_str())
        .or_else(|| {
            (engine == DatabaseEngine::Mysql)
                .then(|| {
                    columns
                        .iter()
                        .find(|column| column.name.eq_ignore_ascii_case(name.as_str()))
                })
                .flatten()
        })
}

fn column_metadata_error(
    table: &TableRef,
    column: &Identifier,
    error: &DatabaseError,
) -> StructureVerification {
    StructureVerification::failure(
        format!(
            "The structure action finished, but metadata for column {} on table {} could not be \
             re-read.",
            column, table.name
        ),
        error.message.clone(),
    )
}

async fn verify_added_column(
    adapter: &dyn DatabaseAdapter,
    table: &TableRef,
    expected: &ValidatedColumnDefinition,
) -> StructureVerification {
    let columns = match read_column_metadata(adapter, table).await {
        Ok(columns) => columns,
        Err(error) => return column_metadata_error(table, &expected.name, &error),
    };
    let Some(actual) = find_column(&columns, &expected.name, adapter.engine()) else {
        return StructureVerification::failure(
            format!(
                "The add_column action finished, but metadata does not list column {} on table {}.",
                expected.name, table.name
            ),
            "The new column was not present in metadata after the action.".to_string(),
        );
    };
    if let Some(problem) = compare_column_definition(adapter.engine(), expected, actual) {
        return StructureVerification::failure(
            format!(
                "The add_column action finished, but column {} on table {} does not match the \
                 requested definition.",
                expected.name, table.name
            ),
            problem,
        );
    }

    StructureVerification::success(
        format!(
            "Metadata re-read confirms column {} was added to table {} with the requested type, \
             nullability, default, identity/generated options, and comment.",
            expected.name, table.name
        ),
        adapter.table_summary(table).await.ok().flatten(),
    )
}

async fn verify_altered_column(
    adapter: &dyn DatabaseAdapter,
    table: &TableRef,
    name: &Identifier,
    changes: &ValidatedColumnChanges,
) -> StructureVerification {
    let columns = match read_column_metadata(adapter, table).await {
        Ok(columns) => columns,
        Err(error) => return column_metadata_error(table, name, &error),
    };
    let Some(actual) = find_column(&columns, name, adapter.engine()) else {
        return StructureVerification::failure(
            format!(
                "The alter_column action finished, but metadata does not list column {} on table \
                 {}.",
                name, table.name
            ),
            "The edited column was not present in metadata after the action.".to_string(),
        );
    };
    if let Some(problem) = compare_column_changes(adapter.engine(), changes, actual) {
        return StructureVerification::failure(
            format!(
                "The alter_column action finished, but column {} on table {} does not match the \
                 requested changes.",
                name, table.name
            ),
            problem,
        );
    }

    StructureVerification::success(
        format!(
            "Metadata re-read confirms column {} on table {} now has the requested type, \
             nullability, default, and comment.",
            name, table.name
        ),
        adapter.table_summary(table).await.ok().flatten(),
    )
}

async fn verify_renamed_column(
    adapter: &dyn DatabaseAdapter,
    table: &TableRef,
    name: &Identifier,
    new_name: &Identifier,
) -> StructureVerification {
    let columns = match read_column_metadata(adapter, table).await {
        Ok(columns) => columns,
        Err(error) => return column_metadata_error(table, new_name, &error),
    };
    if find_column(&columns, new_name, adapter.engine()).is_none() {
        return StructureVerification::failure(
            format!(
                "The rename_column action finished, but metadata does not list column {} on table \
                 {}.",
                new_name, table.name
            ),
            "The new column name was not present in metadata after the rename.".to_string(),
        );
    }
    if find_column(&columns, name, adapter.engine()).is_some() {
        return StructureVerification::failure(
            format!(
                "The rename_column action finished, but metadata still lists both {} and {} on \
                 table {}.",
                name, new_name, table.name
            ),
            "Both the previous and the new column name are present after the rename.".to_string(),
        );
    }

    StructureVerification::success(
        format!(
            "Metadata re-read confirms column {} on table {} was renamed to {}.",
            name, table.name, new_name
        ),
        adapter.table_summary(table).await.ok().flatten(),
    )
}

async fn verify_dropped_column(
    adapter: &dyn DatabaseAdapter,
    table: &TableRef,
    name: &Identifier,
) -> StructureVerification {
    match adapter.table_summary(table).await {
        Ok(None) => {
            return StructureVerification::failure(
                format!(
                    "The drop_column action finished, but table {} is no longer listed in \
                     metadata.",
                    table.name
                ),
                "The table itself disappeared while verifying the column drop.".to_string(),
            );
        }
        Ok(Some(_)) => {}
        Err(error) => return column_metadata_error(table, name, &error),
    }

    let columns = match read_column_metadata(adapter, table).await {
        Ok(columns) => columns,
        Err(error) => return column_metadata_error(table, name, &error),
    };
    if find_column(&columns, name, adapter.engine()).is_some() {
        return StructureVerification::failure(
            format!(
                "The drop_column action finished, but metadata still lists column {} on table {}.",
                name, table.name
            ),
            "The column was still present in metadata after the drop.".to_string(),
        );
    }

    StructureVerification::success(
        format!(
            "Metadata re-read confirms column {} is no longer present on table {}.",
            name, table.name
        ),
        adapter.table_summary(table).await.ok().flatten(),
    )
}

async fn verify_reordered_column(
    adapter: &dyn DatabaseAdapter,
    table: &TableRef,
    expected: &ValidatedColumnDefinition,
    position: &ColumnPosition,
) -> StructureVerification {
    let columns = match read_column_metadata(adapter, table).await {
        Ok(columns) => columns,
        Err(error) => return column_metadata_error(table, &expected.name, &error),
    };
    let Some(actual) = find_column(&columns, &expected.name, adapter.engine()) else {
        return StructureVerification::failure(
            format!(
                "The reorder_column action finished, but metadata does not list column {} on table \
                 {}.",
                expected.name, table.name
            ),
            "The reordered column was not present in metadata after the action.".to_string(),
        );
    };
    let expected_ordinal = match position {
        ColumnPosition::First => 1,
        ColumnPosition::After { column } => {
            let Some(anchor) = find_column(&columns, column, adapter.engine()) else {
                return StructureVerification::failure(
                    format!(
                        "The reorder_column action finished, but metadata does not list anchor \
                         column {} on table {}.",
                        column, table.name
                    ),
                    "The column that should follow the reordered column was not found.".to_string(),
                );
            };
            anchor.ordinal_position.saturating_add(1)
        }
    };
    if actual.ordinal_position != expected_ordinal {
        return StructureVerification::failure(
            format!(
                "The reorder_column action finished, but column {} on table {} reports ordinal \
                 position {} instead of {}.",
                expected.name, table.name, actual.ordinal_position, expected_ordinal
            ),
            "The engine did not place the column at the requested ordinal position.".to_string(),
        );
    }
    if let Some(problem) = compare_column_definition(adapter.engine(), expected, actual) {
        return StructureVerification::failure(
            format!(
                "The reorder_column action finished, but column {} on table {} changed definition.",
                expected.name, table.name
            ),
            problem,
        );
    }

    StructureVerification::success(
        format!(
            "Metadata re-read confirms column {} is at ordinal position {} on table {}.",
            expected.name, expected_ordinal, table.name
        ),
        adapter.table_summary(table).await.ok().flatten(),
    )
}

fn compare_column_definition(
    engine: DatabaseEngine,
    expected: &ValidatedColumnDefinition,
    actual: &ColumnSummary,
) -> Option<String> {
    if !data_types_match(engine, &expected.data_type, &actual.data_type) {
        return Some(format!(
            "Expected type {}, but metadata reports {}.",
            expected.data_type, actual.data_type
        ));
    }
    if expected.nullable != actual.nullable {
        return Some(format!(
            "Expected nullable={}, but metadata reports nullable={}.",
            expected.nullable, actual.nullable
        ));
    }
    if let Some(actual_default) = actual.default_value.value()
        && !defaults_match(expected.default.as_ref(), actual_default.as_deref())
    {
        return Some(format!(
            "Expected default {}, but metadata reports {}.",
            describe_expected_default(expected.default.as_ref()),
            actual_default.as_deref().unwrap_or("no default")
        ));
    }
    if let Some(actual_comment) = actual.comment.value()
        && expected.comment.as_deref() != actual_comment.as_deref()
    {
        return Some(format!(
            "Expected comment {}, but metadata reports {}.",
            expected.comment.as_deref().unwrap_or("no comment"),
            actual_comment.as_deref().unwrap_or("no comment")
        ));
    }
    if let Some(actual_generated) = actual.generated.value()
        && *actual_generated != expected.generated.is_some()
    {
        return Some(format!(
            "Expected generated={}, but metadata reports generated={actual_generated}.",
            expected.generated.is_some()
        ));
    }
    if let Some(actual_expression) = actual.generation_expression.value()
        && let Some(expected_generated) = expected.generated.as_ref()
    {
        let actual_expression = actual_expression.as_deref().unwrap_or_default();
        if normalize_expression(&expected_generated.expression)
            != normalize_expression(actual_expression)
        {
            return Some(format!(
                "Expected generation expression {}, but metadata reports {}.",
                expected_generated.expression, actual_expression
            ));
        }
    }
    if let Some(actual_identity) = actual.identity.value()
        && *actual_identity != expected.identity
    {
        return Some(format!(
            "Expected identity {:?}, but metadata reports {:?}.",
            expected.identity, actual_identity
        ));
    }
    None
}

fn compare_column_changes(
    engine: DatabaseEngine,
    changes: &ValidatedColumnChanges,
    actual: &ColumnSummary,
) -> Option<String> {
    if let Some(expected) = changes.data_type.as_deref()
        && !data_types_match(engine, expected, &actual.data_type)
    {
        return Some(format!(
            "Expected type {expected}, but metadata reports {}.",
            actual.data_type
        ));
    }
    if let Some(expected) = changes.nullable
        && expected != actual.nullable
    {
        return Some(format!(
            "Expected nullable={expected}, but metadata reports nullable={}.",
            actual.nullable
        ));
    }
    if (changes.default.is_some() || changes.drop_default)
        && let Some(actual_default) = actual.default_value.value()
        && !defaults_match(changes.default.as_ref(), actual_default.as_deref())
    {
        return Some(format!(
            "Expected default {}, but metadata reports {}.",
            describe_expected_default(changes.default.as_ref()),
            actual_default.as_deref().unwrap_or("no default")
        ));
    }
    if (changes.comment.is_some() || changes.drop_comment)
        && let Some(actual_comment) = actual.comment.value()
        && changes.comment.as_deref() != actual_comment.as_deref()
    {
        return Some(format!(
            "Expected comment {}, but metadata reports {}.",
            changes.comment.as_deref().unwrap_or("no comment"),
            actual_comment.as_deref().unwrap_or("no comment")
        ));
    }
    None
}

fn data_types_match(engine: DatabaseEngine, expected: &str, actual: &str) -> bool {
    normalize_data_type(engine, expected) == normalize_data_type(engine, actual)
}

fn normalize_data_type(engine: DatabaseEngine, value: &str) -> String {
    let mut normalized = value
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized = normalized
        .replace(" (", "(")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" ,", ",")
        .replace(", ", ",");
    if engine == DatabaseEngine::Postgresql {
        normalized = normalized
            .replace("character varying", "varchar")
            .replace("character(", "char(")
            .replace("timestamp without time zone", "timestamp")
            .replace("timestamp with time zone", "timestamptz")
            .replace("time without time zone", "time")
            .replace("time with time zone", "timetz")
            .replace("integer", "int")
            .replace("decimal", "numeric")
            .replace("boolean", "bool");
    } else {
        for base in ["tinyint", "smallint", "mediumint", "int", "bigint"] {
            if let Some(rest) = normalized.strip_prefix(base)
                && let Some(after_width) = rest.strip_prefix('(')
                && let Some((_, tail)) = after_width.split_once(')')
            {
                normalized = format!("{base}{tail}");
                break;
            }
        }
    }
    normalized
}

fn defaults_match(expected: Option<&ColumnDefault>, actual: Option<&str>) -> bool {
    let actual = actual.map(str::trim).filter(|value| !value.is_empty());
    match expected {
        None | Some(ColumnDefault::Null) => {
            actual.is_none() || actual.is_some_and(|value| value.eq_ignore_ascii_case("null"))
        }
        Some(ColumnDefault::String(value)) => actual.is_some_and(|actual| {
            sql_string_literal_value(actual).map_or_else(
                || actual.eq_ignore_ascii_case(value),
                |literal| literal == *value,
            )
        }),
        Some(ColumnDefault::Number(value)) => actual.is_some_and(|actual| {
            let actual = strip_default_cast(actual);
            actual == value
                || actual
                    .parse::<f64>()
                    .ok()
                    .zip(value.parse::<f64>().ok())
                    .is_some_and(|(left, right)| (left - right).abs() < f64::EPSILON)
        }),
        Some(ColumnDefault::Boolean(value)) => actual.is_some_and(|actual| {
            let actual = strip_default_cast(actual).to_ascii_lowercase();
            match actual.as_str() {
                "1" | "true" | "b'1'" => *value,
                "0" | "false" | "b'0'" => !*value,
                _ => false,
            }
        }),
        Some(ColumnDefault::CurrentTimestamp) => actual.is_some_and(|actual| {
            let actual = actual.to_ascii_lowercase().replace(['_', ' '], "");
            actual.contains("currenttimestamp")
                || actual.contains("now()")
                || actual.contains("localtimestamp")
        }),
    }
}

fn strip_default_cast(value: &str) -> &str {
    let mut value = value.trim();
    while value.starts_with('(') && value.ends_with(')') {
        value = value[1..value.len().saturating_sub(1)].trim();
    }
    value.split("::").next().unwrap_or(value).trim()
}

fn sql_string_literal_value(value: &str) -> Option<String> {
    let value = value.trim();
    let start = value.find('\'')?;
    let mut literal = String::new();
    let mut characters = value[start + 1..].chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\'' => {
                if characters.peek() == Some(&'\'') {
                    literal.push('\'');
                    characters.next();
                } else {
                    return Some(literal);
                }
            }
            '\\' => {
                if let Some(escaped) = characters.next() {
                    literal.push(escaped);
                }
            }
            _ => literal.push(character),
        }
    }
    None
}

fn describe_expected_default(expected: Option<&ColumnDefault>) -> String {
    match expected {
        None | Some(ColumnDefault::Null) => "no default".to_string(),
        Some(ColumnDefault::String(value)) => format!("string {value:?}"),
        Some(ColumnDefault::Number(value)) => value.clone(),
        Some(ColumnDefault::Boolean(value)) => value.to_string(),
        Some(ColumnDefault::CurrentTimestamp) => "CURRENT_TIMESTAMP".to_string(),
    }
}

fn normalize_expression(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '"' | '`'))
        .flat_map(char::to_lowercase)
        .collect()
}

async fn verify_table_columns(
    adapter: &dyn DatabaseAdapter,
    table: &TableRef,
    expected: &[String],
) -> Result<Option<String>, DatabaseError> {
    let page = adapter
        .list_columns(table, PageRequest::new(MAX_PAGE_LIMIT, 0))
        .await?;
    let actual = page
        .items
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    compare_column_names(expected, &actual, page.has_more)
}

async fn verify_duplicate_columns(
    adapter: &dyn DatabaseAdapter,
    source: &TableRef,
    duplicate: &TableRef,
) -> Result<Option<String>, DatabaseError> {
    let source_page = adapter
        .list_columns(source, PageRequest::new(MAX_PAGE_LIMIT, 0))
        .await?;
    let duplicate_page = adapter
        .list_columns(duplicate, PageRequest::new(MAX_PAGE_LIMIT, 0))
        .await?;
    let source_names = source_page
        .items
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let duplicate_names = duplicate_page
        .items
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    if let Some(problem) = compare_column_names(&source_names, &duplicate_names, false)? {
        return Ok(Some(problem));
    }
    if source_page.has_more != duplicate_page.has_more {
        return Ok(Some(
            "The duplicate and source tables report different column counts.".to_string(),
        ));
    }
    Ok(None)
}

fn compare_column_names(
    expected: &[String],
    actual: &[String],
    has_more: bool,
) -> Result<Option<String>, DatabaseError> {
    if has_more {
        return Ok(Some(format!(
            "The table has more than {MAX_PAGE_LIMIT} columns, so the complete column list could \
             not be verified."
        )));
    }
    if expected.len() != actual.len() {
        return Ok(Some(format!(
            "Expected {} column(s), but metadata reports {}.",
            expected.len(),
            actual.len()
        )));
    }
    for (expected, actual) in expected.iter().zip(actual) {
        if !expected.eq_ignore_ascii_case(actual) {
            return Ok(Some(format!(
                "Expected column {expected}, but metadata reports {actual}."
            )));
        }
    }
    Ok(None)
}

fn table_ref_with_name(table: &TableRef, name: &Identifier) -> TableRef {
    TableRef {
        database: table.database.clone(),
        schema: table.schema.clone(),
        name: name.clone(),
    }
}

fn resolved_table_ref(table: &TableRef, metadata: &TableSummary) -> TableRef {
    TableRef {
        database: metadata
            .database
            .as_deref()
            .map(Identifier::new)
            .transpose()
            .ok()
            .flatten()
            .or_else(|| table.database.clone()),
        schema: metadata
            .schema
            .as_deref()
            .map(Identifier::new)
            .transpose()
            .ok()
            .flatten()
            .or_else(|| table.schema.clone()),
        name: table.name.clone(),
    }
}

fn verification_error(name: &Identifier, error: &DatabaseError) -> StructureVerification {
    StructureVerification::failure(
        format!(
            "The structure action finished, but metadata for {} could not be re-read.",
            name
        ),
        error.message.clone(),
    )
}

fn table_metadata_detail(metadata: &TableSummary) -> String {
    let mut details = Vec::new();
    if let Some(engine) = metadata.metadata.engine.value() {
        details.push(format!("engine {engine}"));
    }
    if let Some(rows) = metadata.estimated_rows.value() {
        details.push(format!("estimated rows {rows}"));
    }
    if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join(", "))
    }
}

fn table_ref(
    database: &Option<String>,
    schema: &Option<String>,
    table: &str,
    label: &str,
) -> Result<TableRef, DatabaseError> {
    Ok(TableRef {
        database: database
            .as_deref()
            .map(|value| validate_identifier("database name", value))
            .transpose()?,
        schema: schema
            .as_deref()
            .map(|value| validate_identifier("schema name", value))
            .transpose()?,
        name: validate_identifier(label, table)?,
    })
}

fn validate_identifier(label: &str, value: &str) -> Result<Identifier, DatabaseError> {
    if value.trim() != value {
        return Err(DatabaseError::validation(format!(
            "The {label} must not start or end with whitespace."
        )));
    }
    Identifier::new(value).map_err(|error| {
        DatabaseError::validation(format!("The {label} is not valid.")).with_details(error.message)
    })
}

fn ensure_distinct_name(
    current: &Identifier,
    new_name: &Identifier,
    label: &str,
) -> Result<(), DatabaseError> {
    if current == new_name {
        return Err(DatabaseError::validation(format!(
            "The new {label} name must differ from the current name."
        )));
    }
    Ok(())
}

fn validate_unique_keys(
    keys: &[UniqueKeyRequest],
    columns: &[ValidatedColumnDefinition],
) -> Result<Vec<ValidatedUniqueKey>, DatabaseError> {
    if keys.len() > MAX_COLUMN_LIST_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A new table cannot include more than {MAX_COLUMN_LIST_LENGTH} unique keys in one preview."
        )));
    }
    let mut validated = Vec::with_capacity(keys.len());
    for key in keys {
        let name = validate_identifier("unique key name", &key.name)?;
        let key_columns = validate_column_list("unique key columns", &key.columns)?;
        if key_columns.is_empty() {
            return Err(DatabaseError::validation(
                "A unique key must include at least one column.",
            ));
        }
        for key_column in &key_columns {
            let Some(_column) = columns
                .iter()
                .find(|column| column.name.as_str() == key_column.as_str())
            else {
                return Err(DatabaseError::validation(format!(
                    "The unique key column {key_column} is not part of the new table."
                )));
            };
        }
        if validated
            .iter()
            .any(|existing: &ValidatedUniqueKey| existing.name == name)
        {
            return Err(DatabaseError::validation(format!(
                "The unique key {name} appears more than once in the table definition."
            )));
        }
        validated.push(ValidatedUniqueKey {
            name,
            columns: key_columns,
        });
    }
    Ok(validated)
}

fn validate_table_options(
    options: &Option<TableOptionsRequest>,
) -> Result<ValidatedTableOptions, DatabaseError> {
    let Some(options) = options else {
        return Ok(ValidatedTableOptions::default());
    };
    let comment = options
        .comment
        .as_deref()
        .map(|value| validate_comment("table comment", value))
        .transpose()?;
    Ok(ValidatedTableOptions {
        engine: validate_option_name("storage engine", options.engine.as_deref())?,
        charset: validate_option_name("character set", options.charset.as_deref())?,
        collation: validate_option_name("collation", options.collation.as_deref())?,
        tablespace: options
            .tablespace
            .as_deref()
            .map(|value| validate_identifier("tablespace", value))
            .transpose()?,
        unlogged: options.unlogged,
        comment,
    })
}

fn validate_option_name(label: &str, value: Option<&str>) -> Result<Option<String>, DatabaseError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_DATA_TYPE_LENGTH {
        return Err(DatabaseError::validation(format!(
            "The table {label} must be between 1 and {MAX_DATA_TYPE_LENGTH} characters."
        )));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(DatabaseError::validation(format!(
            "The table {label} contains unsupported characters."
        )));
    }
    Ok(Some(value.to_string()))
}

fn validate_columns(
    columns: &[ColumnDefinitionRequest],
) -> Result<Vec<ValidatedColumnDefinition>, DatabaseError> {
    if columns.is_empty() {
        return Err(DatabaseError::validation(
            "A new table must include at least one column.",
        ));
    }
    if columns.len() > MAX_COLUMN_LIST_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A table definition cannot include more than {MAX_COLUMN_LIST_LENGTH} columns in one preview."
        )));
    }
    let mut validated = Vec::with_capacity(columns.len());
    for column in columns {
        let column = validate_column(column)?;
        if validated
            .iter()
            .any(|existing: &ValidatedColumnDefinition| {
                existing
                    .name
                    .as_str()
                    .eq_ignore_ascii_case(column.name.as_str())
            })
        {
            return Err(DatabaseError::validation(format!(
                "The column {} appears more than once in the table definition.",
                column.name
            )));
        }
        validated.push(column);
    }
    Ok(validated)
}

fn validate_column(
    column: &ColumnDefinitionRequest,
) -> Result<ValidatedColumnDefinition, DatabaseError> {
    let name = validate_identifier("column name", &column.name)?;
    let data_type = validate_data_type("column type", &column.data_type)?;
    let comment = column
        .comment
        .as_deref()
        .map(|value| validate_comment("column comment", value))
        .transpose()?;
    let identity = column.identity.unwrap_or(ColumnIdentity::None);
    let generated = column
        .generated
        .as_ref()
        .map(|generated| {
            Ok(ValidatedGeneratedColumn {
                expression: validate_expression(
                    "generated column expression",
                    &generated.expression,
                )?,
                stored: generated.stored,
            })
        })
        .transpose()?;
    let default = column.default.as_ref().map(validate_default).transpose()?;

    if generated.is_some() && default.is_some() {
        return Err(DatabaseError::validation(
            "A generated column cannot also have a default value.",
        ));
    }
    if generated.is_some() && identity != ColumnIdentity::None {
        return Err(DatabaseError::validation(
            "A generated column cannot also have an identity or auto-increment option.",
        ));
    }
    if default.is_some() && identity != ColumnIdentity::None {
        return Err(DatabaseError::validation(
            "An identity or auto-increment column cannot also have a default value.",
        ));
    }
    if identity != ColumnIdentity::None && column.nullable {
        return Err(DatabaseError::validation(
            "An identity or auto-increment column must be NOT NULL.",
        ));
    }

    Ok(ValidatedColumnDefinition {
        name,
        data_type,
        nullable: column.nullable,
        default,
        comment,
        generated,
        identity,
    })
}

fn validate_default(default: &ColumnDefaultRequest) -> Result<ColumnDefault, DatabaseError> {
    match default {
        ColumnDefaultRequest::Null => Ok(ColumnDefault::Null),
        ColumnDefaultRequest::String { value } => {
            if value.len() > MAX_DEFAULT_STRING_LENGTH {
                return Err(DatabaseError::validation(format!(
                    "A default string cannot exceed {MAX_DEFAULT_STRING_LENGTH} characters."
                )));
            }
            if value.contains('\0') || value.chars().any(char::is_control) {
                return Err(DatabaseError::validation(
                    "A default string cannot contain NUL or control characters.",
                ));
            }
            Ok(ColumnDefault::String(value.clone()))
        }
        ColumnDefaultRequest::Number { value } => {
            if !is_numeric_literal(value) {
                return Err(DatabaseError::validation(
                    "A numeric default must be a plain integer or decimal number.",
                ));
            }
            Ok(ColumnDefault::Number(value.clone()))
        }
        ColumnDefaultRequest::Boolean { value } => Ok(ColumnDefault::Boolean(*value)),
        ColumnDefaultRequest::CurrentTimestamp => Ok(ColumnDefault::CurrentTimestamp),
    }
}

fn validate_position(position: &ColumnPositionRequest) -> Result<ColumnPosition, DatabaseError> {
    match position {
        ColumnPositionRequest::First => Ok(ColumnPosition::First),
        ColumnPositionRequest::After { column } => Ok(ColumnPosition::After {
            column: validate_identifier("column name after", column)?,
        }),
    }
}

fn validate_constraint(
    constraint: &ConstraintRequest,
) -> Result<ValidatedConstraint, DatabaseError> {
    match constraint {
        ConstraintRequest::PrimaryKey { name, columns } => {
            let columns = validate_column_list("primary key columns", columns)?;
            require_columns("primary key", &columns)?;
            Ok(ValidatedConstraint::PrimaryKey {
                name: validate_identifier("constraint name", name)?,
                columns,
            })
        }
        ConstraintRequest::Unique { name, columns } => {
            let columns = validate_column_list("unique constraint columns", columns)?;
            require_columns("unique constraint", &columns)?;
            Ok(ValidatedConstraint::Unique {
                name: validate_identifier("constraint name", name)?,
                columns,
            })
        }
        ConstraintRequest::ForeignKey {
            name,
            columns,
            referenced_schema,
            referenced_table,
            referenced_columns,
            on_delete,
            on_update,
        } => {
            let columns = validate_column_list("foreign key columns", columns)?;
            require_columns("foreign key", &columns)?;
            let referenced_columns =
                validate_column_list("referenced columns", referenced_columns)?;
            require_columns("foreign key reference", &referenced_columns)?;
            if columns.len() != referenced_columns.len() {
                return Err(DatabaseError::validation(
                    "A foreign key must reference the same number of columns it defines.",
                ));
            }
            Ok(ValidatedConstraint::ForeignKey {
                name: validate_identifier("constraint name", name)?,
                columns,
                referenced_schema: referenced_schema
                    .as_deref()
                    .map(|value| validate_identifier("referenced schema", value))
                    .transpose()?,
                referenced_table: validate_identifier("referenced table", referenced_table)?,
                referenced_columns,
                on_delete: *on_delete,
                on_update: *on_update,
            })
        }
        ConstraintRequest::Check { name, expression } => Ok(ValidatedConstraint::Check {
            name: validate_identifier("constraint name", name)?,
            expression: validate_expression("check constraint expression", expression)?,
        }),
    }
}

fn require_columns(label: &str, columns: &[Identifier]) -> Result<(), DatabaseError> {
    if columns.is_empty() {
        return Err(DatabaseError::validation(format!(
            "A {label} must include at least one column."
        )));
    }
    if columns.len() > MAX_COLUMN_LIST_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot include more than {MAX_COLUMN_LIST_LENGTH} columns."
        )));
    }
    Ok(())
}

fn validate_column_list(label: &str, columns: &[String]) -> Result<Vec<Identifier>, DatabaseError> {
    if columns.len() > MAX_COLUMN_LIST_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot include more than {MAX_COLUMN_LIST_LENGTH} columns."
        )));
    }
    let mut identifiers = Vec::with_capacity(columns.len());
    for column in columns {
        let identifier = validate_identifier(label, column)?;
        if identifiers.iter().any(|existing: &Identifier| {
            existing.as_str().eq_ignore_ascii_case(identifier.as_str())
        }) {
            return Err(DatabaseError::validation(format!(
                "The {label} contains the column {identifier} more than once."
            )));
        }
        identifiers.push(identifier);
    }
    Ok(identifiers)
}

fn validate_data_type(label: &str, value: &str) -> Result<String, DatabaseError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot be empty."
        )));
    }
    if value.len() > MAX_DATA_TYPE_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot exceed {MAX_DATA_TYPE_LENGTH} characters."
        )));
    }
    let first = value.chars().next().unwrap_or(' ');
    if !first.is_ascii_alphabetic() {
        return Err(DatabaseError::validation(format!(
            "A {label} must start with a letter."
        )));
    }
    if value.contains("  ") {
        return Err(DatabaseError::validation(format!(
            "The {label} must not contain repeated spaces."
        )));
    }

    // The type grammar deliberately does not accept statement separators or
    // commas outside a length/precision list. A type such as
    // `bigint, ADD COLUMN evil int` must fail before it reaches a planner.
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut depth = 0_u32;
    let mut paren_has_digit = false;
    let mut previous = ' ';
    while index < bytes.len() {
        let character = bytes[index] as char;
        match character {
            'a'..='z' | 'A'..='Z' | '_' | '0'..='9' => {
                if depth > 0 && !character.is_ascii_digit() {
                    return Err(invalid_data_type(label));
                }
                if depth > 0 && character.is_ascii_digit() {
                    paren_has_digit = true;
                }
            }
            ' ' => {
                if depth > 0 {
                    if !paren_has_digit {
                        return Err(invalid_data_type(label));
                    }
                } else if matches!(previous, ' ' | '.' | '(') || index == 0 {
                    return Err(invalid_data_type(label));
                }
            }
            '.' => {
                if depth > 0
                    || !previous.is_ascii_alphanumeric() && previous != '_'
                    || !bytes
                        .get(index + 1)
                        .is_some_and(|next| next.is_ascii_alphanumeric() || *next == b'_')
                {
                    return Err(invalid_data_type(label));
                }
            }
            '(' => {
                if depth > 0 {
                    return Err(invalid_data_type(label));
                }
                depth = 1;
                paren_has_digit = false;
            }
            ')' => {
                if depth == 0 || !paren_has_digit {
                    return Err(invalid_data_type(label));
                }
                depth = 0;
            }
            ',' => {
                if depth == 0 {
                    return Err(invalid_data_type(label));
                }
            }
            '[' => {
                if depth > 0 || bytes.get(index + 1) != Some(&b']') {
                    return Err(invalid_data_type(label));
                }
                index += 1;
            }
            ']' => return Err(invalid_data_type(label)),
            _ => return Err(invalid_data_type(label)),
        }
        previous = character;
        index += 1;
    }
    if depth != 0 || value.ends_with('.') || value.ends_with(' ') {
        return Err(invalid_data_type(label));
    }

    Ok(value.to_string())
}

fn invalid_data_type(label: &str) -> DatabaseError {
    DatabaseError::validation(format!("The {label} contains unsupported characters.")).with_details(
        "Use an engine type name with optional length or precision, for example varchar(255), \
         numeric(10,2), timestamp(3) with time zone, bigint unsigned, or bigint[].",
    )
}

fn validate_comment(label: &str, value: &str) -> Result<String, DatabaseError> {
    if value.len() > MAX_COLUMN_COMMENT_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot exceed {MAX_COLUMN_COMMENT_LENGTH} characters."
        )));
    }
    if value.contains('\0') || value.chars().any(char::is_control) {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot contain NUL or control characters."
        )));
    }
    Ok(value.to_string())
}

fn validate_index_method(label: &str, value: &str) -> Result<String, DatabaseError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_INDEX_METHOD_LENGTH {
        return Err(DatabaseError::validation(format!(
            "An {label} must be between 1 and {MAX_INDEX_METHOD_LENGTH} characters."
        )));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphabetic())
    {
        return Err(DatabaseError::validation(format!(
            "An {label} must contain only letters."
        )));
    }
    Ok(value.to_ascii_lowercase())
}

/// Validates a generated-column, check, or predicate expression.
///
/// Expressions are SQL fragments, not values, so they cannot be bound like a
/// parameter. The editor therefore accepts only a balanced expression: no
/// statement separator, comment, unbalanced quote, unbalanced parenthesis, or
/// top-level comma. That prevents a value from escaping its expression context
/// while still allowing ordinary function calls and arithmetic.
fn validate_expression(label: &str, value: &str) -> Result<String, DatabaseError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot be empty."
        )));
    }
    if value.len() > MAX_COLUMN_EXPRESSION_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot exceed {MAX_COLUMN_EXPRESSION_LENGTH} characters."
        )));
    }
    if value.contains('\0') || value.chars().any(char::is_control) {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot contain NUL or control characters."
        )));
    }
    if value.contains(';') || value.contains("--") || value.contains("/*") || value.contains("*/") {
        return Err(DatabaseError::validation(format!(
            "A {label} cannot contain statement separators or comments."
        )));
    }

    let mut depth = 0_u32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in value.chars() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' | '`' => quote = Some(character),
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Err(DatabaseError::validation(format!(
                        "The {label} has an unmatched closing parenthesis."
                    )));
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                return Err(DatabaseError::validation(format!(
                    "The {label} must be a single expression, not a comma-separated list."
                )));
            }
            _ => {}
        }
    }
    if quote.is_some() {
        return Err(DatabaseError::validation(format!(
            "The {label} has an unterminated quoted value."
        )));
    }
    if escaped {
        return Err(DatabaseError::validation(format!(
            "The {label} ends with an incomplete escape."
        )));
    }
    if depth != 0 {
        return Err(DatabaseError::validation(format!(
            "The {label} has an unmatched opening parenthesis."
        )));
    }
    Ok(value.to_string())
}

fn is_numeric_literal(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut index = 0;
    if matches!(bytes[index], b'+' | b'-') {
        index += 1;
        if index == bytes.len() {
            return false;
        }
    }
    let mut digits = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
        digits += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
        index += 1;
        if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

fn ensure_column_supported(
    column: &ValidatedColumnDefinition,
    capabilities: &DatabaseCapabilities,
) -> Result<(), DatabaseError> {
    if column.generated.is_some() {
        capabilities.ensure(CapabilityAction::CreateGeneratedColumn)?;
    }
    match column.identity {
        ColumnIdentity::AutoIncrement => {
            capabilities.ensure(CapabilityAction::CreateAutoIncrementColumn)?;
        }
        ColumnIdentity::Always | ColumnIdentity::ByDefault => {
            capabilities.ensure(CapabilityAction::CreateIdentityColumn)?;
        }
        ColumnIdentity::None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::database::DatabaseCapabilities;
    use crate::storage::DatabaseEngine;

    fn table(database: Option<&str>, schema: Option<&str>, name: &str) -> TableRef {
        TableRef::new(database, schema, name).expect("table")
    }

    fn column(name: &str) -> ColumnDefinitionRequest {
        ColumnDefinitionRequest {
            name: name.to_string(),
            data_type: "bigint".to_string(),
            nullable: false,
            default: None,
            comment: None,
            generated: None,
            identity: None,
        }
    }

    fn capabilities(engine: DatabaseEngine, version: &str) -> DatabaseCapabilities {
        DatabaseCapabilities::for_engine(engine, Some(version))
    }

    #[test]
    fn requests_use_tagged_camel_case_contract() {
        let request: StructureActionRequest = serde_json::from_value(json!({
            "kind": "create_table",
            "database": "app",
            "schema": "public",
            "name": "Users",
            "primaryKey": ["ID"],
            "columns": [{
                "name": "ID",
                "dataType": "bigint",
                "nullable": false,
                "identity": "by_default"
            }, {
                "name": "Email Address",
                "dataType": "varchar(255)",
                "nullable": false,
                "default": {"kind": "string", "value": "n/a"}
            }]
        }))
        .expect("request");
        assert_eq!(request.kind(), StructureActionKind::CreateTable);
        let validated = request.validate().expect("valid request");
        let ValidatedStructureAction::CreateTable(options) = validated else {
            panic!("create table");
        };
        assert_eq!(options.table, table(Some("app"), Some("public"), "Users"));
        assert_eq!(options.columns[0].identity, ColumnIdentity::ByDefault);
        assert_eq!(options.columns[1].name.as_str(), "Email Address");
        assert_eq!(
            options.columns[1].default,
            Some(ColumnDefault::String("n/a".to_string()))
        );
    }

    #[test]
    fn identifiers_must_be_clean_and_renames_must_change_the_name() {
        for name in ["", " users", "users\nname", "users\0name"] {
            let request = StructureActionRequest::DropTable {
                database: None,
                schema: None,
                name: name.to_string(),
            };
            assert!(request.validate().is_err(), "{name:?} must be rejected");
        }

        let rename = StructureActionRequest::RenameTable {
            database: None,
            schema: None,
            name: "Users".to_string(),
            new_name: "Users".to_string(),
        };
        assert!(rename.validate().is_err());
    }

    #[test]
    fn generated_and_identity_options_are_validated_and_gated() {
        let mut generated = column("slug");
        generated.data_type = "text".to_string();
        generated.generated = Some(GeneratedColumnRequest {
            expression: "lower(email)".to_string(),
            stored: true,
        });
        let request = StructureActionRequest::AddColumn {
            database: None,
            schema: None,
            table: "users".to_string(),
            column: generated.clone(),
            position: None,
        };
        let validated = request.validate().expect("generated column");
        validated
            .ensure_supported(&capabilities(DatabaseEngine::Postgresql, "16.2"))
            .expect("PG 16 supports generated columns");
        let error = validated
            .ensure_supported(&capabilities(DatabaseEngine::Postgresql, "11.22"))
            .expect_err("PG 11 has no generated columns");
        assert_eq!(error.kind, crate::database::DatabaseErrorKind::Capability);
        assert!(error.message.contains("create_generated_column"));

        generated.identity = Some(ColumnIdentity::Always);
        let request = StructureActionRequest::AddColumn {
            database: None,
            schema: None,
            table: "users".to_string(),
            column: generated,
            position: None,
        };
        assert!(request.validate().is_err());

        let mut identity = column("id");
        identity.identity = Some(ColumnIdentity::AutoIncrement);
        let mysql = StructureActionRequest::AddColumn {
            database: None,
            schema: None,
            table: "users".to_string(),
            column: identity.clone(),
            position: None,
        }
        .validate()
        .expect("auto increment");
        mysql
            .ensure_supported(&capabilities(DatabaseEngine::Mysql, "5.7.44"))
            .expect("MySQL supports AUTO_INCREMENT");
        assert!(
            mysql
                .ensure_supported(&capabilities(DatabaseEngine::Postgresql, "16.2"))
                .is_err()
        );

        identity.identity = Some(ColumnIdentity::ByDefault);
        let pg = StructureActionRequest::AddColumn {
            database: None,
            schema: None,
            table: "users".to_string(),
            column: identity,
            position: None,
        }
        .validate()
        .expect("identity");
        pg.ensure_supported(&capabilities(DatabaseEngine::Postgresql, "16.2"))
            .expect("PG 16 supports identity");
        assert!(
            pg.ensure_supported(&capabilities(DatabaseEngine::Postgresql, "9.6"))
                .is_err()
        );
    }

    #[test]
    fn column_reorder_and_type_change_use_capability_flags() {
        let reorder = StructureActionRequest::ReorderColumn {
            database: None,
            schema: None,
            table: "users".to_string(),
            column: column("email"),
            position: ColumnPositionRequest::First,
        }
        .validate()
        .expect("reorder");
        reorder
            .ensure_supported(&capabilities(DatabaseEngine::Mysql, "8.0.36"))
            .expect("MySQL reorders columns");
        assert!(
            reorder
                .ensure_supported(&capabilities(DatabaseEngine::Postgresql, "16.2"))
                .is_err()
        );

        let alter = StructureActionRequest::AlterColumn {
            database: None,
            schema: None,
            table: "users".to_string(),
            name: "email".to_string(),
            data_type: Some("varchar(320)".to_string()),
            nullable: None,
            default: None,
            drop_default: false,
            comment: None,
            drop_comment: false,
        }
        .validate()
        .expect("alter");
        alter
            .ensure_supported(&capabilities(DatabaseEngine::Mysql, "8.0.36"))
            .expect("type change");
        assert!(alter.destructive());
        assert_eq!(alter.object().name.as_deref(), Some("email"));
    }

    #[test]
    fn check_and_partial_index_capabilities_are_enforced() {
        let check = StructureActionRequest::AddConstraint {
            database: None,
            schema: None,
            table: "users".to_string(),
            constraint: ConstraintRequest::Check {
                name: "users_email_check".to_string(),
                expression: "email <> ''".to_string(),
            },
        }
        .validate()
        .expect("check");
        check
            .ensure_supported(&capabilities(DatabaseEngine::Mysql, "8.0.16"))
            .expect("MySQL 8.0.16 checks");
        assert!(
            check
                .ensure_supported(&capabilities(DatabaseEngine::Mysql, "5.7.44"))
                .is_err()
        );

        let partial = StructureActionRequest::CreateIndex {
            database: None,
            schema: None,
            table: "users".to_string(),
            name: "users_active".to_string(),
            columns: vec!["email".to_string()],
            unique: false,
            method: None,
            predicate: Some("active = true".to_string()),
        }
        .validate()
        .expect("partial index");
        partial
            .ensure_supported(&capabilities(DatabaseEngine::Postgresql, "16.2"))
            .expect("PG partial index");
        assert!(
            partial
                .ensure_supported(&capabilities(DatabaseEngine::Mysql, "8.0.36"))
                .is_err()
        );
    }

    #[test]
    fn expressions_reject_statement_breakouts_and_unbalanced_input() {
        for expression in [
            "1; DROP TABLE users",
            "1 -- comment",
            "1 /* comment */",
            "lower(email",
            "lower(email))",
            "a, b",
            "concat('unterminated)",
            "1) , DROP COLUMN email, ADD COLUMN x int --",
        ] {
            let request = StructureActionRequest::AddConstraint {
                database: None,
                schema: None,
                table: "users".to_string(),
                constraint: ConstraintRequest::Check {
                    name: "check_1".to_string(),
                    expression: expression.to_string(),
                },
            };
            assert!(
                request.validate().is_err(),
                "{expression:?} must be rejected"
            );
        }

        for expression in [
            "lower(email)",
            "coalesce(first_name, '') <> ''",
            "price * quantity > 0",
            "'a,b' <> ''",
        ] {
            let request = StructureActionRequest::AddConstraint {
                database: None,
                schema: None,
                table: "users".to_string(),
                constraint: ConstraintRequest::Check {
                    name: "check_1".to_string(),
                    expression: expression.to_string(),
                },
            };
            assert!(
                request.validate().is_ok(),
                "{expression:?} must be accepted"
            );
        }
    }

    #[test]
    fn defaults_reject_executable_expressions_and_bad_numbers() {
        for default in [
            ColumnDefaultRequest::Number {
                value: "1; DROP TABLE users".to_string(),
            },
            ColumnDefaultRequest::Number {
                value: "1.2.3".to_string(),
            },
            ColumnDefaultRequest::Number {
                value: "0x10".to_string(),
            },
        ] {
            assert!(
                validate_default(&default).is_err(),
                "{default:?} must be rejected"
            );
        }
        assert_eq!(
            validate_default(&ColumnDefaultRequest::Number {
                value: "-12.50e3".to_string()
            })
            .expect("number"),
            ColumnDefault::Number("-12.50e3".to_string())
        );
    }

    #[test]
    fn data_types_reject_statement_and_column_breakouts() {
        for data_type in [
            "bigint, ADD COLUMN evil int",
            "bigint); DROP TABLE users; --",
            "varchar(20, ADD COLUMN evil int)",
            "varchar(20))",
            "varchar((20))",
            "varchar(20) /* comment */",
            "enum('a','b')",
            "bigint + 1",
            "varchar(20)..int",
        ] {
            assert!(
                validate_data_type("column type", data_type).is_err(),
                "{data_type:?} must be rejected"
            );
        }

        for data_type in [
            "varchar(255)",
            "numeric(10, 2)",
            "timestamp(3) with time zone",
            "bigint unsigned",
            "bigint[]",
            "public.my_type",
        ] {
            assert!(
                validate_data_type("column type", data_type).is_ok(),
                "{data_type:?} must be accepted"
            );
        }
    }

    #[test]
    fn duplicate_and_invalid_column_lists_are_rejected() {
        assert!(
            StructureActionRequest::CreateTable {
                database: None,
                schema: None,
                name: "users".to_string(),
                columns: vec![column("Email"), column("email")],
                primary_key: Vec::new(),
                unique_keys: Vec::new(),
                options: None,
            }
            .validate()
            .is_err()
        );
        assert!(
            StructureActionRequest::CreateTable {
                database: None,
                schema: None,
                name: "users".to_string(),
                columns: vec![column("id")],
                primary_key: vec!["missing".to_string()],
                unique_keys: Vec::new(),
                options: None,
            }
            .validate()
            .is_err()
        );
        assert!(
            StructureActionRequest::CreateIndex {
                database: None,
                schema: None,
                table: "users".to_string(),
                name: "idx".to_string(),
                columns: Vec::new(),
                unique: false,
                method: None,
                predicate: None,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn expression_depth_is_balanced_across_quoted_values() {
        assert!(validate_expression("check", "concat('(', email) <> ''").is_ok());
        assert!(validate_expression("check", "concat(')', email) <> ''").is_ok());
    }

    #[test]
    fn create_table_validates_unique_keys_and_engine_options() {
        let mut email = column("email");
        email.data_type = "varchar(320)".to_string();
        let request = StructureActionRequest::CreateTable {
            database: Some("app".to_string()),
            schema: None,
            name: "users".to_string(),
            columns: vec![column("id"), email],
            primary_key: vec!["id".to_string()],
            unique_keys: vec![UniqueKeyRequest {
                name: "users_email_key".to_string(),
                columns: vec!["email".to_string()],
            }],
            options: Some(TableOptionsRequest {
                engine: Some("InnoDB".to_string()),
                charset: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_unicode_ci".to_string()),
                tablespace: None,
                unlogged: false,
                comment: Some("Users table".to_string()),
            }),
        };
        let validated = request.validate().expect("valid table");
        let ValidatedStructureAction::CreateTable(options) = validated else {
            panic!("create table");
        };
        assert_eq!(options.unique_keys.len(), 1);
        assert_eq!(options.options.engine.as_deref(), Some("InnoDB"));
        assert_eq!(options.options.charset.as_deref(), Some("utf8mb4"));

        let missing_column = StructureActionRequest::CreateTable {
            database: None,
            schema: None,
            name: "users".to_string(),
            columns: vec![column("id")],
            primary_key: Vec::new(),
            unique_keys: vec![UniqueKeyRequest {
                name: "users_email_key".to_string(),
                columns: vec!["email".to_string()],
            }],
            options: None,
        };
        assert!(missing_column.validate().is_err());

        let duplicate_key_names = StructureActionRequest::CreateTable {
            database: None,
            schema: None,
            name: "users".to_string(),
            columns: vec![column("id")],
            primary_key: Vec::new(),
            unique_keys: vec![
                UniqueKeyRequest {
                    name: "same".to_string(),
                    columns: vec!["id".to_string()],
                },
                UniqueKeyRequest {
                    name: "same".to_string(),
                    columns: vec!["id".to_string()],
                },
            ],
            options: None,
        };
        assert!(duplicate_key_names.validate().is_err());

        let bad_option = StructureActionRequest::CreateTable {
            database: None,
            schema: None,
            name: "users".to_string(),
            columns: vec![column("id")],
            primary_key: Vec::new(),
            unique_keys: Vec::new(),
            options: Some(TableOptionsRequest {
                engine: Some("InnoDB; DROP TABLE users".to_string()),
                ..TableOptionsRequest::default()
            }),
        };
        assert!(bad_option.validate().is_err());
    }

    #[test]
    fn duplicate_table_requires_a_new_name_and_uses_the_duplicate_capability() {
        let request = StructureActionRequest::DuplicateTable {
            database: Some("app".to_string()),
            schema: None,
            name: "users".to_string(),
            new_name: "users_copy".to_string(),
        };
        let validated = request.validate().expect("valid duplicate");
        assert_eq!(validated.kind(), StructureActionKind::DuplicateTable);
        assert_eq!(validated.target_table_name(), Some("users_copy"));
        assert_eq!(
            validated.capability(),
            Some(CapabilityAction::DuplicateTable)
        );
        validated
            .ensure_supported(&capabilities(DatabaseEngine::Mysql, "5.7.44"))
            .expect("duplicate is baseline");
        validated
            .ensure_supported(&capabilities(DatabaseEngine::Postgresql, "9.6.24"))
            .expect("duplicate is baseline");

        let same_name = StructureActionRequest::DuplicateTable {
            database: Some("app".to_string()),
            schema: None,
            name: "users".to_string(),
            new_name: "users".to_string(),
        };
        assert!(same_name.validate().is_err());
    }
}
