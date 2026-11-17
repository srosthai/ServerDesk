//! Typed view, routine, trigger, sequence, and materialized-view management.
//!
//! Requests cross the Tauri boundary as untrusted JSON. [`ObjectActionRequest::validate`]
//! turns them into [`ValidatedObjectAction`] values with every identifier checked
//! by the shared [`Identifier`] rules and every definition bounded before an
//! engine planner sees it. Definitions are user-authored SQL bodies, so they are
//! never logged, never placed in history summaries, and always rendered as text
//! by the UI rather than interpreted as markup.
//!
//! Execution asks the adapter for the same preview first, so capability checks,
//! read-only checks, and destructive confirmations cannot be bypassed. Metadata
//! is re-read after execution before success is reported.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{core::AppError, storage::DatabaseEngine};

use super::{
    capabilities::{DatabaseAction as CapabilityAction, DatabaseCapabilities},
    dialect::Identifier,
    errors::{DatabaseError, DatabaseErrorKind},
    queries::{SqlToken, split_sql_statements},
    service::DatabaseConnectionRegistry,
    traits::{DatabaseAdapter, MetadataValue},
};

/// Longest accepted definition or routine body returned to the UI.
pub const MAX_OBJECT_DEFINITION_LENGTH: usize = 65_536;
/// Longest accepted view query.
pub const MAX_OBJECT_QUERY_LENGTH: usize = 32_768;
/// Longest accepted routine or trigger body.
pub const MAX_OBJECT_BODY_LENGTH: usize = 32_768;
/// Longest accepted routine argument list.
pub const MAX_ROUTINE_ARGUMENTS_LENGTH: usize = 2_000;
/// Longest accepted routine return type.
pub const MAX_ROUTINE_RETURN_TYPE_LENGTH: usize = 128;
/// Longest accepted trigger function argument list.
pub const MAX_TRIGGER_ARGUMENTS_LENGTH: usize = 1_000;
/// Maximum number of trigger events in one definition.
pub const MAX_TRIGGER_EVENTS: usize = 4;

/// The kind of database object managed by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseObjectKind {
    View,
    MaterializedView,
    Routine,
    Trigger,
    Sequence,
}

impl DatabaseObjectKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::MaterializedView => "materialized_view",
            Self::Routine => "routine",
            Self::Trigger => "trigger",
            Self::Sequence => "sequence",
        }
    }
}

/// PostgreSQL and MySQL routine kinds. MySQL functions and procedures share
/// the same metadata catalog; PostgreSQL distinguishes them with `prokind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineKind {
    Function,
    Procedure,
}

impl RoutineKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Procedure => "procedure",
        }
    }

    pub const fn postgres_kind(self) -> &'static str {
        match self {
            Self::Function => "f",
            Self::Procedure => "p",
        }
    }
}

/// Trigger timing relative to the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerTiming {
    Before,
    After,
    InsteadOf,
}

impl TriggerTiming {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Before => "BEFORE",
            Self::After => "AFTER",
            Self::InsteadOf => "INSTEAD OF",
        }
    }

    pub fn from_mysql(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "BEFORE" => Some(Self::Before),
            "AFTER" => Some(Self::After),
            _ => None,
        }
    }

    pub fn from_postgres(trigger_type: i32) -> Self {
        if trigger_type & 2 != 0 {
            Self::Before
        } else if trigger_type & 64 != 0 {
            Self::InsteadOf
        } else {
            Self::After
        }
    }
}

/// One trigger event. PostgreSQL supports TRUNCATE triggers; MySQL does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
    Truncate,
}

impl TriggerEvent {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
        }
    }

    pub fn from_mysql(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "INSERT" => Some(Self::Insert),
            "UPDATE" => Some(Self::Update),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn from_postgres(trigger_type: i32) -> Vec<Self> {
        let mut events = Vec::new();
        if trigger_type & 4 != 0 {
            events.push(Self::Insert);
        }
        if trigger_type & 16 != 0 {
            events.push(Self::Update);
        }
        if trigger_type & 8 != 0 {
            events.push(Self::Delete);
        }
        if trigger_type & 32 != 0 {
            events.push(Self::Truncate);
        }
        events
    }
}

/// PostgreSQL sequence integer types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceDataType {
    SmallInt,
    Integer,
    BigInt,
}

impl SequenceDataType {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::SmallInt => "smallint",
            Self::Integer => "integer",
            Self::BigInt => "bigint",
        }
    }

    pub fn from_metadata(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "smallint" => Some(Self::SmallInt),
            "integer" => Some(Self::Integer),
            "bigint" => Some(Self::BigInt),
            _ => None,
        }
    }
}

/// A bounded routine summary for the explorer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineSummary {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: RoutineKind,
    pub language: MetadataValue<String>,
    pub return_type: MetadataValue<String>,
    pub arguments: MetadataValue<String>,
    pub owner: MetadataValue<String>,
}

/// A bounded trigger summary for the explorer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSummary {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub table: String,
    pub timing: TriggerTiming,
    pub events: Vec<TriggerEvent>,
    pub owner: MetadataValue<String>,
}

/// A validated object reference used by inspect, preview, and execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRef {
    pub kind: DatabaseObjectKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Identifier>,
    pub name: Identifier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routine_kind: Option<RoutineKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Identifier>,
}

impl ObjectRef {
    pub fn label(&self) -> String {
        let mut label = String::new();
        if let Some(database) = self.database.as_ref() {
            label.push_str(database.as_str());
            label.push('.');
        }
        if let Some(schema) = self.schema.as_ref()
            && Some(schema.as_str()) != self.database.as_ref().map(Identifier::as_str)
        {
            label.push_str(schema.as_str());
            label.push('.');
        }
        label.push_str(self.name.as_str());
        label
    }

