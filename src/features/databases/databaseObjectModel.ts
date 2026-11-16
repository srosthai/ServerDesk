import type {
  CapabilityFlags,
  DatabaseAction,
  DatabaseEngine,
  DatabaseObjectActionKind,
  DatabaseObjectActionRequest,
  DatabaseObjectDefinition,
  DatabaseObjectKind,
  DatabaseObjectRef,
  DatabaseRoutineKind,
  DatabaseSequenceDataType,
  DatabaseTriggerEvent,
  DatabaseTriggerTiming,
} from "@/types/contracts";
import type { DatabaseExplorerSelection } from "@/features/databases/databaseExplorerTreeModel";
import { engineLabel, type StructureDecision } from "@/features/databases/databaseStructureModel";

export type ObjectCapabilities = {
  createView: StructureDecision;
  replaceView: StructureDecision;
  dropView: StructureDecision;
  createRoutine: StructureDecision;
  editRoutine: StructureDecision;
  dropRoutine: StructureDecision;
  createTrigger: StructureDecision;
  editTrigger: StructureDecision;
  dropTrigger: StructureDecision;
  createSequence: StructureDecision;
  alterSequence: StructureDecision;
  dropSequence: StructureDecision;
  refreshMaterializedView: StructureDecision;
  dropMaterializedView: StructureDecision;
};

function enabled(): StructureDecision {
  return { enabled: true, reason: null };
}

function disabled(reason: string): StructureDecision {
  return { enabled: false, reason };
}

function unsupportedReason(
  label: string,
  engine: DatabaseEngine,
  version: string | null | undefined,
): string {
  return `${label} is not supported by ${engineLabel(engine)} ${version ?? "on this server"}.`;
}

/** Derives object action availability from the connection capability document. */
export function objectCapabilitiesFor(input: {
  engine: DatabaseEngine;
  version?: string | null;
  connected: boolean;
  readOnly: boolean;
  actions?: DatabaseAction[];
  flags?: CapabilityFlags;
}): ObjectCapabilities {
  const { actions, flags } = input;
  const actionSet = actions ? new Set(actions) : null;
  const globalReason = !input.connected
    ? "Connect this database profile before changing objects."
    : input.readOnly
      ? "This database profile is read-only. Save a writable profile before changing objects."
      : null;

  function decide(
    action: DatabaseAction | null,
    label: string,
    flag?: keyof CapabilityFlags,
  ): StructureDecision {
    if (globalReason) {
      return disabled(globalReason);
    }
    if (actionSet && action && !actionSet.has(action)) {
      return disabled(unsupportedReason(label, input.engine, input.version));
    }
    if (!actionSet && flag) {
      if (!flags) {
        return disabled(
          "Capability details are unavailable for this connection. Reconnect to refresh them before using this action.",
        );
      }
      if (!flags[flag]) {
        return disabled(unsupportedReason(label, input.engine, input.version));
      }
    }
    return enabled();
  }

  return {
    createView: decide("create_view", "View creation", "viewManagement"),
    replaceView: decide("replace_view", "View replacement", "viewManagement"),
    dropView: decide("drop_view", "View drop", "viewManagement"),
    createRoutine: decide("create_routine", "Routine creation", "routines"),
    editRoutine: decide("edit_routine", "Routine editing", "routineEdit"),
    dropRoutine: decide("drop_routine", "Routine drop", "routines"),
    createTrigger: decide("create_trigger", "Trigger creation", "triggers"),
    editTrigger: decide("edit_trigger", "Trigger editing", "triggerEdit"),
    dropTrigger: decide("drop_trigger", "Trigger drop", "triggers"),
    createSequence: decide("create_sequence", "Sequence creation", "sequences"),
    alterSequence: decide("alter_sequence", "Sequence changes", "sequences"),
    dropSequence: decide("drop_sequence", "Sequence drop", "sequences"),
    refreshMaterializedView: decide(
      "refresh_materialized_view",
      "Materialized view refresh",
      "materializedViews",
    ),
    dropMaterializedView: decide(
      "drop_materialized_view",
      "Materialized view drop",
      "materializedViews",
    ),
  };
}

