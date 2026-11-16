//! MySQL view, routine, and trigger DDL planning.
//!
//! MySQL has no sequences or materialized views. Those actions are rejected by
//! the capability gate before this planner runs. Definitions are rendered as
//! bounded user-authored SQL after validation; identifiers are always quoted.

use crate::database::{
    dialect::{Identifier, IdentifierQuoter, QualifiedName, SqlDialect},
    errors::DatabaseError,
    objects::{
        ObjectDdlPreview, ObjectRef, RoutineKind, TriggerEvent, ValidatedObjectAction,
        ValidatedTriggerBody,
    },
};

use super::MYSQL_DIALECT;

/// Builds a preview for one validated MySQL object action.
pub fn preview(action: &ValidatedObjectAction) -> Result<ObjectDdlPreview, DatabaseError> {
    match action {
        ValidatedObjectAction::CreateView {
            object,
            query,
            or_replace,
        } => {
            let name = object_name(object)?;
            action.preview(
                MYSQL_DIALECT.engine(),
                vec![format!(
                    "CREATE {}VIEW {} AS\n{}",
                    if *or_replace { "OR REPLACE " } else { "" },
                    MYSQL_DIALECT.quote_qualified_name(&name),
                    query
                )],
                vec![
                    "MySQL stores the view definition with DEFINER security semantics. Review who \
                     owns the view before exposing it to application accounts."
                        .to_string(),
                ],
            )
        }
        ValidatedObjectAction::DropView { object } => {
            if object.kind == crate::database::objects::DatabaseObjectKind::MaterializedView {
                return Err(DatabaseError::capability(
                    "MySQL does not support materialized views.",
                ));
            }
            action.preview(
                MYSQL_DIALECT.engine(),
                vec![format!(
                    "DROP VIEW {}",
                    MYSQL_DIALECT.quote_qualified_name(&object_name(object)?)
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
            let mut statements = Vec::new();
            let mut warnings = vec![
                "Routine bodies are executed by the database server with the definer's \
                 privileges. Review every referenced table and account."
                    .to_string(),
                "MySQL can reject function creation when binary logging is enabled unless the \
                 routine declares DETERMINISTIC, NO SQL, or READS SQL DATA. ServerDesk does not \
                 add a characteristic automatically."
                    .to_string(),
            ];
            if *or_replace {
                statements.push(format!(
                    "DROP {} IF EXISTS {}",
                    routine_kind.as_sql(),
                    MYSQL_DIALECT.quote_qualified_name(&name)
                ));
                warnings.push(
                    "MySQL has no CREATE OR REPLACE for routines. The preview drops the existing \
                     routine before recreating it, so a failed create leaves no routine."
                        .to_string(),
                );
            }
            let header = match routine_kind {
                RoutineKind::Function => {
                    let return_type = return_type.as_deref().ok_or_else(|| {
                        DatabaseError::validation("A MySQL function requires a return type.")
                    })?;
                    format!(
                        "CREATE FUNCTION {}({arguments}) RETURNS {return_type}",
                        MYSQL_DIALECT.quote_qualified_name(&name)
                    )
                }
                RoutineKind::Procedure => format!(
                    "CREATE PROCEDURE {}({arguments})",
                    MYSQL_DIALECT.quote_qualified_name(&name)
                ),
            };
            let mut statement = header;
            if let Some(language) = language {
                statement.push_str(" LANGUAGE ");
                statement.push_str(language.as_str());
            }
            statement.push('\n');
            statement.push_str(body);
            statements.push(statement);
            action.preview(MYSQL_DIALECT.engine(), statements, warnings)
        }
        ValidatedObjectAction::DropRoutine {
            object,
            routine_kind,
            ..
        } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "DROP {} {}",
                routine_kind.as_sql(),
                MYSQL_DIALECT.quote_qualified_name(&object_name(object)?)
            )],
            vec![
                "Dropping a routine permanently removes its body and grants. Application code \
                 that calls it will fail."
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
            if events.len() != 1 {
                return Err(DatabaseError::capability(
                    "MySQL supports exactly one event per trigger. Create one trigger for each \
                     INSERT, UPDATE, or DELETE event.",
                ));
            }
            if events.contains(&TriggerEvent::Truncate) {
                return Err(DatabaseError::capability(
                    "MySQL does not support TRUNCATE triggers.",
                ));
            }
            let body = match body {
                ValidatedTriggerBody::Statement(body) => body,
                ValidatedTriggerBody::Function { .. } => {
                    return Err(DatabaseError::capability(
                        "MySQL triggers use a statement body, not a PostgreSQL trigger function.",
                    ));
                }
            };
            let name = object_name(object)?;
            let table = trigger_table_name(object, table)?;
            let mut statements = Vec::new();
            let mut warnings = vec![
                "MySQL executes one trigger body per event and table. Trigger ordering is \
                 determined by creation order."
                    .to_string(),
            ];
            if *or_replace {
                statements.push(format!(
                    "DROP TRIGGER IF EXISTS {}",
                    MYSQL_DIALECT.quote_qualified_name(&name)
                ));
                warnings.push(
                    "MySQL has no CREATE OR REPLACE TRIGGER. The preview drops the existing \
                     trigger before recreating it, so a failed create leaves no trigger."
                        .to_string(),
                );
            }
            statements.push(format!(
                "CREATE TRIGGER {} {} {} ON {} FOR EACH ROW\n{}",
                MYSQL_DIALECT.quote_qualified_name(&name),
                timing.as_sql(),
                events[0].as_sql(),
                MYSQL_DIALECT.quote_qualified_name(&table),
                body
            ));
            action.preview(MYSQL_DIALECT.engine(), statements, warnings)
        }
        ValidatedObjectAction::DropTrigger { object, table } => action.preview(
            MYSQL_DIALECT.engine(),
            vec![format!(
                "DROP TRIGGER {}",
                MYSQL_DIALECT.quote_qualified_name(&object_name(object)?)
            )],
            vec![
                "Dropping a trigger removes the automatic action for every write to the target \
                 table."
                    .to_string(),
                format!(
                    "Target table: {}.",
                    MYSQL_DIALECT.quote_qualified_name(&trigger_table_name(object, table)?)
                ),
            ],
        ),
        ValidatedObjectAction::CreateSequence { .. }
        | ValidatedObjectAction::AlterSequence { .. }
        | ValidatedObjectAction::DropSequence { .. } => Err(DatabaseError::capability(
            "MySQL does not support sequences.",
        )),
        ValidatedObjectAction::RefreshMaterializedView { .. }
        | ValidatedObjectAction::DropMaterializedView { .. } => Err(DatabaseError::capability(
            "MySQL does not support materialized views.",
        )),
    }
}

fn object_name(object: &ObjectRef) -> Result<QualifiedName, DatabaseError> {
    let database = object.database.as_ref().or(object.schema.as_ref());
    QualifiedName::new(
        database
            .into_iter()
            .cloned()
            .chain(std::iter::once(object.name.clone())),
    )
}

fn trigger_table_name(
    object: &ObjectRef,
    table: &Identifier,
) -> Result<QualifiedName, DatabaseError> {
    let database = object.database.as_ref().or(object.schema.as_ref());
    QualifiedName::new(
        database
            .into_iter()
            .cloned()
            .chain(std::iter::once(table.clone())),
    )
}

impl RoutineKind {
    const fn as_sql(self) -> &'static str {
        match self {
            Self::Function => "FUNCTION",
            Self::Procedure => "PROCEDURE",
        }
    }
}
