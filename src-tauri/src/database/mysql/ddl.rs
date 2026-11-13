//! MySQL table, column, index, and constraint DDL planning.
//!
//! A planner receives only [`ValidatedStructureAction`] values. Every name is
//! quoted with [`MYSQL_DIALECT`], every value is rendered as a typed literal,
//! and engine/version restrictions are rejected before a statement is returned.

use crate::database::{
    dialect::{Identifier, IdentifierQuoter, QualifiedName, SqlDialect},
    errors::DatabaseError,
    structure::{
        ColumnDefault, ColumnPosition, ReferentialAction, StructureDdlPreview,
        ValidatedColumnDefinition, ValidatedConstraint, ValidatedStructureAction,
    },
    traits::{ColumnIdentity, ConstraintKind, TableRef},
};

use super::MYSQL_DIALECT;

/// Builds a preview for one validated MySQL structure action.
pub fn preview(action: &ValidatedStructureAction) -> Result<StructureDdlPreview, DatabaseError> {
    let mut preview = match action {
        ValidatedStructureAction::CreateTable(options) => {
            let table = table_name(&options.table)?;
            let mut definitions = options
                .columns
                .iter()
                .map(render_column)
                .collect::<Result<Vec<_>, _>>()?;
            if !options.primary_key.is_empty() {
                definitions.push(format!(
                    "PRIMARY KEY ({})",
                    quote_column_list(&options.primary_key)
                ));
            }
            for key in &options.unique_keys {
                definitions.push(format!(
                    "UNIQUE KEY {} ({})",
                    MYSQL_DIALECT.quote_identifier(&key.name),
                    quote_column_list(&key.columns)
                ));
            }
            let options_sql = render_table_options(&options.options)?;
            action.preview(
                MYSQL_DIALECT.engine(),
                vec![format!(
                    "CREATE TABLE {} (\n  {}\n){}",
                    MYSQL_DIALECT.quote_qualified_name(&table),
                    definitions.join(",\n  "),
                    options_sql
                )],
                vec![
                    "MySQL creates the table with the database default character set and \
                     collation unless the preview sets them explicitly."
                        .to_string(),
                ],
            )
        }
        ValidatedStructureAction::RenameTable { table, new_name } => {
            let old_name = table_name(table)?;
            let new_table = table_name_with(table, new_name)?;
            action.preview(
                MYSQL_DIALECT.engine(),
                vec![format!(
                    "RENAME TABLE {} TO {}",
                    MYSQL_DIALECT.quote_qualified_name(&old_name),
                    MYSQL_DIALECT.quote_qualified_name(&new_table)
                )],
                vec![
                    "Saved queries, grants, and application code that use the old table name must \
                     be updated."
                        .to_string(),
                ],
            )
        }
        ValidatedStructureAction::DuplicateTable { table, new_name } => {
            let old_name = table_name(table)?;
            let new_table = table_name_with(table, new_name)?;
            action.preview(
                MYSQL_DIALECT.engine(),
                vec![format!(
                    "CREATE TABLE {} LIKE {}",
                    MYSQL_DIALECT.quote_qualified_name(&new_table),
                    MYSQL_DIALECT.quote_qualified_name(&old_name)
                )],
                vec![
                    "The duplicate copies columns, indexes, and the table options. Rows are not \
                     copied."
                        .to_string(),
                    "Foreign keys are not copied by CREATE TABLE ... LIKE; add them explicitly if \
                     the duplicate needs the same relationships."
                        .to_string(),
                ],
            )
        }
        ValidatedStructureAction::DropTable { table } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "DROP TABLE {}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?)
            )],
            vec![
                "Data loss: dropping a table permanently removes every row and its schema \
                 definition."
                    .to_string(),
                "Incoming foreign keys, views, and triggers can block the drop or become invalid; \
                 inspect table dependencies first."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::TruncateTable { table } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "TRUNCATE TABLE {}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?)
            )],
            vec![
                "Data loss: TRUNCATE TABLE permanently removes every row and resets MySQL \
                 auto-increment counters."
                    .to_string(),
                "MySQL commits TRUNCATE TABLE immediately; it cannot be rolled back.".to_string(),
            ],
        ),
        ValidatedStructureAction::AddColumn {
            table,
            column,
            position,
        } => {
            let mut statement = format!(
                "ALTER TABLE {} ADD COLUMN {}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?),
                render_column(column)?
            );
            if let Some(position) = position {
                statement.push_str(&render_position(position));
            }
            let mut warnings = Vec::new();
            if !column.nullable && column.default.is_none() && column.generated.is_none() {
                warnings.push(
                    "Adding a NOT NULL column without a default can fail or backfill existing \
                     rows with an implicit value."
                        .to_string(),
                );
            }
            if column.generated.is_some() {
                warnings.push(
                    "Adding a generated column may rebuild the table and can take a long time."
                        .to_string(),
                );
            }
            if column.identity == ColumnIdentity::AutoIncrement {
                warnings.push(
                    "MySQL requires an AUTO_INCREMENT column to be indexed; add the primary key \
                     or a unique index in the same change."
                        .to_string(),
                );
            }
            action.preview(MYSQL_DIALECT.engine(), vec![statement], warnings)
        }
        ValidatedStructureAction::AlterColumn {
            table,
            name,
            changes,
        } => {
            let Some(data_type) = changes.data_type.as_deref() else {
                return Err(DatabaseError::capability(
                    "MySQL requires the full column type when a column is changed.",
                )
                .with_details(
                    "ALTER TABLE ... MODIFY COLUMN replaces the complete column definition, so a \
                     preview cannot change only nullability, default, or comment.",
                ));
            };
            let Some(nullable) = changes.nullable else {
                return Err(DatabaseError::capability(
                    "MySQL requires explicit nullability when a column is changed.",
                )
                .with_details(
                    "ALTER TABLE ... MODIFY COLUMN replaces the complete column definition, so a \
                     preview cannot change only the type, default, or comment.",
                ));
            };
            if changes.default.is_some() && changes.drop_default {
                return Err(DatabaseError::validation(
                    "A column change cannot set and drop a default at the same time.",
                ));
            }
            let column = ValidatedColumnDefinition {
                name: name.clone(),
                data_type: data_type.to_string(),
                nullable,
                default: changes.default.clone(),
                comment: changes.comment.clone(),
                generated: None,
                identity: ColumnIdentity::None,
            };
            let mut warnings = vec![
                "MySQL MODIFY COLUMN replaces the entire column definition; attributes not \
                 supplied in this preview revert to their type defaults."
                    .to_string(),
                "Data loss risk: changing a column type can truncate values or fail when existing \
                 rows cannot be converted safely."
                    .to_string(),
            ];
            if changes.comment.is_none() && !changes.drop_comment {
                warnings.push(
                    "Any existing column comment is removed because MODIFY COLUMN replaces the \
                     whole definition."
                        .to_string(),
                );
            }
            if !nullable && changes.default.is_none() {
                warnings.push(
                    "Changing a column to NOT NULL without a default can fail when existing rows \
                     contain NULL values."
                        .to_string(),
                );
            }
            action.preview(
                MYSQL_DIALECT.engine(),
                vec![format!(
                    "ALTER TABLE {} MODIFY COLUMN {}",
                    MYSQL_DIALECT.quote_qualified_name(&table_name(table)?),
                    render_column(&column)?
                )],
                warnings,
            )
        }
        ValidatedStructureAction::RenameColumn {
            table,
            name,
            new_name,
        } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?),
                MYSQL_DIALECT.quote_identifier(name),
                MYSQL_DIALECT.quote_identifier(new_name)
            )],
            vec![
                "MySQL updates indexes and foreign keys that reference the column.".to_string(),
                "Saved queries and application code must use the new column name.".to_string(),
            ],
        ),
        ValidatedStructureAction::DropColumn { table, name } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} DROP COLUMN {}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?),
                MYSQL_DIALECT.quote_identifier(name)
            )],
            vec![
                "Data loss: dropping a column permanently removes every value stored in it."
                    .to_string(),
                "Indexes, foreign keys, generated columns, and views that reference the column can \
                 block the drop or become invalid."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::ReorderColumn {
            table,
            column,
            position,
        } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} MODIFY COLUMN {}{}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?),
                render_column(column)?,
                render_position(position)
            )],
            vec![
                "MySQL has no move-only syntax; this preview replaces the whole column definition \
                 while changing its position."
                    .to_string(),
                "Reordering columns may rebuild the table and can take a long time on large \
                 tables."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::CreateIndex(options) => {
            let table = table_name(&options.table)?;
            let mut statement = String::from("CREATE ");
            if options.unique {
                statement.push_str("UNIQUE ");
            }
            statement.push_str("INDEX ");
            statement.push_str(&MYSQL_DIALECT.quote_identifier(&options.name));
            if let Some(method) = options.method.as_deref() {
                let method = validate_method(method)?;
                statement.push_str(" USING ");
                statement.push_str(method);
            }
            statement.push_str(" ON ");
            statement.push_str(&MYSQL_DIALECT.quote_qualified_name(&table));
            statement.push_str(" (");
            statement.push_str(&quote_column_list(&options.columns));
            statement.push(')');

            if options.predicate.is_some() {
                return Err(DatabaseError::capability(
                    "MySQL does not support partial indexes with a WHERE predicate.",
                ));
            }

            let mut warnings = vec![
                "Creating an index can take a long time and temporarily reduce write throughput."
                    .to_string(),
            ];
            if options.unique {
                warnings.push(
                    "The unique index creation fails if existing rows contain duplicate values."
                        .to_string(),
                );
            }
            action.preview(MYSQL_DIALECT.engine(), vec![statement], warnings)
        }
        ValidatedStructureAction::RenameIndex {
            table,
            name,
            new_name,
        } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} RENAME INDEX {} TO {}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?),
                MYSQL_DIALECT.quote_identifier(name),
                MYSQL_DIALECT.quote_identifier(new_name)
            )],
            vec![
                "Saved queries and application hints that name the old index must be updated."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::DropIndex { table, name } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} DROP INDEX {}",
                MYSQL_DIALECT.quote_qualified_name(&table_name(table)?),
                MYSQL_DIALECT.quote_identifier(name)
            )],
            vec![
                "Dropping an index does not delete table data but can make queries slower."
                    .to_string(),
                "A foreign key that depends on the index can block the drop.".to_string(),
            ],
        ),
        ValidatedStructureAction::AddConstraint { table, constraint } => {
            let table_name = table_name(table)?;
            let statement = render_add_constraint(&table_name, constraint)?;
            let mut warnings = vec![
                "Adding a constraint validates existing rows and can temporarily lock writes."
                    .to_string(),
            ];
            if matches!(constraint, ValidatedConstraint::PrimaryKey { .. }) {
                warnings.push(
                    "MySQL stores a primary key as PRIMARY; the requested constraint name is not \
                     preserved."
                        .to_string(),
                );
            }
            if let ValidatedConstraint::Check { .. } = constraint {
                warnings.push(
                    "MySQL 8.0.16 and later enforce CHECK constraints for existing and new rows."
                        .to_string(),
                );
            }
            action.preview(MYSQL_DIALECT.engine(), vec![statement], warnings)
        }
        ValidatedStructureAction::DropConstraint { table, name, kind } => {
            let table_name = MYSQL_DIALECT.quote_qualified_name(&table_name(table)?);
            let statement = match kind {
                ConstraintKind::PrimaryKey => {
                    format!("ALTER TABLE {table_name} DROP PRIMARY KEY")
                }
                ConstraintKind::Unique => format!(
                    "ALTER TABLE {table_name} DROP INDEX {}",
                    MYSQL_DIALECT.quote_identifier(name)
                ),
                ConstraintKind::ForeignKey => format!(
                    "ALTER TABLE {table_name} DROP FOREIGN KEY {}",
                    MYSQL_DIALECT.quote_identifier(name)
                ),
                ConstraintKind::Check => {
                    format!(
                        "ALTER TABLE {table_name} DROP CHECK {}",
                        MYSQL_DIALECT.quote_identifier(name)
                    )
                }
                ConstraintKind::Other => {
                    return Err(DatabaseError::capability(
                        "MySQL cannot drop an unclassified constraint safely. Drop the underlying \
                         index or foreign key explicitly instead.",
                    ));
                }
            };
            action.preview(
                MYSQL_DIALECT.engine(),
                vec![statement],
                vec![
                    "Dropping a constraint does not delete rows, but existing data may violate the \
                     rule afterwards."
                        .to_string(),
                ],
            )
        }
    };

    if preview.object.database.is_none() && preview.object.schema.is_none() {
        preview.warnings.push(
            "The table name is unqualified and resolves against the connection's active database."
                .to_string(),
        );
    }
    Ok(preview)
}