export function objectRefForSelection(
  selection: DatabaseExplorerSelection | null,
): DatabaseObjectRef | null {
  if (!selection?.objectName || !selection.objectKind) {
    return null;
  }
  return {
    kind: selection.objectKind,
    database: selection.database,
    schema: selection.schema,
    name: selection.objectName,
    table: selection.objectTable ?? null,
    arguments: selection.objectArguments ?? null,
  };
}

export function objectKindLabel(kind: DatabaseObjectKind): string {
  switch (kind) {
    case "materialized_view":
      return "Materialized view";
    case "routine":
      return "Routine";
    case "trigger":
      return "Trigger";
    case "sequence":
      return "Sequence";
    default:
      return "View";
  }
}

export function routineKindLabel(kind: DatabaseRoutineKind): string {
  return kind === "procedure" ? "Procedure" : "Function";
}

export function triggerTimingLabel(timing: DatabaseTriggerTiming): string {
  switch (timing) {
    case "before":
      return "Before";
    case "instead_of":
      return "Instead of";
    default:
      return "After";
  }
}

export function triggerEventLabel(event: DatabaseTriggerEvent): string {
  return event === "insert"
    ? "Insert"
    : event === "update"
      ? "Update"
      : event === "delete"
        ? "Delete"
        : "Truncate";
}

export function sequenceDataTypeLabel(dataType: DatabaseSequenceDataType): string {
  switch (dataType) {
    case "small_int":
      return "smallint";
    case "integer":
      return "integer";
    default:
      return "bigint";
  }
}

export function objectDefinitionTitle(definition: DatabaseObjectDefinition): string {
  if (definition.kind === "routine" && definition.routineKind.status === "available") {
    return `${routineKindLabel(definition.routineKind.value)} ${definition.name}`;
  }
  if (definition.kind === "view" || definition.kind === "materialized_view") {
    return `${objectKindLabel(definition.kind)} ${definition.name}`;
  }
  return `${objectKindLabel(definition.kind)} ${definition.name}`;
}

export function objectDefinitionLines(definition: DatabaseObjectDefinition): Array<{
  label: string;
  value: string;
  mono?: boolean;
}> {
  const rows: Array<{ label: string; value: string; mono?: boolean }> = [
    { label: "Name", value: definition.name, mono: true },
    { label: "Kind", value: objectKindLabel(definition.kind) },
    {
      label: "Database",
      value: definition.database ?? "Active database",
      mono: true,
    },
    { label: "Schema", value: definition.schema ?? "Default schema", mono: true },
    {
      label: "Owner",
      value: definition.owner.status === "available" ? definition.owner.value : "Unavailable",
      mono: true,
    },
  ];

  if (definition.kind === "routine") {
    rows.push(
      {
        label: "Routine",
        value:
          definition.routineKind.status === "available"
            ? routineKindLabel(definition.routineKind.value)
            : "Unavailable",
      },
      {
        label: "Language",
        value:
          definition.language.status === "available" ? definition.language.value : "Unavailable",
      },
      {
        label: "Arguments",
        value:
          definition.arguments.status === "available"
            ? definition.arguments.value || "None"
            : "Unavailable",
        mono: true,
      },
      {
        label: "Returns",
        value:
          definition.returnType.status === "available"
            ? definition.returnType.value
            : "Unavailable",
        mono: true,
      },
    );
  }

  if (definition.trigger) {
    rows.push(
      { label: "Target table", value: definition.trigger.table, mono: true },
      { label: "Timing", value: triggerTimingLabel(definition.trigger.timing) },
      {
        label: "Events",
        value: definition.trigger.events.map(triggerEventLabel).join(", ") || "Unavailable",
      },
      {
        label: "Function",
        value:
          definition.trigger.functionName.status === "available"
            ? definition.trigger.functionName.value
            : definition.trigger.function.status === "available"
              ? definition.trigger.function.value
              : "Unavailable",
        mono: true,
      },
    );
  }

  if (definition.sequence) {
    const sequence = definition.sequence;
    rows.push(
      {
        label: "Data type",
        value: sequence.dataType.status === "available" ? sequence.dataType.value : "Unavailable",
        mono: true,
      },
      {
        label: "Current value",
        value:
          sequence.currentValue.status === "available"
            ? sequence.currentValue.value
            : "Unavailable",
        mono: true,
      },
      {
        label: "Start",
        value:
          sequence.startValue.status === "available" ? sequence.startValue.value : "Unavailable",
        mono: true,
      },
      {
        label: "Increment",
        value: sequence.increment.status === "available" ? sequence.increment.value : "Unavailable",
        mono: true,
      },
      {
        label: "Minimum",
        value: sequence.minimum.status === "available" ? sequence.minimum.value : "Unavailable",
        mono: true,
      },
      {
        label: "Maximum",
        value: sequence.maximum.status === "available" ? sequence.maximum.value : "Unavailable",
        mono: true,
      },
      {
        label: "Cache",
        value: sequence.cache.status === "available" ? sequence.cache.value : "Unavailable",
        mono: true,
      },
      {
        label: "Cycle",
        value:
          sequence.cycle.status === "available"
            ? sequence.cycle.value
              ? "Yes"
              : "No"
            : "Unavailable",
      },
    );
  }

  return rows;
}