    pub fn qualified_table(&self) -> String {
        let mut label = String::new();
        if let Some(schema) = self.schema.as_ref() {
            label.push_str(schema.as_str());
            label.push('.');
        }
        label.push_str(self.name.as_str());
        label
    }
}

/// Untrusted object reference from React.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRefRequest {
    pub kind: DatabaseObjectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    #[serde(
        default,
        rename = "routineKind",
        skip_serializing_if = "Option::is_none"
    )]
    pub routine_kind: Option<RoutineKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
}

impl ObjectRefRequest {
    pub fn validate(&self) -> Result<ObjectRef, DatabaseError> {
        Ok(ObjectRef {
            kind: self.kind,
            database: self
                .database
                .as_deref()
                .map(|value| validate_identifier("database name", value))
                .transpose()?,
            schema: self
                .schema
                .as_deref()
                .map(|value| validate_identifier("schema name", value))
                .transpose()?,
            name: validate_identifier("object name", &self.name)?,
            routine_kind: self.routine_kind,
            arguments: self
                .arguments
                .as_deref()
                .map(|value| {
                    validate_fragment("routine arguments", value, MAX_ROUTINE_ARGUMENTS_LENGTH)
                })
                .transpose()?,
            table: self
                .table
                .as_deref()
                .map(|value| validate_identifier("trigger table", value))
                .transpose()?,
        })
    }
}

/// A typed trigger body. MySQL uses a statement body; PostgreSQL executes a
/// trigger function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerBodyRequest {
    Statement {
        body: String,
    },
    Function {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedTriggerBody {
    Statement(String),
    Function {
        schema: Option<Identifier>,
        name: Identifier,
        arguments: Option<String>,
    },
}

/// One typed object action requested by the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectActionRequest {
    CreateView {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        query: String,
        #[serde(default, rename = "orReplace")]
        or_replace: bool,
    },
    DropView {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(default, rename = "materialized")]
        materialized: bool,
    },
    CreateRoutine {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(rename = "routineKind")]
        routine_kind: RoutineKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<String>,
        #[serde(
            default,
            rename = "returnType",
            skip_serializing_if = "Option::is_none"
        )]
        return_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        body: String,
        #[serde(default, rename = "orReplace")]
        or_replace: bool,
    },
    DropRoutine {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(rename = "routineKind")]
        routine_kind: RoutineKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<String>,
    },
    CreateTrigger {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        table: String,
        timing: TriggerTiming,
        events: Vec<TriggerEvent>,
        body: TriggerBodyRequest,
        #[serde(default, rename = "orReplace")]
        or_replace: bool,
    },
    DropTrigger {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        table: String,
    },
    CreateSequence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(default, rename = "dataType", skip_serializing_if = "Option::is_none")]
        data_type: Option<SequenceDataType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        increment: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimum: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        maximum: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<i64>,
        #[serde(default)]
        cycle: bool,
    },
    AlterSequence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        increment: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimum: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        maximum: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restart: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cycle: Option<bool>,
    },
    DropSequence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
    },
    RefreshMaterializedView {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
    },
    DropMaterializedView {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        name: String,
    },
}