fn table_name(table: &TableRef) -> Result<QualifiedName, DatabaseError> {
    match (&table.database, &table.schema) {
        (Some(database), Some(schema)) if database.as_str() != schema.as_str() => {
            Err(DatabaseError::validation(
                "MySQL schemas and databases share one namespace; the database and schema must \
                 match.",
            ))
        }
        (Some(database), _) => QualifiedName::new([database.clone(), table.name.clone()]),
        (None, Some(schema)) => QualifiedName::new([schema.clone(), table.name.clone()]),
        (None, None) => QualifiedName::new([table.name.clone()]),
    }
}

fn table_name_with(
    table: &TableRef,
    new_name: &Identifier,
) -> Result<QualifiedName, DatabaseError> {
    match (&table.database, &table.schema) {
        (Some(database), Some(schema)) if database.as_str() != schema.as_str() => {
            Err(DatabaseError::validation(
                "MySQL schemas and databases share one namespace; the database and schema must \
                 match.",
            ))
        }
        (Some(database), _) => QualifiedName::new([database.clone(), new_name.clone()]),
        (None, Some(schema)) => QualifiedName::new([schema.clone(), new_name.clone()]),
        (None, None) => QualifiedName::new([new_name.clone()]),
    }
}

fn quote_column_list(columns: &[Identifier]) -> String {
    columns
        .iter()
        .map(|column| MYSQL_DIALECT.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_position(position: &ColumnPosition) -> String {
    match position {
        ColumnPosition::First => " FIRST".to_string(),
        ColumnPosition::After { column } => {
            format!(" AFTER {}", MYSQL_DIALECT.quote_identifier(column))
        }
    }
}

fn render_table_options(
    options: &crate::database::structure::ValidatedTableOptions,
) -> Result<String, DatabaseError> {
    if options.tablespace.is_some() {
        return Err(DatabaseError::capability(
            "MySQL does not support a PostgreSQL-style table tablespace option. Use CREATE TABLE \
             ... TABLESPACE with an InnoDB general tablespace separately.",
        ));
    }
    if options.unlogged {
        return Err(DatabaseError::capability(
            "MySQL does not support PostgreSQL UNLOGGED tables.",
        ));
    }

    let mut suffix = String::new();
    if let Some(engine) = options.engine.as_deref() {
        suffix.push_str(" ENGINE=");
        suffix.push_str(engine);
    }
    if let Some(charset) = options.charset.as_deref() {
        suffix.push_str(" DEFAULT CHARACTER SET ");
        suffix.push_str(charset);
    }
    if let Some(collation) = options.collation.as_deref() {
        suffix.push_str(" COLLATE ");
        suffix.push_str(collation);
    }
    if let Some(comment) = options.comment.as_deref() {
        suffix.push_str(" COMMENT=");
        suffix.push_str(&string_literal(comment));
    }
    Ok(suffix)
}

fn render_column(column: &ValidatedColumnDefinition) -> Result<String, DatabaseError> {
    if column.generated.is_some() && column.identity != ColumnIdentity::None {
        return Err(DatabaseError::validation(
            "A generated column cannot also have an identity or auto-increment option.",
        ));
    }
    let mut definition = format!(
        "{} {}",
        MYSQL_DIALECT.quote_identifier(&column.name),
        column.data_type
    );

    match (&column.generated, column.identity) {
        (Some(generated), _) => {
            definition.push_str(&format!(
                " GENERATED ALWAYS AS ({}) {}",
                generated.expression,
                if generated.stored {
                    "STORED"
                } else {
                    "VIRTUAL"
                }
            ));
            definition.push_str(if column.nullable {
                " NULL"
            } else {
                " NOT NULL"
            });
        }
        (None, identity) => {
            definition.push_str(if column.nullable {
                " NULL"
            } else {
                " NOT NULL"
            });
            if let Some(default) = column.default.as_ref() {
                definition.push_str(" DEFAULT ");
                definition.push_str(&render_default(default));
            }
            match identity {
                ColumnIdentity::AutoIncrement => definition.push_str(" AUTO_INCREMENT"),
                ColumnIdentity::Always | ColumnIdentity::ByDefault => {
                    return Err(DatabaseError::capability(
                        "MySQL does not support SQL-standard GENERATED ... AS IDENTITY columns. Use \
                         an AUTO_INCREMENT column instead.",
                    ));
                }
                ColumnIdentity::None => {}
            }
        }
    }

    if let Some(comment) = column.comment.as_deref() {
        definition.push_str(" COMMENT ");
        definition.push_str(&string_literal(comment));
    }

    Ok(definition)
}

fn render_default(default: &ColumnDefault) -> String {
    match default {
        ColumnDefault::Null => "NULL".to_string(),
        ColumnDefault::String(value) => string_literal(value),
        ColumnDefault::Number(value) => value.clone(),
        ColumnDefault::Boolean(value) => {
            if *value {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        ColumnDefault::CurrentTimestamp => "CURRENT_TIMESTAMP".to_string(),
    }
}

fn render_add_constraint(
    table: &QualifiedName,
    constraint: &ValidatedConstraint,
) -> Result<String, DatabaseError> {
    let table = MYSQL_DIALECT.quote_qualified_name(table);
    match constraint {
        ValidatedConstraint::PrimaryKey { name, columns } => Ok(format!(
            "ALTER TABLE {table} ADD CONSTRAINT {} PRIMARY KEY ({})",
            MYSQL_DIALECT.quote_identifier(name),
            quote_column_list(columns)
        )),
        ValidatedConstraint::Unique { name, columns } => Ok(format!(
            "ALTER TABLE {table} ADD CONSTRAINT {} UNIQUE ({})",
            MYSQL_DIALECT.quote_identifier(name),
            quote_column_list(columns)
        )),
        ValidatedConstraint::ForeignKey {
            name,
            columns,
            referenced_schema,
            referenced_table,
            referenced_columns,
            on_delete,
            on_update,
        } => {
            let referenced = match referenced_schema {
                Some(schema) => QualifiedName::new([schema.clone(), referenced_table.clone()])?,
                None => QualifiedName::new([referenced_table.clone()])?,
            };
            let mut statement = format!(
                "ALTER TABLE {table} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
                MYSQL_DIALECT.quote_identifier(name),
                quote_column_list(columns),
                MYSQL_DIALECT.quote_qualified_name(&referenced),
                quote_column_list(referenced_columns)
            );
            if let Some(on_delete) = on_delete {
                if *on_delete == ReferentialAction::SetDefault {
                    return Err(DatabaseError::capability(
                        "MySQL/InnoDB does not support ON DELETE SET DEFAULT for foreign keys.",
                    ));
                }
                statement.push_str(" ON DELETE ");
                statement.push_str(on_delete.as_sql());
            }
            if let Some(on_update) = on_update {
                if *on_update == ReferentialAction::SetDefault {
                    return Err(DatabaseError::capability(
                        "MySQL/InnoDB does not support ON UPDATE SET DEFAULT for foreign keys.",
                    ));
                }
                statement.push_str(" ON UPDATE ");
                statement.push_str(on_update.as_sql());
            }
            Ok(statement)
        }
        ValidatedConstraint::Check { name, expression } => Ok(format!(
            "ALTER TABLE {table} ADD CONSTRAINT {} CHECK ({})",
            MYSQL_DIALECT.quote_identifier(name),
            expression
        )),
    }
}

fn validate_method(method: &str) -> Result<&'static str, DatabaseError> {
    match method {
        "btree" => Ok("BTREE"),
        "hash" => Ok("HASH"),
        "fulltext" => Ok("FULLTEXT"),
        "spatial" => Ok("SPATIAL"),
        other => Err(DatabaseError::capability(format!(
            "MySQL does not support the {other} index method."
        ))
        .with_details("Use BTREE, HASH, FULLTEXT, or SPATIAL.")),
    }
}

/// MySQL string literal escaping. Backslashes are doubled because MySQL treats
/// them as escapes by default; single quotes are doubled for both escape modes.
fn string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use crate::database::{
        DatabaseErrorKind, TableRef,
        capabilities::DatabaseCapabilities,
        structure::{
            ColumnDefinitionRequest, ConstraintRequest, StructureActionRequest,
            TableOptionsRequest, UniqueKeyRequest,
        },
    };
    use crate::storage::DatabaseEngine;

    use super::*;

    fn validate(request: StructureActionRequest) -> ValidatedStructureAction {
        request.validate().expect("valid structure request")
    }

    fn capabilities() -> DatabaseCapabilities {
        DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some("8.0.36"))
    }

    fn column(name: &str, data_type: &str) -> ColumnDefinitionRequest {
        ColumnDefinitionRequest {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable: false,
            default: None,
            comment: None,
            generated: None,
            identity: None,
        }
    }

    #[test]
    fn create_table_quotes_every_identifier_and_renders_engine_column_options() {
        let mut id = column("ID", "bigint unsigned");
        id.identity = Some(ColumnIdentity::AutoIncrement);
        let mut slug = column("Slug Value", "varchar(180)");
        slug.generated = Some(crate::database::GeneratedColumnRequest {
            expression: "lower(`Email`)".to_string(),
            stored: true,
        });
        slug.nullable = true;
        let mut email = column("Email", "varchar(255)");
        email.default = Some(crate::database::ColumnDefaultRequest::String {
            value: "n/a".to_string(),
        });
        email.comment = Some("Login address".to_string());

        let action = validate(StructureActionRequest::CreateTable {
            database: Some("app db".to_string()),
            schema: None,
            name: "Users".to_string(),
            columns: vec![id, slug, email],
            primary_key: vec!["ID".to_string()],
            unique_keys: Vec::new(),
            options: None,
        });
        action.ensure_supported(&capabilities()).expect("supported");
        let preview = preview(&action).expect("preview");

        assert_eq!(
            preview.statements,
            [concat!(
                "CREATE TABLE `app db`.`Users` (\n",
                "  `ID` bigint unsigned NOT NULL AUTO_INCREMENT,\n",
                "  `Slug Value` varchar(180) GENERATED ALWAYS AS (lower(`Email`)) STORED NULL,\n",
                "  `Email` varchar(255) NOT NULL DEFAULT 'n/a' COMMENT 'Login address',\n",
                "  PRIMARY KEY (`ID`)\n",
                ")"
            )]
        );
        assert_eq!(preview.object.table, "Users");
        assert_eq!(preview.object.name, None);
        assert!(!preview.destructive);
    }

    #[test]
    fn column_changes_quote_mixed_case_and_escape_default_values() {
        let mut added = column("Display Name", "varchar(120)");
        added.default = Some(crate::database::ColumnDefaultRequest::String {
            value: "Robert'; DROP TABLE users; --".to_string(),
        });
        let add = validate(StructureActionRequest::AddColumn {
            database: Some("app".to_string()),
            schema: None,
            table: "Users".to_string(),
            column: added,
            position: Some(crate::database::ColumnPositionRequest::After {
                column: "Email".to_string(),
            }),
        });
        add.ensure_supported(&capabilities()).expect("supported");
        let add_preview = preview(&add).expect("add preview");
        assert_eq!(
            add_preview.statements,
            [concat!(
                "ALTER TABLE `app`.`Users` ADD COLUMN `Display Name` varchar(120) NOT NULL ",
                "DEFAULT 'Robert''; DROP TABLE users; --' AFTER `Email`"
            )]
        );
        assert!(!add_preview.ddl.contains("DROP TABLE users`"));

        let alter = validate(StructureActionRequest::AlterColumn {
            database: Some("app".to_string()),
            schema: None,
            table: "Users".to_string(),
            name: "Email".to_string(),
            data_type: Some("varchar(320)".to_string()),
            nullable: Some(false),
            default: Some(crate::database::ColumnDefaultRequest::Null),
            drop_default: false,
            comment: Some("Updated address".to_string()),
            drop_comment: false,
        });
        alter.ensure_supported(&capabilities()).expect("supported");
        let preview = preview(&alter).expect("alter preview");
        assert_eq!(
            preview.statements,
            [concat!(
                "ALTER TABLE `app`.`Users` MODIFY COLUMN `Email` varchar(320) NOT NULL ",
                "DEFAULT NULL COMMENT 'Updated address'"
            )]
        );
        assert!(preview.destructive);
        assert!(preview.requires_confirmation);
    }

    #[test]
    fn rename_and_drop_statement_forms_are_engine_specific() {
        let rename = validate(StructureActionRequest::RenameTable {
            database: Some("app".to_string()),
            schema: None,
            name: "Users".to_string(),
            new_name: "Accounts".to_string(),
        });
        assert_eq!(
            preview(&rename).expect("rename").statements,
            ["RENAME TABLE `app`.`Users` TO `app`.`Accounts`"]
        );

        let drop = validate(StructureActionRequest::DropTable {
            database: Some("app".to_string()),
            schema: None,
            name: "Users".to_string(),
        });
        let drop_preview = preview(&drop).expect("drop");
        assert_eq!(drop_preview.statements, ["DROP TABLE `app`.`Users`"]);
        assert!(drop_preview.destructive);
        assert!(drop_preview.requires_confirmation);

        let truncate = validate(StructureActionRequest::TruncateTable {
            database: Some("app".to_string()),
            schema: None,
            name: "Users".to_string(),
        });
        assert_eq!(
            preview(&truncate).expect("truncate").statements,
            ["TRUNCATE TABLE `app`.`Users`"]
        );
    }

    #[test]
    fn index_and_constraint_ddl_is_quoted_and_capability_checked() {
        let index = validate(StructureActionRequest::CreateIndex {
            database: Some("app".to_string()),
            schema: None,
            table: "Users".to_string(),
            name: "Users Email Index".to_string(),
            columns: vec!["Email".to_string()],
            unique: true,
            method: Some("btree".to_string()),
            predicate: None,
        });
        index.ensure_supported(&capabilities()).expect("supported");
        assert_eq!(
            preview(&index).expect("index").statements,
            ["CREATE UNIQUE INDEX `Users Email Index` USING BTREE ON `app`.`Users` (`Email`)"]
        );

        let constraint = validate(StructureActionRequest::AddConstraint {
            database: Some("app".to_string()),
            schema: None,
            table: "Orders".to_string(),
            constraint: ConstraintRequest::ForeignKey {
                name: "orders_user_fk".to_string(),
                columns: vec!["User ID".to_string()],
                referenced_schema: Some("app".to_string()),
                referenced_table: "Users".to_string(),
                referenced_columns: vec!["ID".to_string()],
                on_delete: Some(crate::database::ReferentialAction::Cascade),
                on_update: None,
            },
        });
        constraint
            .ensure_supported(&capabilities())
            .expect("supported");
        assert_eq!(
            preview(&constraint).expect("constraint").statements,
            [concat!(
                "ALTER TABLE `app`.`Orders` ADD CONSTRAINT `orders_user_fk` FOREIGN KEY ",
                "(`User ID`) REFERENCES `app`.`Users` (`ID`) ON DELETE CASCADE"
            )]
        );

        let drop = validate(StructureActionRequest::DropConstraint {
            database: Some("app".to_string()),
            schema: None,
            table: "Orders".to_string(),
            name: "orders_user_fk".to_string(),
            kind: ConstraintKind::ForeignKey,
        });
        assert_eq!(
            preview(&drop).expect("drop constraint").statements,
            ["ALTER TABLE `app`.`Orders` DROP FOREIGN KEY `orders_user_fk`"]
        );
    }

    #[test]
    fn unsupported_engine_features_fail_before_ddl() {
        let check = validate(StructureActionRequest::AddConstraint {
            database: Some("app".to_string()),
            schema: None,
            table: "Users".to_string(),
            constraint: ConstraintRequest::Check {
                name: "users_email_check".to_string(),
                expression: "`Email` <> ''".to_string(),
            },
        });
        let mysql_57 = DatabaseCapabilities::for_engine(DatabaseEngine::Mysql, Some("5.7.44"));
        let error = check
            .ensure_supported(&mysql_57)
            .expect_err("MySQL 5.7 has no check constraints");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);

        let identity = validate(StructureActionRequest::AddColumn {
            database: Some("app".to_string()),
            schema: None,
            table: "Users".to_string(),
            column: {
                let mut column = column("id", "bigint");
                column.identity = Some(ColumnIdentity::ByDefault);
                column
            },
            position: None,
        });
        let error = preview(&identity).expect_err("MySQL has no standard identity");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);

        let partial = validate(StructureActionRequest::CreateIndex {
            database: Some("app".to_string()),
            schema: None,
            table: "Users".to_string(),
            name: "users_active".to_string(),
            columns: vec!["Email".to_string()],
            unique: false,
            method: None,
            predicate: Some("active = TRUE".to_string()),
        });
        let error = partial
            .ensure_supported(&DatabaseCapabilities::for_engine(
                DatabaseEngine::Mysql,
                Some("8.0.36"),
            ))
            .expect_err("MySQL has no partial indexes");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);
    }

    #[test]
    fn unqualified_names_get_an_explicit_warning() {
        let action = validate(StructureActionRequest::DropTable {
            database: None,
            schema: None,
            name: "users".to_string(),
        });
        let preview = preview(&action).expect("preview");
        assert_eq!(preview.statements, ["DROP TABLE `users`"]);
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("unqualified"))
        );
    }

    #[test]
    fn table_ref_supports_schema_as_mysql_database() {
        let table = TableRef::new(None, Some("app"), "users").expect("table");
        assert_eq!(
            MYSQL_DIALECT.quote_qualified_name(&table_name(&table).expect("name")),
            "`app`.`users`"
        );
        let mismatched = TableRef::new(Some("app"), Some("other"), "users").expect("table");
        assert!(table_name(&mismatched).is_err());
    }

    #[test]
    fn create_table_applies_unique_keys_and_mysql_table_options() {
        let action = validate(StructureActionRequest::CreateTable {
            database: Some("app".to_string()),
            schema: None,
            name: "Users".to_string(),
            columns: vec![
                column("ID", "bigint unsigned"),
                column("Email", "varchar(320)"),
            ],
            primary_key: vec!["ID".to_string()],
            unique_keys: vec![UniqueKeyRequest {
                name: "Users Email Key".to_string(),
                columns: vec!["Email".to_string()],
            }],
            options: Some(TableOptionsRequest {
                engine: Some("InnoDB".to_string()),
                charset: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_unicode_ci".to_string()),
                tablespace: None,
                unlogged: false,
                comment: Some("Users table".to_string()),
            }),
        });
        action.ensure_supported(&capabilities()).expect("supported");
        let preview = preview(&action).expect("preview");

        assert!(preview.statements[0].contains("UNIQUE KEY `Users Email Key` (`Email`)"));
        assert!(preview.statements[0].contains("ENGINE=InnoDB"));
        assert!(
            preview.statements[0]
                .contains("DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci")
        );
        assert!(preview.statements[0].contains("COMMENT='Users table'"));
    }

    #[test]
    fn duplicate_table_uses_create_table_like() {
        let action = validate(StructureActionRequest::DuplicateTable {
            database: Some("app".to_string()),
            schema: None,
            name: "Users".to_string(),
            new_name: "Accounts".to_string(),
        });
        let preview = preview(&action).expect("duplicate preview");
        assert_eq!(
            preview.statements,
            ["CREATE TABLE `app`.`Accounts` LIKE `app`.`Users`"]
        );
        assert!(!preview.destructive);
        assert!(!preview.requires_confirmation);
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("Rows are not copied"))
        );
    }

    #[test]
    fn postgres_only_table_options_are_rejected_before_ddl() {
        let action = validate(StructureActionRequest::CreateTable {
            database: Some("app".to_string()),
            schema: None,
            name: "Users".to_string(),
            columns: vec![column("id", "bigint")],
            primary_key: Vec::new(),
            unique_keys: Vec::new(),
            options: Some(TableOptionsRequest {
                tablespace: Some("fast".to_string()),
                ..TableOptionsRequest::default()
            }),
        });
        let error = preview(&action).expect_err("MySQL has no PostgreSQL tablespace option");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);
    }
}