export function dropActionForDefinition(
  definition: DatabaseObjectDefinition,
): DatabaseObjectActionRequest | null {
  const scope = { database: definition.database, schema: definition.schema };
  switch (definition.kind) {
    case "view":
      return { kind: "drop_view", name: definition.name, materialized: false, ...scope };
    case "materialized_view":
      return { kind: "drop_materialized_view", name: definition.name, ...scope };
    case "routine":
      return definition.routineKind.status === "available"
        ? {
            kind: "drop_routine",
            name: definition.name,
            routineKind: definition.routineKind.value,
            arguments:
              definition.arguments.status === "available" ? definition.arguments.value : null,
            ...scope,
          }
        : null;
    case "trigger":
      return definition.trigger
        ? { kind: "drop_trigger", name: definition.name, table: definition.trigger.table, ...scope }
        : null;
    case "sequence":
      return { kind: "drop_sequence", name: definition.name, ...scope };
  }
}

export function refreshActionForDefinition(
  definition: DatabaseObjectDefinition,
): DatabaseObjectActionRequest | null {
  if (definition.kind !== "materialized_view") {
    return null;
  }
  return {
    kind: "refresh_materialized_view",
    name: definition.name,
    database: definition.database,
    schema: definition.schema,
  };
}

export function replaceViewAction(
  definition: DatabaseObjectDefinition,
): DatabaseObjectActionRequest | null {
  if (definition.kind !== "view" || definition.viewQuery.status !== "available") {
    return null;
  }
  return {
    kind: "create_view",
    name: definition.name,
    query: definition.viewQuery.value,
    orReplace: true,
    database: definition.database,
    schema: definition.schema,
  };
}

export function editRoutineAction(
  definition: DatabaseObjectDefinition,
): DatabaseObjectActionRequest | null {
  if (definition.kind !== "routine" || definition.routineKind.status !== "available") {
    return null;
  }
  const body = definition.routineBody;
  return {
    kind: "create_routine",
    name: definition.name,
    routineKind: definition.routineKind.value,
    arguments: definition.arguments.status === "available" ? definition.arguments.value : null,
    returnType: definition.returnType.status === "available" ? definition.returnType.value : null,
    language: definition.language.status === "available" ? definition.language.value : null,
    body: body.status === "available" ? body.value : "",
    orReplace: true,
    database: definition.database,
    schema: definition.schema,
  };
}

export function objectActionKindLabel(kind: DatabaseObjectActionKind): string {
  switch (kind) {
    case "replace_view":
      return "Replace view";
    case "drop_view":
      return "Drop view";
    case "create_routine":
      return "Create routine";
    case "replace_routine":
      return "Replace routine";
    case "drop_routine":
      return "Drop routine";
    case "create_trigger":
      return "Create trigger";
    case "replace_trigger":
      return "Replace trigger";
    case "drop_trigger":
      return "Drop trigger";
    case "create_sequence":
      return "Create sequence";
    case "alter_sequence":
      return "Alter sequence";
    case "drop_sequence":
      return "Drop sequence";
    case "refresh_materialized_view":
      return "Refresh materialized view";
    case "drop_materialized_view":
      return "Drop materialized view";
    default:
      return "Create view";
  }
}
