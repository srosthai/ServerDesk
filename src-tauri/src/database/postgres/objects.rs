//! PostgreSQL view, routine, trigger, sequence, and materialized-view DDL.
//!
//! Identifiers are always quoted. User-authored bodies are bounded before they
//! reach this planner and are rendered as SQL string literals or statement
//! bodies; they are never interpolated into identifiers or metadata queries.

use crate::database::{
    dialect::{Identifier, IdentifierQuoter, QualifiedName, SqlDialect},
    errors::DatabaseError,
    objects::{
        DatabaseObjectKind, ObjectDdlPreview, ObjectRef, RoutineKind, SequenceDataType,
        TriggerEvent, ValidatedObjectAction, ValidatedTriggerBody,
    },
};

use super::POSTGRES_DIALECT;

/// Builds a preview for one validated PostgreSQL object action.
pub fn preview(action: &ValidatedObjectAction) -> Result<ObjectDdlPreview, DatabaseError> {
    match action {
        ValidatedObjectAction::CreateView {
            object,
            query,
            or_replace,
        } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "CREATE {}VIEW {} AS\n{}",
                if *or_replace { "OR REPLACE " } else { "" },
                POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?),
                query
            )],
            vec![
                "PostgreSQL evaluates a view with the view owner's permissions by default. Review \
                 row-level security and grants before exposing it."
                    .to_string(),
            ],
        ),
        ValidatedObjectAction::DropView { object } => {
            if object.kind == DatabaseObjectKind::MaterializedView {
                return drop_materialized_view(action, object);
            }
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![format!(
                    "DROP VIEW {}",
                    POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?)
                )],
                vec![
                    "Dropping a view removes the definition; dependent views and application \
                     queries can fail."
                        .to_string(),
                ],
            )
        }
        ValidatedObjectAction::CreateRoutine {
            object,
            routine_kind,
            arguments,
            return_type,
            language,
            body,
            or_replace,
        } => {
            let name = object_name(object)?;
            let arguments = arguments.as_deref().unwrap_or("");
            let language = language
                .as_ref()
                .map(|value| POSTGRES_DIALECT.quote_identifier(value))
                .unwrap_or_else(|| "plpgsql".to_string());
            let header = match routine_kind {
                RoutineKind::Function => {
                    let return_type = return_type.as_deref().ok_or_else(|| {
                        DatabaseError::validation("A PostgreSQL function requires a return type.")
                    })?;
                    format!(
                        "CREATE {}FUNCTION {}({arguments}) RETURNS {return_type}\nLANGUAGE {language}",
                        if *or_replace { "OR REPLACE " } else { "" },
                        POSTGRES_DIALECT.quote_qualified_name(&name)
                    )
                }
                RoutineKind::Procedure => format!(
                    "CREATE {}PROCEDURE {}({arguments})\nLANGUAGE {language}",
                    if *or_replace { "OR REPLACE " } else { "" },
                    POSTGRES_DIALECT.quote_qualified_name(&name)
                ),
            };
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![format!("{header}\nAS {}", string_literal(body))],
                vec![
                    "Routine bodies execute with the privileges of the routine owner. Review \
                     SECURITY DEFINER settings and search_path before deploying."
                        .to_string(),
                    "CREATE OR REPLACE cannot change a function's return type or argument types. \
                     Drop and recreate the routine when its signature changes."
                        .to_string(),
                ],
            )
        }
        ValidatedObjectAction::DropRoutine {
            object,
            routine_kind,
            arguments,
        } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "DROP {} {}({})",
                match routine_kind {
                    RoutineKind::Function => "FUNCTION",
                    RoutineKind::Procedure => "PROCEDURE",
                },
                POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?),
                arguments.as_deref().unwrap_or("")
            )],
            vec![
                "Dropping a routine permanently removes its body and grants. Application code \
                 that calls it will fail."
                    .to_string(),
                "PostgreSQL identifies functions by name and argument types. Verify the argument \
                 list before dropping an overloaded routine."
                    .to_string(),
            ],
        ),
        ValidatedObjectAction::CreateTrigger {
            object,
            table,
            timing,
            events,
            body,
            or_replace,
        } => {
            let function = match body {
                ValidatedTriggerBody::Function {
                    schema,
                    name,
                    arguments,
                } => {
                    let schema = schema.as_ref().or(object.schema.as_ref());
                    let qualified = QualifiedName::new(
                        schema
                            .into_iter()
                            .cloned()
                            .chain(std::iter::once(name.clone())),
                    )?;
                    format!(
                        "{}({})",
                        POSTGRES_DIALECT.quote_qualified_name(&qualified),
                        arguments.as_deref().unwrap_or("")
                    )
                }
                ValidatedTriggerBody::Statement(_) => {
                    return Err(DatabaseError::capability(
                        "PostgreSQL triggers execute a trigger function. Provide the function name \
                         and arguments instead of a statement body.",
                    ));
                }
            };
            let table = trigger_table_name(object, table)?;
            let mut statements = Vec::new();
            let mut warnings = vec![
                "The trigger function must return trigger and must already exist. PostgreSQL does \
                 not store the body on the trigger itself."
                    .to_string(),
            ];
            if *or_replace {
                statements.push(format!(
                    "DROP TRIGGER IF EXISTS {} ON {}",
                    POSTGRES_DIALECT.quote_identifier(&object.name),
                    POSTGRES_DIALECT.quote_qualified_name(&table)
                ));
                warnings.push(
                    "CREATE OR REPLACE TRIGGER requires PostgreSQL 14 or newer. The preview drops \
                     and recreates the trigger so older servers follow the same path; a failed \
                     create leaves no trigger."
                        .to_string(),
                );
            }
            statements.push(format!(
                "CREATE TRIGGER {} {} {} ON {} FOR EACH ROW\nEXECUTE FUNCTION {}",
                POSTGRES_DIALECT.quote_identifier(&object.name),
                timing.as_sql(),
                render_events(events),
                POSTGRES_DIALECT.quote_qualified_name(&table),
                function
            ));
            action.preview(POSTGRES_DIALECT.engine(), statements, warnings)
        }
        ValidatedObjectAction::DropTrigger { object, table } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "DROP TRIGGER {} ON {}",
                POSTGRES_DIALECT.quote_identifier(&object.name),
                POSTGRES_DIALECT.quote_qualified_name(&trigger_table_name(object, table)?)
            )],
            vec![
                "Dropping a trigger removes the automatic action for every write to the target \
                 table. The trigger function itself is not dropped."
                    .to_string(),
            ],
        ),
        ValidatedObjectAction::CreateSequence {
            object,
            data_type,
            start,
            increment,
            minimum,
            maximum,
            cache,
            cycle,
        } => {
            let mut statement = format!(
                "CREATE SEQUENCE {}",
                POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?)
            );
            if let Some(data_type) = data_type {
                statement.push_str(" AS ");
                statement.push_str(sequence_type(*data_type));
            }
            if let Some(increment) = increment {
                statement.push_str(&format!(" INCREMENT BY {increment}"));
            }
            if let Some(minimum) = minimum {
                statement.push_str(&format!(" MINVALUE {minimum}"));
            }
            if let Some(maximum) = maximum {
                statement.push_str(&format!(" MAXVALUE {maximum}"));
            }
            if let Some(start) = start {
                statement.push_str(&format!(" START WITH {start}"));
            }
            if let Some(cache) = cache {
                statement.push_str(&format!(" CACHE {cache}"));
            }
            statement.push_str(if *cycle { " CYCLE" } else { " NO CYCLE" });
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![statement],
                vec![
                    "Sequence values are not transactional in the same way as row data; a rolled \
                     back transaction can still consume numbers."
                        .to_string(),
                ],
            )
        }
        ValidatedObjectAction::AlterSequence {
            object,
            increment,
            minimum,
            maximum,
            restart,
            cache,
            cycle,
        } => {
            let mut statement = format!(
                "ALTER SEQUENCE {}",
                POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?)
            );
            if let Some(increment) = increment {
                statement.push_str(&format!(" INCREMENT BY {increment}"));
            }
            if let Some(minimum) = minimum {
                statement.push_str(&format!(" MINVALUE {minimum}"));
            }
            if let Some(maximum) = maximum {
                statement.push_str(&format!(" MAXVALUE {maximum}"));
            }
            if let Some(restart) = restart {
                statement.push_str(&format!(" RESTART WITH {restart}"));
            }
            if let Some(cache) = cache {
                statement.push_str(&format!(" CACHE {cache}"));
            }
            if let Some(cycle) = cycle {
                statement.push_str(if *cycle { " CYCLE" } else { " NO CYCLE" });
            }
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![statement],
                vec![
                    "Changing a sequence does not change defaults that already reference it, but \
                     RESTART can collide with values already stored in a table."
                        .to_string(),
                ],
            )
        }
        ValidatedObjectAction::DropSequence { object } => action.preview(
            POSTGRES_DIALECT.engine(),
            vec![format!(
                "DROP SEQUENCE {}",
                POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?)
            )],
            vec![
                "Dropping a sequence permanently removes its counter. Columns with a default that \
                 calls nextval on this sequence will fail."
                    .to_string(),
            ],
        ),
        ValidatedObjectAction::RefreshMaterializedView { object } => {
            if object.kind != DatabaseObjectKind::MaterializedView {
                return Err(DatabaseError::validation(
                    "Only a materialized view can be refreshed.",
                ));
            }
            action.preview(
                POSTGRES_DIALECT.engine(),
                vec![format!(
                    "REFRESH MATERIALIZED VIEW {}",
                    POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?)
                )],
                vec![
                    "REFRESH MATERIALIZED VIEW takes an exclusive lock and blocks reads until the \
                     refresh finishes. Use CONCURRENTLY outside ServerDesk when the view has a \
                     suitable unique index."
                        .to_string(),
                ],
            )
        }
        ValidatedObjectAction::DropMaterializedView { object } => {
            drop_materialized_view(action, object)
        }
    }
}