impl ObjectActionRequest {
    pub fn validate(&self) -> Result<ValidatedObjectAction, DatabaseError> {
        match self {
            Self::CreateView {
                database,
                schema,
                name,
                query,
                or_replace,
            } => {
                let object = object_ref(database, schema, name, DatabaseObjectKind::View)?;
                let query = validate_view_query(query)?;
                Ok(ValidatedObjectAction::CreateView {
                    object,
                    query,
                    or_replace: *or_replace,
                })
            }
            Self::DropView {
                database,
                schema,
                name,
                materialized,
            } => Ok(ValidatedObjectAction::DropView {
                object: object_ref(
                    database,
                    schema,
                    name,
                    if *materialized {
                        DatabaseObjectKind::MaterializedView
                    } else {
                        DatabaseObjectKind::View
                    },
                )?,
            }),
            Self::CreateRoutine {
                database,
                schema,
                name,
                routine_kind,
                arguments,
                return_type,
                language,
                body,
                or_replace,
            } => {
                let object = object_ref(database, schema, name, DatabaseObjectKind::Routine)?;
                let arguments = arguments
                    .as_deref()
                    .map(|value| {
                        validate_fragment("routine arguments", value, MAX_ROUTINE_ARGUMENTS_LENGTH)
                    })
                    .transpose()?;
                let return_type = return_type
                    .as_deref()
                    .map(|value| {
                        validate_fragment(
                            "routine return type",
                            value,
                            MAX_ROUTINE_RETURN_TYPE_LENGTH,
                        )
                    })
                    .transpose()?;
                if *routine_kind == RoutineKind::Function && return_type.is_none() {
                    return Err(DatabaseError::validation(
                        "A function definition requires a return type.",
                    ));
                }
                let language = language
                    .as_deref()
                    .map(validate_routine_language)
                    .transpose()?;
                let body = validate_definition("routine body", body, MAX_OBJECT_BODY_LENGTH)?;
                Ok(ValidatedObjectAction::CreateRoutine {
                    object,
                    routine_kind: *routine_kind,
                    arguments,
                    return_type,
                    language,
                    body,
                    or_replace: *or_replace,
                })
            }
            Self::DropRoutine {
                database,
                schema,
                name,
                routine_kind,
                arguments,
            } => Ok(ValidatedObjectAction::DropRoutine {
                object: object_ref(database, schema, name, DatabaseObjectKind::Routine)?,
                routine_kind: *routine_kind,
                arguments: arguments
                    .as_deref()
                    .map(|value| {
                        validate_fragment("routine arguments", value, MAX_ROUTINE_ARGUMENTS_LENGTH)
                    })
                    .transpose()?,
            }),
            Self::CreateTrigger {
                database,
                schema,
                name,
                table,
                timing,
                events,
                body,
                or_replace,
            } => {
                let object = object_ref(database, schema, name, DatabaseObjectKind::Trigger)?;
                let table = validate_identifier("trigger table", table)?;
                if events.is_empty() {
                    return Err(DatabaseError::validation(
                        "A trigger must listen for at least one event.",
                    ));
                }
                if events.len() > MAX_TRIGGER_EVENTS {
                    return Err(DatabaseError::validation(format!(
                        "A trigger cannot listen for more than {MAX_TRIGGER_EVENTS} events."
                    )));
                }
                let mut unique = events.clone();
                unique.sort_by_key(|event| event.as_sql());
                unique.dedup();
                if unique.len() != events.len() {
                    return Err(DatabaseError::validation(
                        "A trigger cannot listen for the same event twice.",
                    ));
                }
                let body = validate_trigger_body(body)?;
                Ok(ValidatedObjectAction::CreateTrigger {
                    object,
                    table,
                    timing: *timing,
                    events: events.clone(),
                    body,
                    or_replace: *or_replace,
                })
            }
            Self::DropTrigger {
                database,
                schema,
                name,
                table,
            } => Ok(ValidatedObjectAction::DropTrigger {
                object: object_ref(database, schema, name, DatabaseObjectKind::Trigger)?,
                table: validate_identifier("trigger table", table)?,
            }),
            Self::CreateSequence {
                database,
                schema,
                name,
                data_type,
                start,
                increment,
                minimum,
                maximum,
                cache,
                cycle,
            } => {
                validate_sequence_bounds(*minimum, *maximum, *increment, *start, *cache)?;
                Ok(ValidatedObjectAction::CreateSequence {
                    object: object_ref(database, schema, name, DatabaseObjectKind::Sequence)?,
                    data_type: *data_type,
                    start: *start,
                    increment: *increment,
                    minimum: *minimum,
                    maximum: *maximum,
                    cache: *cache,
                    cycle: *cycle,
                })
            }
            Self::AlterSequence {
                database,
                schema,
                name,
                increment,
                minimum,
                maximum,
                restart,
                cache,
                cycle,
            } => {
                if increment.is_none()
                    && minimum.is_none()
                    && maximum.is_none()
                    && restart.is_none()
                    && cache.is_none()
                    && cycle.is_none()
                {
                    return Err(DatabaseError::validation(
                        "A sequence change must set an increment, bound, restart value, cache, or cycle option.",
                    ));
                }
                validate_sequence_bounds(*minimum, *maximum, *increment, *restart, *cache)?;
                Ok(ValidatedObjectAction::AlterSequence {
                    object: object_ref(database, schema, name, DatabaseObjectKind::Sequence)?,
                    increment: *increment,
                    minimum: *minimum,
                    maximum: *maximum,
                    restart: *restart,
                    cache: *cache,
                    cycle: *cycle,
                })
            }
            Self::DropSequence {
                database,
                schema,
                name,
            } => Ok(ValidatedObjectAction::DropSequence {
                object: object_ref(database, schema, name, DatabaseObjectKind::Sequence)?,
            }),
            Self::RefreshMaterializedView {
                database,
                schema,
                name,
            } => Ok(ValidatedObjectAction::RefreshMaterializedView {
                object: object_ref(database, schema, name, DatabaseObjectKind::MaterializedView)?,
            }),
            Self::DropMaterializedView {
                database,
                schema,
                name,
            } => Ok(ValidatedObjectAction::DropMaterializedView {
                object: object_ref(database, schema, name, DatabaseObjectKind::MaterializedView)?,
            }),
        }
    }
}

/// A validated object action. Engine planners rely on every identifier and
/// definition here already being bounded and safe to quote or render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedObjectAction {
    CreateView {
        object: ObjectRef,
        query: String,
        or_replace: bool,
    },
    DropView {
        object: ObjectRef,
    },
    CreateRoutine {
        object: ObjectRef,
        routine_kind: RoutineKind,
        arguments: Option<String>,
        return_type: Option<String>,
        language: Option<Identifier>,
        body: String,
        or_replace: bool,
    },
    DropRoutine {
        object: ObjectRef,
        routine_kind: RoutineKind,
        arguments: Option<String>,
    },
    CreateTrigger {
        object: ObjectRef,
        table: Identifier,
        timing: TriggerTiming,
        events: Vec<TriggerEvent>,
        body: ValidatedTriggerBody,
        or_replace: bool,
    },
    DropTrigger {
        object: ObjectRef,
        table: Identifier,
    },
    CreateSequence {
        object: ObjectRef,
        data_type: Option<SequenceDataType>,
        start: Option<i64>,
        increment: Option<i64>,
        minimum: Option<i64>,
        maximum: Option<i64>,
        cache: Option<i64>,
        cycle: bool,
    },
    AlterSequence {
        object: ObjectRef,
        increment: Option<i64>,
        minimum: Option<i64>,
        maximum: Option<i64>,
        restart: Option<i64>,
        cache: Option<i64>,
        cycle: Option<bool>,
    },
    DropSequence {
        object: ObjectRef,
    },
    RefreshMaterializedView {
        object: ObjectRef,
    },
    DropMaterializedView {
        object: ObjectRef,
    },
}

