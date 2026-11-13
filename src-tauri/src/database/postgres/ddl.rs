//! PostgreSQL table, column, index, and constraint DDL planning.
//!
//! A planner receives only [`ValidatedStructureAction`] values. PostgreSQL
//! column comments are emitted as `COMMENT ON` statements, type changes use
//! `ALTER COLUMN ... TYPE`, and unsupported operations such as stored-column
//! reordering fail before any DDL is returned.

use crate::database::{
    dialect::{Identifier, IdentifierQuoter, QualifiedName, SqlDialect},
    errors::DatabaseError,
    structure::{
        ColumnDefault, StructureDdlPreview, ValidatedColumnDefinition, ValidatedConstraint,
        ValidatedStructureAction,
    },
    traits::{ColumnIdentity, TableRef},
};

use super::POSTGRES_DIALECT;

/// Builds a preview for one validated PostgreSQL structure action.
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
                    "CONSTRAINT {} UNIQUE ({})",
                    POSTGRES_DIALECT.quote_identifier(&key.name),
                    quote_column_list(&key.columns)
                ));
            }
            let mut statements = vec![format!(
                "CREATE {}TABLE {} (\n  {}\n){}",
                if options.options.unlogged {
                    "UNLOGGED "
                } else {
                    ""
                },
                POSTGRES_DIALECT.quote_qualified_name(&table),
                definitions.join(",\n  "),
                render_table_options(&options.options)?
            )];
            statements.extend(comment_statements(&table, &options.columns));
            if let Some(comment) = options.options.comment.as_deref() {
                statements.push(format!(
                    "COMMENT ON TABLE {} IS {}",
                    POSTGRES_DIALECT.quote_qualified_name(&table),
                    string_literal(comment)
                ));
            }

            action.preview(
                POSTGRES_DIALECT.engine(),
                statements,
                vec![
                    "PostgreSQL records column order at creation time; later column additions \
                     always appear at the end."
                        .to_string(),
                ],
            )
        }
        ValidatedStructureAction::RenameTable { table, new_name } => {
            let old_name = table_name(table)?;
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![format!(
                    "ALTER TABLE {} RENAME TO {}",
                    POSTGRES_DIALECT.quote_qualified_name(&old_name),
                    POSTGRES_DIALECT.quote_identifier(new_name)
                )],
                vec![
                    "Saved queries, grants, and application code that use the old table name must \
                     be updated."
                        .to_string(),
                    "PostgreSQL updates dependent views, indexes, and constraints when the table \
                     is renamed."
                        .to_string(),
                ],
            )
        }
        ValidatedStructureAction::DuplicateTable { table, new_name } => {
            let old_name = table_name(table)?;
            let new_name = table_name_with(table, new_name)?;
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![format!(
                    "CREATE TABLE {} (LIKE {} INCLUDING ALL)",
                    POSTGRES_DIALECT.quote_qualified_name(&new_name),
                    POSTGRES_DIALECT.quote_qualified_name(&old_name)
                )],
                vec![
                    "The duplicate copies columns, indexes, defaults, constraints, comments, and \
                     identity options. Rows are not copied."
                        .to_string(),
                    "Foreign keys are not copied by CREATE TABLE ... LIKE; add them explicitly if \
                     the duplicate needs the same relationships."
                        .to_string(),
                ],
            )
        }
        ValidatedStructureAction::DropTable { table } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "DROP TABLE {}",
                POSTGRES_DIALECT.quote_qualified_name(&table_name(table)?)
            )],
            vec![
                "Dropping a table permanently removes every row and its schema definition."
                    .to_string(),
                "Incoming foreign keys, views, and triggers can block the drop or become invalid; \
                 inspect table dependencies first."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::TruncateTable { table } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "TRUNCATE TABLE {}",
                POSTGRES_DIALECT.quote_qualified_name(&table_name(table)?)
            )],
            vec![
                "TRUNCATE TABLE removes every row but keeps the table definition.".to_string(),
                "PostgreSQL can roll TRUNCATE back inside a transaction, but the rows are gone \
                 after commit."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::AddColumn {
            table,
            column,
            position,
        } => {
            if position.is_some() {
                return Err(DatabaseError::capability(
                    "PostgreSQL does not support positioning a new column at a specific ordinal. \
                     New columns are appended to the table.",
                ));
            }
            let table = table_name(table)?;
            let mut statements = vec![format!(
                "ALTER TABLE {} ADD COLUMN {}",
                POSTGRES_DIALECT.quote_qualified_name(&table),
                render_column(column)?
            )];
            statements.extend(comment_statements(&table, std::slice::from_ref(column)));
            let mut warnings = Vec::new();
            if !column.nullable && column.default.is_none() && column.generated.is_none() {
                warnings.push(
                    "Adding a NOT NULL column without a default fails when the table already \
                     contains rows."
                        .to_string(),
                );
            }
            action.preview(POSTGRES_DIALECT.engine(), statements, warnings)
        }
        ValidatedStructureAction::AlterColumn {
            table,
            name,
            changes,
        } => {
            let table_name = table_name(table)?;
            let qualified = POSTGRES_DIALECT.quote_qualified_name(&table_name);
            let column = POSTGRES_DIALECT.quote_identifier(name);
            let mut statements = Vec::new();
            let mut warnings = Vec::new();

            if let Some(data_type) = changes.data_type.as_deref() {
                statements.push(format!(
                    "ALTER TABLE {qualified} ALTER COLUMN {column} TYPE {data_type}"
                ));
                warnings.push(
                    "PostgreSQL may need a USING expression for an incompatible type change and \
                     can rewrite the table."
                        .to_string(),
                );
            }
            if let Some(nullable) = changes.nullable {
                if nullable {
                    statements.push(format!(
                        "ALTER TABLE {qualified} ALTER COLUMN {column} DROP NOT NULL"
                    ));
                } else {
                    statements.push(format!(
                        "ALTER TABLE {qualified} ALTER COLUMN {column} SET NOT NULL"
                    ));
                    warnings.push(
                        "SET NOT NULL scans the table and fails when existing rows contain NULL."
                            .to_string(),
                    );
                }
            }
            if let Some(default) = changes.default.as_ref() {
                statements.push(format!(
                    "ALTER TABLE {qualified} ALTER COLUMN {column} SET DEFAULT {}",
                    render_default(default)
                ));
            } else if changes.drop_default {
                statements.push(format!(
                    "ALTER TABLE {qualified} ALTER COLUMN {column} DROP DEFAULT"
                ));
            }
            if let Some(comment) = changes.comment.as_deref() {
                statements.push(format!(
                    "COMMENT ON COLUMN {qualified}.{column} IS {}",
                    string_literal(comment)
                ));
            } else if changes.drop_comment {
                statements.push(format!("COMMENT ON COLUMN {qualified}.{column} IS NULL"));
            }

            if statements.is_empty() {
                return Err(DatabaseError::validation(
                    "The PostgreSQL column change does not include a supported alteration.",
                ));
            }
            action.preview(POSTGRES_DIALECT.engine(), statements, warnings)
        }
        ValidatedStructureAction::RenameColumn {
            table,
            name,
            new_name,
        } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {}",
                POSTGRES_DIALECT.quote_qualified_name(&table_name(table)?),
                POSTGRES_DIALECT.quote_identifier(name),
                POSTGRES_DIALECT.quote_identifier(new_name)
            )],
            vec![
                "PostgreSQL updates views, indexes, and constraints that reference the column."
                    .to_string(),
                "Saved queries and application code must use the new column name.".to_string(),
            ],
        ),
        ValidatedStructureAction::DropColumn { table, name } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} DROP COLUMN {}",
                POSTGRES_DIALECT.quote_qualified_name(&table_name(table)?),
                POSTGRES_DIALECT.quote_identifier(name)
            )],
            vec![
                "Dropping a column permanently removes its data.".to_string(),
                "Dependent views, indexes, and constraints block the drop unless they are removed \
                 first."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::ReorderColumn { .. } => {
            return Err(DatabaseError::capability(
                "PostgreSQL does not support reordering stored columns. Column order has no \
                 effect on query results; add a new column or recreate the table if the physical \
                 order matters.",
            ));
        }
        ValidatedStructureAction::CreateIndex(options) => {
            let table = table_name(&options.table)?;
            let mut statement = String::from("CREATE ");
            if options.unique {
                statement.push_str("UNIQUE ");
            }
            statement.push_str("INDEX ");
            statement.push_str(&POSTGRES_DIALECT.quote_identifier(&options.name));
            statement.push_str(" ON ");
            statement.push_str(&POSTGRES_DIALECT.quote_qualified_name(&table));
            if let Some(method) = options.method.as_deref() {
                statement.push_str(" USING ");
                statement.push_str(validate_method(method)?);
            }
            statement.push_str(" (");
            statement.push_str(&quote_column_list(&options.columns));
            statement.push(')');
            if let Some(predicate) = options.predicate.as_deref() {
                statement.push_str(" WHERE (");
                statement.push_str(predicate);
                statement.push(')');
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
            action.preview(POSTGRES_DIALECT.engine(), vec![statement], warnings)
        }
        ValidatedStructureAction::RenameIndex {
            table,
            name,
            new_name,
        } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "ALTER INDEX {} RENAME TO {}",
                POSTGRES_DIALECT.quote_qualified_name(&index_name(table, name)?),
                POSTGRES_DIALECT.quote_identifier(new_name)
            )],
            vec![
                "PostgreSQL updates dependent constraints and query plans that name the index."
                    .to_string(),
            ],
        ),
        ValidatedStructureAction::DropIndex { table, name } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "DROP INDEX {}",
                POSTGRES_DIALECT.quote_qualified_name(&index_name(table, name)?)
            )],
            vec![
                "Dropping an index does not delete table data but can make queries slower."
                    .to_string(),
                "A constraint that depends on the index can block the drop.".to_string(),
            ],
        ),
        ValidatedStructureAction::AddConstraint { table, constraint } => {
            let table_name = table_name(table)?;
            let statement = render_add_constraint(&table_name, constraint)?;
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![statement],
                vec![
                    "Adding a constraint validates existing rows and can temporarily lock writes."
                        .to_string(),
                ],
            )
        }
        ValidatedStructureAction::DropConstraint { table, name, .. } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                POSTGRES_DIALECT.quote_qualified_name(&table_name(table)?),
                POSTGRES_DIALECT.quote_identifier(name)
            )],
            vec![
                "Dropping a constraint does not delete rows, but existing data may violate the \
                 rule afterwards."
                    .to_string(),
            ],
        ),
    };

    if preview.object.schema.is_none() {
        preview.warnings.push(
            "The table name is unqualified and resolves through the PostgreSQL search_path."
                .to_string(),
        );
    }
    Ok(preview)
}