fn drop_materialized_view(
    action: &ValidatedObjectAction,
    object: &ObjectRef,
) -> Result<ObjectDdlPreview, DatabaseError> {
    action.preview(
        POSTGRES_DIALECT.engine(),
        vec![format!(
            "DROP MATERIALIZED VIEW {}",
            POSTGRES_DIALECT.quote_qualified_name(&object_name(object)?)
        )],
        vec![
            "Dropping a materialized view permanently removes its stored rows and definition."
                .to_string(),
        ],
    )
}

fn object_name(object: &ObjectRef) -> Result<QualifiedName, DatabaseError> {
    QualifiedName::new(
        object
            .schema
            .clone()
            .into_iter()
            .chain(std::iter::once(object.name.clone())),
    )
}

fn trigger_table_name(
    object: &ObjectRef,
    table: &Identifier,
) -> Result<QualifiedName, DatabaseError> {
    QualifiedName::new(
        object
            .schema
            .clone()
            .into_iter()
            .chain(std::iter::once(table.clone())),
    )
}

fn render_events(events: &[TriggerEvent]) -> String {
    events
        .iter()
        .map(|event| event.as_sql())
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn sequence_type(data_type: SequenceDataType) -> &'static str {
    match data_type {
        SequenceDataType::SmallInt => "smallint",
        SequenceDataType::Integer => "integer",
        SequenceDataType::BigInt => "bigint",
    }
}

fn string_literal(value: &str) -> String {
    if value.contains('\\') {
        format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}