impl ValidatedObjectAction {
    pub fn kind(&self) -> ObjectActionKind {
        match self {
            Self::CreateView { or_replace, .. } => {
                if *or_replace {
                    ObjectActionKind::ReplaceView
                } else {
                    ObjectActionKind::CreateView
                }
            }
            Self::DropView { object } => {
                if object.kind == DatabaseObjectKind::MaterializedView {
                    ObjectActionKind::DropMaterializedView
                } else {
                    ObjectActionKind::DropView
                }
            }
            Self::CreateRoutine { or_replace, .. } => {
                if *or_replace {
                    ObjectActionKind::ReplaceRoutine
                } else {
                    ObjectActionKind::CreateRoutine
                }
            }
            Self::DropRoutine { .. } => ObjectActionKind::DropRoutine,
            Self::CreateTrigger { or_replace, .. } => {
                if *or_replace {
                    ObjectActionKind::ReplaceTrigger
                } else {
                    ObjectActionKind::CreateTrigger
                }
            }
            Self::DropTrigger { .. } => ObjectActionKind::DropTrigger,
            Self::CreateSequence { .. } => ObjectActionKind::CreateSequence,
            Self::AlterSequence { .. } => ObjectActionKind::AlterSequence,
            Self::DropSequence { .. } => ObjectActionKind::DropSequence,
            Self::RefreshMaterializedView { .. } => ObjectActionKind::RefreshMaterializedView,
            Self::DropMaterializedView { .. } => ObjectActionKind::DropMaterializedView,
        }
    }

    pub fn object(&self) -> &ObjectRef {
        match self {
            Self::CreateView { object, .. }
            | Self::DropView { object }
            | Self::CreateRoutine { object, .. }
            | Self::DropRoutine { object, .. }
            | Self::CreateTrigger { object, .. }
            | Self::DropTrigger { object, .. }
            | Self::CreateSequence { object, .. }
            | Self::AlterSequence { object, .. }
            | Self::DropSequence { object }
            | Self::RefreshMaterializedView { object }
            | Self::DropMaterializedView { object } => object,
        }
    }

    pub fn capability(&self) -> CapabilityAction {
        match self {
            Self::CreateView { or_replace, .. } => {
                if *or_replace {
                    CapabilityAction::ReplaceView
                } else {
                    CapabilityAction::CreateView
                }
            }
            Self::DropView { object } => {
                if object.kind == DatabaseObjectKind::MaterializedView {
                    CapabilityAction::DropMaterializedView
                } else {
                    CapabilityAction::DropView
                }
            }
            Self::CreateRoutine { or_replace, .. } => {
                if *or_replace {
                    CapabilityAction::EditRoutine
                } else {
                    CapabilityAction::CreateRoutine
                }
            }
            Self::DropRoutine { .. } => CapabilityAction::DropRoutine,
            Self::CreateTrigger { or_replace, .. } => {
                if *or_replace {
                    CapabilityAction::EditTrigger
                } else {
                    CapabilityAction::CreateTrigger
                }
            }
            Self::DropTrigger { .. } => CapabilityAction::DropTrigger,
            Self::CreateSequence { .. } => CapabilityAction::CreateSequence,
            Self::AlterSequence { .. } => CapabilityAction::AlterSequence,
            Self::DropSequence { .. } => CapabilityAction::DropSequence,
            Self::RefreshMaterializedView { .. } => CapabilityAction::RefreshMaterializedView,
            Self::DropMaterializedView { .. } => CapabilityAction::DropMaterializedView,
        }
    }

    pub fn ensure_supported(
        &self,
        capabilities: &DatabaseCapabilities,
    ) -> Result<(), DatabaseError> {
        capabilities.ensure(self.capability())?;
        match self {
            Self::CreateView { object, .. }
                if object.kind == DatabaseObjectKind::MaterializedView =>
            {
                capabilities.ensure(CapabilityAction::BrowseMaterializedViews)?;
            }
            Self::CreateTrigger { events, .. }
                if events.contains(&TriggerEvent::Truncate)
                    && capabilities.engine() == DatabaseEngine::Mysql =>
            {
                return Err(DatabaseError::capability(
                    "MySQL does not support TRUNCATE triggers.",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    pub const fn destructive(&self) -> bool {
        matches!(
            self,
            Self::DropView { .. }
                | Self::DropRoutine { .. }
                | Self::DropTrigger { .. }
                | Self::DropSequence { .. }
                | Self::DropMaterializedView { .. }
        )
    }

    /// Replacing a view, routine, or trigger can break dependents and is
    /// therefore treated as a confirmation-required action even though it does
    /// not delete a data object.
    pub const fn requires_confirmation(&self) -> bool {
        self.destructive()
            || matches!(
                self,
                Self::CreateView {
                    or_replace: true,
                    ..
                } | Self::CreateRoutine {
                    or_replace: true,
                    ..
                } | Self::CreateTrigger {
                    or_replace: true,
                    ..
                }
            )
    }

    pub fn preview(
        &self,
        engine: DatabaseEngine,
        statements: Vec<String>,
        warnings: Vec<String>,
    ) -> Result<ObjectDdlPreview, DatabaseError> {
        let ddl = statements.join(";\n");
        Ok(ObjectDdlPreview {
            action: self.kind(),
            engine,
            object: self.object().clone(),
            statements,
            ddl,
            warnings,
            destructive: self.destructive(),
            requires_confirmation: self.requires_confirmation(),
        })
    }
}

/// Object action names used by previews and history entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectActionKind {
    CreateView,
    ReplaceView,
    DropView,
    CreateRoutine,
    ReplaceRoutine,
    DropRoutine,
    CreateTrigger,
    ReplaceTrigger,
    DropTrigger,
    CreateSequence,
    AlterSequence,
    DropSequence,
    RefreshMaterializedView,
    DropMaterializedView,
}

impl ObjectActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateView => "create_view",
            Self::ReplaceView => "replace_view",
            Self::DropView => "drop_view",
            Self::CreateRoutine => "create_routine",
            Self::ReplaceRoutine => "replace_routine",
            Self::DropRoutine => "drop_routine",
            Self::CreateTrigger => "create_trigger",
            Self::ReplaceTrigger => "replace_trigger",
            Self::DropTrigger => "drop_trigger",
            Self::CreateSequence => "create_sequence",
            Self::AlterSequence => "alter_sequence",
            Self::DropSequence => "drop_sequence",
            Self::RefreshMaterializedView => "refresh_materialized_view",
            Self::DropMaterializedView => "drop_materialized_view",
        }
    }
}

/// Trigger details returned by inspect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDetails {
    pub table: String,
    pub timing: TriggerTiming,
    pub events: Vec<TriggerEvent>,
    pub statement: MetadataValue<String>,
    pub function_name: MetadataValue<String>,
    pub function: MetadataValue<String>,
}

