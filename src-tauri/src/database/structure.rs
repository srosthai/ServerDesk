//! Typed table, column, index, and constraint changes with DDL previews.
//!
//! Requests cross the Tauri boundary as untrusted JSON. [`StructureActionRequest::validate`]
//! turns them into [`ValidatedStructureAction`] values: every identifier has been
//! checked by the shared [`Identifier`] rules, every type name and expression is
//! bounded, and every user value is represented by a typed value instead of raw
//! SQL. Engine modules generate the DDL from the validated action.
//!
//! This module deliberately does not execute anything. DDL generation is a pure
//! planning step; a future execution command must ask an adapter for the same
//! preview first so capability checks and read-only checks cannot be bypassed.

use serde::{Deserialize, Serialize};

use crate::{core::AppError, storage::DatabaseEngine};

use super::{
    capabilities::{DatabaseAction as CapabilityAction, DatabaseCapabilities},
    dialect::Identifier,
    errors::DatabaseError,
    service::DatabaseConnectionRegistry,
    traits::{ColumnIdentity, ConstraintKind, DatabaseAdapter, TableRef},
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

    /// Validates the complete request before an engine planner sees it.
    pub fn validate(&self) -> Result<ValidatedStructureAction, DatabaseError> {
        match self {
            Self::CreateTable {
                database,
                schema,
                name,
                columns,
                primary_key,
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
                Ok(ValidatedStructureAction::CreateTable(
                    ValidatedCreateTable {
                        table,
                        columns,
                        primary_key,
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

/// Stateless preview service used by the structure command.
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
}

/// Validates a request and asks the adapter for a preview. There is no execute
/// counterpart in this module yet; when one is added it must call this function
/// or [`DatabaseAdapter::preview_structure_action`] first.
pub async fn preview_structure_with_adapter(
    adapter: &dyn DatabaseAdapter,
    request: &StructureActionRequest,
) -> Result<StructureDdlPreview, DatabaseError> {
    let validated = request.validate()?;
    adapter.preview_structure_action(&validated).await
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
}