fn table_name(table: &TableRef) -> Result<QualifiedName, DatabaseError> {
    match &table.schema {
        Some(schema) => QualifiedName::new([schema.clone(), table.name.clone()]),
        None => QualifiedName::new([table.name.clone()]),
    }
}

fn table_name_with(
    table: &TableRef,
    new_name: &Identifier,
) -> Result<QualifiedName, DatabaseError> {
    match &table.schema {
        Some(schema) => QualifiedName::new([schema.clone(), new_name.clone()]),
        None => QualifiedName::new([new_name.clone()]),
    }
}

fn render_table_options(
    options: &crate::database::structure::ValidatedTableOptions,
) -> Result<String, DatabaseError> {
    if options.engine.is_some() || options.charset.is_some() || options.collation.is_some() {
        return Err(DatabaseError::capability(
            "PostgreSQL does not support a MySQL-style storage engine or per-table character set \
             and collation option. Use the database encoding, collation, or a per-column collation \
             instead.",
        ));
    }

    let mut suffix = String::new();
    if let Some(tablespace) = options.tablespace.as_ref() {
        suffix.push_str(" TABLESPACE ");
        suffix.push_str(&POSTGRES_DIALECT.quote_identifier(tablespace));
    }
    Ok(suffix)
}

fn index_name(table: &TableRef, index: &Identifier) -> Result<QualifiedName, DatabaseError> {
    match &table.schema {
        Some(schema) => QualifiedName::new([schema.clone(), index.clone()]),
        None => QualifiedName::new([index.clone()]),
    }
}