/// PostgreSQL sequence details returned by inspect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceDetails {
    pub data_type: MetadataValue<String>,
    pub start_value: MetadataValue<String>,
    pub increment: MetadataValue<String>,
    pub minimum: MetadataValue<String>,
    pub maximum: MetadataValue<String>,
    pub current_value: MetadataValue<String>,
    pub cache: MetadataValue<String>,
    pub cycle: MetadataValue<bool>,
}

/// Bounded object definition returned by inspect. Definitions are text, not
/// HTML, and callers must render them as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDefinition {
    pub kind: DatabaseObjectKind,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub owner: MetadataValue<String>,
    pub language: MetadataValue<String>,
    pub routine_kind: MetadataValue<RoutineKind>,
    pub arguments: MetadataValue<String>,
    pub return_type: MetadataValue<String>,
    pub view_query: MetadataValue<String>,
    pub routine_body: MetadataValue<String>,
    pub definition: MetadataValue<String>,
    pub trigger: Option<TriggerDetails>,
    pub sequence: Option<SequenceDetails>,
    pub truncated: bool,
}

impl ObjectDefinition {
    /// Bounds every definition field before it crosses the command boundary.
    /// The returned flag tells the UI that at least one field was truncated.
    pub fn bounded(mut self) -> Self {
        let mut truncated = self.truncated;
        let mut bound = |value: MetadataValue<String>| {
            value.map(|value| {
                if value.len() <= MAX_OBJECT_DEFINITION_LENGTH {
                    return value;
                }
                let mut end = MAX_OBJECT_DEFINITION_LENGTH;
                while end > 0 && !value.is_char_boundary(end) {
                    end -= 1;
                }
                truncated = true;
                format!(
                    "{}\n-- definition truncated by ServerDesk at {MAX_OBJECT_DEFINITION_LENGTH} bytes --",
                    &value[..end]
                )
            })
        };
        self.owner = bound(self.owner);
        self.language = bound(self.language);
        self.arguments = bound(self.arguments);
        self.return_type = bound(self.return_type);
        self.view_query = bound(self.view_query);
        self.routine_body = bound(self.routine_body);
        self.definition = bound(self.definition);
        if let Some(trigger) = self.trigger.as_mut() {
            trigger.statement = bound(std::mem::take(&mut trigger.statement));
            trigger.function_name = bound(std::mem::take(&mut trigger.function_name));
            trigger.function = bound(std::mem::take(&mut trigger.function));
        }
        self.truncated = truncated;
        self
    }
}

/// Generated DDL for one object action. `statements` is the stable contract;
/// `ddl` is the same statements joined with `;` for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDdlPreview {
    pub action: ObjectActionKind,
    pub engine: DatabaseEngine,
    pub object: ObjectRef,
    pub statements: Vec<String>,
    pub ddl: String,
    pub warnings: Vec<String>,
    pub destructive: bool,
    pub requires_confirmation: bool,
}

/// Verified outcome of one object action. Definitions are intentionally not
/// copied into history; the result is only returned to the requesting UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectActionResult {
    pub action: ObjectActionKind,
    pub engine: DatabaseEngine,
    pub object: ObjectRef,
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
    pub verified: bool,
    pub verification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<ObjectDefinition>,
    pub duration_ms: u64,
}

/// Stateless preview/execute/inspect service used by object commands.
pub struct DatabaseObjectService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseObjectService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn inspect(
        &self,
        profile_id: &str,
        object: ObjectRefRequest,
    ) -> Result<ObjectDefinition, AppError> {
        let object = object.validate()?;
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        adapter
            .object_definition(&object)
            .await
            .map(ObjectDefinition::bounded)
            .map_err(AppError::from)
    }

    pub async fn preview(
        &self,
        profile_id: &str,
        request: ObjectActionRequest,
    ) -> Result<ObjectDdlPreview, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let adapter = adapter.lock().await;
        preview_object_with_adapter(adapter.as_ref(), &request)
            .await
            .map_err(AppError::from)
    }

    pub async fn execute(
        &self,
        profile_id: &str,
        request: ObjectActionRequest,
        confirmed: bool,
    ) -> Result<ObjectActionResult, AppError> {
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        execute_object_with_adapter(adapter.as_mut(), request, confirmed)
            .await
            .map_err(AppError::from)
    }
}

/// Validates a request and asks the adapter for a preview. Execution always
/// calls this same path first so a caller cannot skip capability checks, the
/// read-only gate, or confirmation.
pub async fn preview_object_with_adapter(
    adapter: &dyn DatabaseAdapter,
    request: &ObjectActionRequest,
) -> Result<ObjectDdlPreview, DatabaseError> {
    let validated = request.validate()?;
    adapter.preview_object_action(&validated).await
}