fn quote_column_list(columns: &[Identifier]) -> String {
    columns
        .iter()
        .map(|column| POSTGRES_DIALECT.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_column(column: &ValidatedColumnDefinition) -> Result<String, DatabaseError> {
    if column.generated.is_some() && column.identity != ColumnIdentity::None {
        return Err(DatabaseError::validation(
            "A generated column cannot also have an identity column option.",
        ));
    }

    let mut definition = format!(
        "{} {}",
        POSTGRES_DIALECT.quote_identifier(&column.name),
        column.data_type
    );

    if let Some(generated) = column.generated.as_ref() {
        if !generated.stored {
            return Err(DatabaseError::capability(
                "PostgreSQL supports only STORED generated columns; virtual generated columns \
                 are not available.",
            ));
        }
        definition.push_str(&format!(
            " GENERATED ALWAYS AS ({}) STORED",
            generated.expression
        ));
    } else {
        match column.identity {
            ColumnIdentity::Always => {
                definition.push_str(" GENERATED ALWAYS AS IDENTITY");
            }
            ColumnIdentity::ByDefault => {
                definition.push_str(" GENERATED BY DEFAULT AS IDENTITY");
            }
            ColumnIdentity::AutoIncrement => {
                return Err(DatabaseError::capability(
                    "PostgreSQL does not support AUTO_INCREMENT. Use a GENERATED ... AS IDENTITY \
                     column or a sequence-backed default instead.",
                ));
            }
            ColumnIdentity::None => {}
        }
    }

    definition.push_str(if column.nullable {
        " NULL"
    } else {
        " NOT NULL"
    });
    if column.generated.is_none()
        && column.identity == ColumnIdentity::None
        && let Some(default) = column.default.as_ref()
    {
        definition.push_str(" DEFAULT ");
        definition.push_str(&render_default(default));
    }
    Ok(definition)
}

fn comment_statements(table: &QualifiedName, columns: &[ValidatedColumnDefinition]) -> Vec<String> {
    let table = POSTGRES_DIALECT.quote_qualified_name(table);
    columns
        .iter()
        .filter_map(|column| {
            column.comment.as_deref().map(|comment| {
                format!(
                    "COMMENT ON COLUMN {table}.{} IS {}",
                    POSTGRES_DIALECT.quote_identifier(&column.name),
                    string_literal(comment)
                )
            })
        })
        .collect()
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
    let table = POSTGRES_DIALECT.quote_qualified_name(table);
    match constraint {
        ValidatedConstraint::PrimaryKey { name, columns } => Ok(format!(
            "ALTER TABLE {table} ADD CONSTRAINT {} PRIMARY KEY ({})",
            POSTGRES_DIALECT.quote_identifier(name),
            quote_column_list(columns)
        )),
        ValidatedConstraint::Unique { name, columns } => Ok(format!(
            "ALTER TABLE {table} ADD CONSTRAINT {} UNIQUE ({})",
            POSTGRES_DIALECT.quote_identifier(name),
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
                POSTGRES_DIALECT.quote_identifier(name),
                quote_column_list(columns),
                POSTGRES_DIALECT.quote_qualified_name(&referenced),
                quote_column_list(referenced_columns)
            );
            if let Some(on_delete) = on_delete {
                statement.push_str(" ON DELETE ");
                statement.push_str(on_delete.as_sql());
            }
            if let Some(on_update) = on_update {
                statement.push_str(" ON UPDATE ");
                statement.push_str(on_update.as_sql());
            }
            Ok(statement)
        }
        ValidatedConstraint::Check { name, expression } => Ok(format!(
            "ALTER TABLE {table} ADD CONSTRAINT {} CHECK ({})",
            POSTGRES_DIALECT.quote_identifier(name),
            expression
        )),
    }
}

fn validate_method(method: &str) -> Result<&'static str, DatabaseError> {
    match method {
        "btree" => Ok("btree"),
        "hash" => Ok("hash"),
        "gist" => Ok("gist"),
        "spgist" => Ok("spgist"),
        "gin" => Ok("gin"),
        "brin" => Ok("brin"),
        other => Err(DatabaseError::capability(format!(
            "PostgreSQL does not support the {other} index method."
        ))
        .with_details("Use btree, hash, gist, spgist, gin, or brin.")),
    }
}

/// PostgreSQL string literal escaping. A literal that contains a backslash is
/// emitted as an E-string so it stays correct even if
/// `standard_conforming_strings` is disabled.
fn string_literal(value: &str) -> String {
    if value.contains('\\') {
        format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

#[cfg(test)]
mod tests {
    use crate::database::{
        DatabaseErrorKind, ReferentialAction, TableRef,
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

    fn capabilities(version: &str) -> DatabaseCapabilities {
        DatabaseCapabilities::for_engine(DatabaseEngine::Postgresql, Some(version))
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
    fn create_table_uses_postgres_identity_generated_and_comment_syntax() {
        let mut id = column("ID", "bigint");
        id.identity = Some(ColumnIdentity::ByDefault);
        let mut slug = column("Slug Value", "text");
        slug.generated = Some(crate::database::GeneratedColumnRequest {
            expression: "lower(\"Email\")".to_string(),
            stored: true,
        });
        let mut email = column("Email", "varchar(255)");
        email.default = Some(crate::database::ColumnDefaultRequest::String {
            value: "n/a".to_string(),
        });
        email.comment = Some("Login address".to_string());

        let action = validate(StructureActionRequest::CreateTable {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "Users".to_string(),
            columns: vec![id, slug, email],
            primary_key: vec!["ID".to_string()],
            unique_keys: Vec::new(),
            options: None,
        });
        action
            .ensure_supported(&capabilities("16.2"))
            .expect("supported");
        let preview = preview(&action).expect("preview");

        assert_eq!(
            preview.statements,
            [
                concat!(
                    "CREATE TABLE \"public\".\"Users\" (\n",
                    "  \"ID\" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL,\n",
                    "  \"Slug Value\" text GENERATED ALWAYS AS (lower(\"Email\")) STORED NOT NULL,\n",
                    "  \"Email\" varchar(255) NOT NULL DEFAULT 'n/a',\n",
                    "  PRIMARY KEY (\"ID\")\n",
                    ")"
                ),
                "COMMENT ON COLUMN \"public\".\"Users\".\"Email\" IS 'Login address'"
            ]
        );
        assert_eq!(preview.ddl.split(";\n").count(), 2);
    }

    #[test]
    fn alter_column_emits_separate_postgres_statements() {
        let action = validate(StructureActionRequest::AlterColumn {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Users".to_string(),
            name: "Email".to_string(),
            data_type: Some("varchar(320)".to_string()),
            nullable: Some(false),
            default: Some(crate::database::ColumnDefaultRequest::String {
                value: "n/a".to_string(),
            }),
            drop_default: false,
            comment: Some("Updated address".to_string()),
            drop_comment: false,
        });
        action
            .ensure_supported(&capabilities("16.2"))
            .expect("supported");
        let preview = preview(&action).expect("alter preview");
        assert_eq!(
            preview.statements,
            [
                "ALTER TABLE \"public\".\"Users\" ALTER COLUMN \"Email\" TYPE varchar(320)",
                "ALTER TABLE \"public\".\"Users\" ALTER COLUMN \"Email\" SET NOT NULL",
                "ALTER TABLE \"public\".\"Users\" ALTER COLUMN \"Email\" SET DEFAULT 'n/a'",
                "COMMENT ON COLUMN \"public\".\"Users\".\"Email\" IS 'Updated address'"
            ]
        );
        assert!(preview.destructive);
    }

    #[test]
    fn index_predicates_and_identifiers_are_engine_specific() {
        let index = validate(StructureActionRequest::CreateIndex {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Users".to_string(),
            name: "Users Active Index".to_string(),
            columns: vec!["Email".to_string()],
            unique: true,
            method: Some("btree".to_string()),
            predicate: Some("\"active\" = TRUE".to_string()),
        });
        index
            .ensure_supported(&capabilities("16.2"))
            .expect("supported");
        assert_eq!(
            preview(&index).expect("index").statements,
            [concat!(
                "CREATE UNIQUE INDEX \"Users Active Index\" ON \"public\".\"Users\" ",
                "USING btree (\"Email\") WHERE (\"active\" = TRUE)"
            )]
        );

        let rename = validate(StructureActionRequest::RenameIndex {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Users".to_string(),
            name: "Users Active Index".to_string(),
            new_name: "users_active_idx".to_string(),
        });
        assert_eq!(
            preview(&rename).expect("rename").statements,
            ["ALTER INDEX \"public\".\"Users Active Index\" RENAME TO \"users_active_idx\""]
        );
    }

    #[test]
    fn postgres_rejects_auto_increment_virtual_columns_and_reorder_before_ddl() {
        let auto_increment = validate(StructureActionRequest::AddColumn {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Users".to_string(),
            column: {
                let mut column = column("id", "bigint");
                column.identity = Some(ColumnIdentity::AutoIncrement);
                column
            },
            position: None,
        });
        let error = preview(&auto_increment).expect_err("no auto increment");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);
        assert!(error.message.contains("AUTO_INCREMENT"));

        let virtual_column = validate(StructureActionRequest::AddColumn {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Users".to_string(),
            column: {
                let mut column = column("slug", "text");
                column.generated = Some(crate::database::GeneratedColumnRequest {
                    expression: "lower(email)".to_string(),
                    stored: false,
                });
                column
            },
            position: None,
        });
        let error = preview(&virtual_column).expect_err("no virtual columns");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);

        let reorder = validate(StructureActionRequest::ReorderColumn {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Users".to_string(),
            column: column("email", "text"),
            position: crate::database::ColumnPositionRequest::First,
        });
        let error = reorder
            .ensure_supported(&capabilities("16.2"))
            .expect_err("no reorder");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);
        assert!(error.message.contains("reorder_columns"));
    }

    #[test]
    fn foreign_key_and_check_constraints_use_postgres_forms() {
        let fk = validate(StructureActionRequest::AddConstraint {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Orders".to_string(),
            constraint: ConstraintRequest::ForeignKey {
                name: "orders_user_fk".to_string(),
                columns: vec!["User ID".to_string()],
                referenced_schema: Some("public".to_string()),
                referenced_table: "Users".to_string(),
                referenced_columns: vec!["ID".to_string()],
                on_delete: Some(ReferentialAction::Cascade),
                on_update: Some(ReferentialAction::NoAction),
            },
        });
        fk.ensure_supported(&capabilities("16.2"))
            .expect("supported");
        assert_eq!(
            preview(&fk).expect("fk").statements,
            [concat!(
                "ALTER TABLE \"public\".\"Orders\" ADD CONSTRAINT \"orders_user_fk\" FOREIGN KEY ",
                "(\"User ID\") REFERENCES \"public\".\"Users\" (\"ID\") ON DELETE CASCADE ",
                "ON UPDATE NO ACTION"
            )]
        );

        let check = validate(StructureActionRequest::AddConstraint {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            table: "Users".to_string(),
            constraint: ConstraintRequest::Check {
                name: "users_email_check".to_string(),
                expression: "\"Email\" <> ''".to_string(),
            },
        });
        assert_eq!(
            preview(&check).expect("check").statements,
            [
                "ALTER TABLE \"public\".\"Users\" ADD CONSTRAINT \"users_email_check\" CHECK (\"Email\" <> '')"
            ]
        );
    }

    #[test]
    fn rename_table_keeps_the_new_name_unqualified() {
        let action = validate(StructureActionRequest::RenameTable {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "Users".to_string(),
            new_name: "Accounts".to_string(),
        });
        assert_eq!(
            preview(&action).expect("rename").statements,
            ["ALTER TABLE \"public\".\"Users\" RENAME TO \"Accounts\""]
        );
    }

    #[test]
    fn unqualified_names_warn_about_search_path() {
        let action = validate(StructureActionRequest::DropTable {
            database: None,
            schema: None,
            name: "users".to_string(),
        });
        let preview = preview(&action).expect("preview");
        assert_eq!(preview.statements, ["DROP TABLE \"users\""]);
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("search_path"))
        );
    }

    #[test]
    fn table_name_ignores_database_because_postgres_is_one_catalog() {
        let table = TableRef::new(Some("app"), Some("public"), "users").expect("table");
        assert_eq!(
            POSTGRES_DIALECT.quote_qualified_name(&table_name(&table).expect("name")),
            "\"public\".\"users\""
        );
    }

    #[test]
    fn create_table_applies_postgres_keys_options_and_table_comment() {
        let action = validate(StructureActionRequest::CreateTable {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "Users".to_string(),
            columns: vec![column("ID", "bigint"), column("Email", "varchar(320)")],
            primary_key: vec!["ID".to_string()],
            unique_keys: vec![UniqueKeyRequest {
                name: "Users Email Key".to_string(),
                columns: vec!["Email".to_string()],
            }],
            options: Some(TableOptionsRequest {
                tablespace: Some("fast".to_string()),
                unlogged: true,
                comment: Some("Users table".to_string()),
                ..TableOptionsRequest::default()
            }),
        });
        action
            .ensure_supported(&capabilities("16.2"))
            .expect("supported");
        let preview = preview(&action).expect("preview");

        assert!(preview.statements[0].starts_with("CREATE UNLOGGED TABLE \"public\".\"Users\""));
        assert!(
            preview.statements[0].contains("CONSTRAINT \"Users Email Key\" UNIQUE (\"Email\")")
        );
        assert!(preview.statements[0].contains("TABLESPACE \"fast\""));
        assert!(preview.statements.iter().any(|statement| {
            statement == "COMMENT ON TABLE \"public\".\"Users\" IS 'Users table'"
        }));
    }

    #[test]
    fn duplicate_table_uses_postgres_like_including_all() {
        let action = validate(StructureActionRequest::DuplicateTable {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "Users".to_string(),
            new_name: "Accounts".to_string(),
        });
        let preview = preview(&action).expect("duplicate preview");
        assert_eq!(
            preview.statements,
            ["CREATE TABLE \"public\".\"Accounts\" (LIKE \"public\".\"Users\" INCLUDING ALL)"]
        );
        assert!(!preview.destructive);
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("Rows are not copied"))
        );
    }

    #[test]
    fn mysql_only_table_options_are_rejected_before_ddl() {
        let action = validate(StructureActionRequest::CreateTable {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "Users".to_string(),
            columns: vec![column("id", "bigint")],
            primary_key: Vec::new(),
            unique_keys: Vec::new(),
            options: Some(TableOptionsRequest {
                engine: Some("InnoDB".to_string()),
                ..TableOptionsRequest::default()
            }),
        });
        let error = preview(&action).expect_err("PostgreSQL has no MySQL engine option");
        assert_eq!(error.kind, DatabaseErrorKind::Capability);
    }
}