/// Executes one validated object action and re-reads metadata before reporting
/// success.
pub async fn execute_object_with_adapter(
    adapter: &mut dyn DatabaseAdapter,
    request: ObjectActionRequest,
    confirmed: bool,
) -> Result<ObjectActionResult, DatabaseError> {
    let started = Instant::now();
    let validated = request.validate()?;
    let preview = preview_object_with_adapter(adapter, &request).await?;

    if preview.requires_confirmation && !confirmed {
        return Err(DatabaseError::cancelled(format!(
            "The {} action for {} was cancelled because it was not confirmed.",
            validated.kind().as_str(),
            validated.object().label()
        ))
        .with_details(
            "No statement was executed. Review the connection, engine, object, warnings, and the \
             exact preview, then confirm the destructive or replacement action.",
        ));
    }

    if adapter.is_read_only() {
        return Err(DatabaseError::permission(
            "The database connection profile is read-only; object changes require a writable \
             connection.",
        )
        .with_scope(
            validated.object().kind.as_str(),
            validated.object().name.as_str(),
        ));
    }

    adapter.execute_object_action(&validated).await?;

    let verification = verify_object_action(adapter, &validated).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(ObjectActionResult {
        action: validated.kind(),
        engine: adapter.engine(),
        object: preview.object,
        statements: preview.statements,
        warnings: preview.warnings,
        verified: verification.verified,
        verification: verification.summary,
        verification_error: verification.error,
        definition: verification.definition,
        duration_ms,
    })
}

struct ObjectVerification {
    verified: bool,
    summary: String,
    error: Option<String>,
    definition: Option<ObjectDefinition>,
}

async fn verify_object_action(
    adapter: &dyn DatabaseAdapter,
    action: &ValidatedObjectAction,
) -> ObjectVerification {
    let object = action.object();
    let expected_absent = matches!(
        action,
        ValidatedObjectAction::DropView { .. }
            | ValidatedObjectAction::DropRoutine { .. }
            | ValidatedObjectAction::DropTrigger { .. }
            | ValidatedObjectAction::DropSequence { .. }
            | ValidatedObjectAction::DropMaterializedView { .. }
    );

    match adapter.object_definition(object).await {
        Ok(definition) if expected_absent => ObjectVerification {
            verified: false,
            summary: format!(
                "Metadata still lists {} {} after the drop.",
                object.kind.as_str(),
                object.label()
            ),
            error: Some("The object still exists; inspect it before retrying.".to_string()),
            definition: Some(definition.bounded()),
        },
        Ok(definition) => {
            let matches = definition_matches(action, &definition);
            let definition = definition.bounded();
            ObjectVerification {
                verified: matches,
                summary: if matches {
                    format!(
                        "Metadata re-read confirms the {} {} matches the requested change.",
                        object.kind.as_str(),
                        object.label()
                    )
                } else {
                    format!(
                        "Metadata re-read found {} {} but it does not match the requested change.",
                        object.kind.as_str(),
                        object.label()
                    )
                },
                error: (!matches).then(|| {
                    "The server reported success, but the inspected metadata differs from the \
                     requested definition. Inspect the object before retrying."
                        .to_string()
                }),
                definition: Some(definition),
            }
        }
        Err(error) if expected_absent && matches!(error.kind, DatabaseErrorKind::Query) => {
            ObjectVerification {
                verified: true,
                summary: format!(
                    "Metadata re-read confirms {} {} is absent.",
                    object.kind.as_str(),
                    object.label()
                ),
                error: None,
                definition: None,
            }
        }
        Err(error) if expected_absent => ObjectVerification {
            verified: false,
            summary: format!(
                "The drop finished, but metadata verification could not confirm that {} {} is \
                 absent.",
                object.kind.as_str(),
                object.label()
            ),
            error: Some(error.message),
            definition: None,
        },
        Err(error) => ObjectVerification {
            verified: false,
            summary: format!(
                "The action finished, but metadata verification could not read {} {}.",
                object.kind.as_str(),
                object.label()
            ),
            error: Some(error.message),
            definition: None,
        },
    }
}

fn definition_matches(action: &ValidatedObjectAction, definition: &ObjectDefinition) -> bool {
    match action {
        ValidatedObjectAction::CreateView { query, .. } => definition
            .view_query
            .value()
            .is_some_and(|value| normalize_definition(value) == normalize_definition(query)),
        ValidatedObjectAction::CreateRoutine {
            routine_kind,
            language,
            ..
        } => {
            definition.routine_kind.value() == Some(routine_kind)
                && language.as_ref().is_none_or(|language| {
                    definition
                        .language
                        .value()
                        .is_some_and(|value| value.eq_ignore_ascii_case(language.as_str()))
                })
                && definition.definition.is_available()
        }
        ValidatedObjectAction::CreateTrigger {
            table,
            timing,
            events,
            ..
        } => definition.trigger.as_ref().is_some_and(|trigger| {
            trigger.table == table.as_str()
                && trigger.timing == *timing
                && trigger.events == *events
                && (trigger.statement.is_available() || trigger.function.is_available())
        }),
        ValidatedObjectAction::CreateSequence {
            increment, cycle, ..
        } => definition.sequence.as_ref().is_some_and(|sequence| {
            increment.is_none_or(|expected| {
                sequence
                    .increment
                    .value()
                    .is_some_and(|value| value == &expected.to_string())
            }) && sequence.cycle.value().is_none_or(|value| *value == *cycle)
        }),
        ValidatedObjectAction::AlterSequence {
            increment,
            restart,
            cycle,
            ..
        } => definition.sequence.as_ref().is_some_and(|sequence| {
            increment.is_none_or(|expected| {
                sequence
                    .increment
                    .value()
                    .is_some_and(|value| value == &expected.to_string())
            }) && restart.is_none_or(|expected| {
                sequence
                    .current_value
                    .value()
                    .is_some_and(|value| value == &expected.to_string())
            }) && cycle.is_none_or(|expected| sequence.cycle.value() == Some(&expected))
        }),
        ValidatedObjectAction::RefreshMaterializedView { .. }
        | ValidatedObjectAction::DropView { .. }
        | ValidatedObjectAction::DropRoutine { .. }
        | ValidatedObjectAction::DropTrigger { .. }
        | ValidatedObjectAction::DropSequence { .. }
        | ValidatedObjectAction::DropMaterializedView { .. } => true,
    }
}

fn normalize_definition(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn object_ref(
    database: &Option<String>,
    schema: &Option<String>,
    name: &str,
    kind: DatabaseObjectKind,
) -> Result<ObjectRef, DatabaseError> {
    Ok(ObjectRef {
        kind,
        database: database
            .as_deref()
            .map(|value| validate_identifier("database name", value))
            .transpose()?,
        schema: schema
            .as_deref()
            .map(|value| validate_identifier("schema name", value))
            .transpose()?,
        name: validate_identifier("object name", name)?,
        routine_kind: None,
        arguments: None,
        table: None,
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

/// Routine language names are rendered beside `LANGUAGE`. They must therefore
/// be a plain identifier, not arbitrary text that could start a new statement.
fn validate_routine_language(value: &str) -> Result<Identifier, DatabaseError> {
    let value = value.trim();
    if value.is_empty() || value.len() > crate::database::dialect::MAX_IDENTIFIER_LENGTH {
        return Err(DatabaseError::validation(
            "The routine language must be between 1 and 63 bytes.",
        ));
    }
    let mut characters = value.chars();
    let first = characters.next().unwrap_or(' ');
    if !(first.is_ascii_alphabetic() || first == '_')
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(DatabaseError::validation(
            "The routine language must be a plain identifier.",
        )
        .with_details(
            "Use letters, digits, and '_', starting with a letter or '_'. Statement separators, quotes, dashes, and spaces are rejected.",
        ));
    }
    Identifier::new(value).map_err(|error| {
        DatabaseError::validation("The routine language is not valid.").with_details(error.message)
    })
}

fn validate_fragment(label: &str, value: &str, maximum: usize) -> Result<String, DatabaseError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(DatabaseError::validation(format!(
            "The {label} cannot be empty."
        )));
    }
    if value.len() > maximum {
        return Err(DatabaseError::validation(format!(
            "The {label} cannot exceed {maximum} bytes."
        )));
    }
    if value.contains('\0')
        || value.chars().any(|character| {
            character.is_control() && character != '\n' && character != '\t' && character != '\r'
        })
    {
        return Err(DatabaseError::validation(format!(
            "The {label} cannot contain NUL or control characters."
        )));
    }
    if value.contains(';') || value.contains("--") || value.contains("/*") || value.contains("*/") {
        return Err(DatabaseError::validation(format!(
            "The {label} cannot contain statement separators or SQL comments."
        )));
    }
    Ok(value.to_string())
}

fn validate_definition(label: &str, value: &str, maximum: usize) -> Result<String, DatabaseError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(DatabaseError::validation(format!(
            "The {label} cannot be empty."
        )));
    }
    if value.len() > maximum {
        return Err(DatabaseError::validation(format!(
            "The {label} cannot exceed {maximum} bytes."
        )));
    }
    if value.contains('\0')
        || value.chars().any(|character| {
            character.is_control() && character != '\n' && character != '\t' && character != '\r'
        })
    {
        return Err(DatabaseError::validation(format!(
            "The {label} cannot contain NUL or control characters."
        )));
    }
    Ok(value.to_string())
}

fn validate_view_query(value: &str) -> Result<String, DatabaseError> {
    let query = validate_definition("view query", value, MAX_OBJECT_QUERY_LENGTH)?;
    let statements = split_sql_statements(DatabaseEngine::Postgresql, &query)?;
    if statements.len() != 1 {
        return Err(DatabaseError::validation(
            "A view query must contain exactly one SELECT statement.",
        ));
    }
    let first = statements[0].tokens.iter().find_map(|token| match token {
        SqlToken::Word(word) => Some(word.as_str()),
        SqlToken::Symbol(_) => None,
    });
    if !matches!(first, Some("select" | "with" | "values" | "table")) {
        return Err(DatabaseError::validation(
            "A view query must start with SELECT, WITH, VALUES, or TABLE.",
        ));
    }
    Ok(query.trim_end_matches(';').trim().to_string())
}

fn validate_trigger_body(body: &TriggerBodyRequest) -> Result<ValidatedTriggerBody, DatabaseError> {
    match body {
        TriggerBodyRequest::Statement { body } => Ok(ValidatedTriggerBody::Statement(
            validate_definition("trigger body", body, MAX_OBJECT_BODY_LENGTH)?,
        )),
        TriggerBodyRequest::Function {
            schema,
            name,
            arguments,
        } => Ok(ValidatedTriggerBody::Function {
            schema: schema
                .as_deref()
                .map(|value| validate_identifier("trigger function schema", value))
                .transpose()?,
            name: validate_identifier("trigger function name", name)?,
            arguments: arguments
                .as_deref()
                .map(|value| {
                    validate_fragment(
                        "trigger function arguments",
                        value,
                        MAX_TRIGGER_ARGUMENTS_LENGTH,
                    )
                })
                .transpose()?,
        }),
    }
}

fn validate_sequence_bounds(
    minimum: Option<i64>,
    maximum: Option<i64>,
    increment: Option<i64>,
    start: Option<i64>,
    cache: Option<i64>,
) -> Result<(), DatabaseError> {
    if increment == Some(0) {
        return Err(DatabaseError::validation(
            "A sequence increment cannot be zero.",
        ));
    }
    if let (Some(minimum), Some(maximum)) = (minimum, maximum)
        && minimum >= maximum
    {
        return Err(DatabaseError::validation(
            "A sequence minimum must be less than its maximum.",
        ));
    }
    if let Some(start) = start {
        if minimum.is_some_and(|minimum| start < minimum) {
            return Err(DatabaseError::validation(
                "A sequence start value cannot be below its minimum.",
            ));
        }
        if maximum.is_some_and(|maximum| start > maximum) {
            return Err(DatabaseError::validation(
                "A sequence start value cannot be above its maximum.",
            ));
        }
    }
    if cache.is_some_and(|cache| cache < 1) {
        return Err(DatabaseError::validation(
            "A sequence cache must be at least 1.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_are_truncated_at_a_character_boundary() {
        let long = format!("{}\u{00e9}", "x".repeat(MAX_OBJECT_DEFINITION_LENGTH));
        let definition = ObjectDefinition {
            kind: DatabaseObjectKind::View,
            database: None,
            schema: None,
            name: "long_view".to_string(),
            owner: MetadataValue::Unavailable,
            language: MetadataValue::Unavailable,
            routine_kind: MetadataValue::Unavailable,
            arguments: MetadataValue::Unavailable,
            return_type: MetadataValue::Unavailable,
            view_query: MetadataValue::Unavailable,
            routine_body: MetadataValue::Unavailable,
            definition: MetadataValue::available(long),
            trigger: None,
            sequence: None,
            truncated: false,
        }
        .bounded();

        assert!(definition.truncated);
        assert!(
            definition
                .definition
                .value()
                .is_some_and(|value| value.len() <= MAX_OBJECT_DEFINITION_LENGTH + 100)
        );
    }

    #[test]
    fn frontend_camel_case_requests_deserialize() {
        let request: ObjectActionRequest = serde_json::from_value(serde_json::json!({
            "kind": "create_routine",
            "database": "app",
            "schema": "public",
            "name": "user_email",
            "routineKind": "function",
            "arguments": "user_id bigint",
            "returnType": "text",
            "language": "sql",
            "body": "SELECT email FROM users WHERE id = user_id",
            "orReplace": true
        }))
        .expect("routine request");

        let validated = request.validate().expect("valid routine");
        assert_eq!(validated.kind(), ObjectActionKind::ReplaceRoutine);
        assert_eq!(validated.object().label(), "app.public.user_email");

        let reference: ObjectRefRequest = serde_json::from_value(serde_json::json!({
            "kind": "routine",
            "database": "app",
            "schema": "public",
            "name": "user_email",
            "routineKind": "function",
            "arguments": "user_id bigint"
        }))
        .expect("routine reference");
        assert_eq!(
            reference.validate().expect("valid reference").routine_kind,
            Some(RoutineKind::Function)
        );
    }

    #[test]
    fn view_queries_are_single_select_statements() {
        let request = ObjectActionRequest::CreateView {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "active_users".to_string(),
            query: "SELECT id, email FROM users WHERE active".to_string(),
            or_replace: false,
        };
        let validated = request.validate().expect("valid view");
        assert_eq!(validated.kind(), ObjectActionKind::CreateView);
        assert_eq!(validated.object().label(), "app.public.active_users");

        let multiple = ObjectActionRequest::CreateView {
            database: None,
            schema: None,
            name: "bad".to_string(),
            query: "SELECT 1; DROP TABLE users".to_string(),
            or_replace: false,
        };
        assert!(multiple.validate().is_err());

        let ddl = ObjectActionRequest::CreateView {
            database: None,
            schema: None,
            name: "bad".to_string(),
            query: "DROP TABLE users".to_string(),
            or_replace: false,
        };
        assert!(ddl.validate().is_err());
    }

    #[test]
    fn definitions_are_bounded_and_reject_nul() {
        let too_long = "x".repeat(MAX_OBJECT_BODY_LENGTH + 1);
        let request = ObjectActionRequest::CreateRoutine {
            database: None,
            schema: None,
            name: "routine".to_string(),
            routine_kind: RoutineKind::Procedure,
            arguments: None,
            return_type: None,
            language: None,
            body: too_long,
            or_replace: false,
        };
        assert!(request.validate().is_err());

        let nul = ObjectActionRequest::CreateRoutine {
            database: None,
            schema: None,
            name: "routine".to_string(),
            routine_kind: RoutineKind::Procedure,
            arguments: None,
            return_type: None,
            language: None,
            body: "BEGIN\0END".to_string(),
            or_replace: false,
        };
        assert!(nul.validate().is_err());
    }

    #[test]
    fn drop_and_replace_require_confirmation() {
        let drop = ObjectActionRequest::DropView {
            database: None,
            schema: None,
            name: "active_users".to_string(),
            materialized: false,
        }
        .validate()
        .expect("valid drop");
        assert!(drop.requires_confirmation());
        assert!(drop.destructive());

        let replace = ObjectActionRequest::CreateView {
            database: None,
            schema: None,
            name: "active_users".to_string(),
            query: "SELECT 1".to_string(),
            or_replace: true,
        }
        .validate()
        .expect("valid replace");
        assert_eq!(replace.kind(), ObjectActionKind::ReplaceView);
        assert!(replace.requires_confirmation());
        assert!(!replace.destructive());
    }

    #[test]
    fn sequence_bounds_are_validated() {
        let request = ObjectActionRequest::CreateSequence {
            database: Some("app".to_string()),
            schema: Some("public".to_string()),
            name: "orders_id_seq".to_string(),
            data_type: Some(SequenceDataType::BigInt),
            start: Some(1),
            increment: Some(0),
            minimum: None,
            maximum: None,
            cache: Some(1),
            cycle: false,
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn trigger_events_are_typed_and_deduplicated() {
        let request = ObjectActionRequest::CreateTrigger {
            database: None,
            schema: Some("public".to_string()),
            name: "users_audit".to_string(),
            table: "users".to_string(),
            timing: TriggerTiming::After,
            events: vec![TriggerEvent::Insert, TriggerEvent::Insert],
            body: TriggerBodyRequest::Statement {
                body: "INSERT INTO audit VALUES (NEW.id)".to_string(),
            },
            or_replace: false,
        };
        assert!(request.validate().is_err());
    }
}
