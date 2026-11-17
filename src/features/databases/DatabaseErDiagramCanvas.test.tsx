import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import DatabaseErDiagramCanvas from "@/features/databases/DatabaseErDiagramCanvas";
import { buildErDiagram, erDiagramTableKey } from "@/features/databases/databaseErDiagramModel";
import type {
  DatabaseConstraintSummary,
  DatabaseMetadataValue,
  DatabaseTableSummary,
} from "@/types/contracts";

const unavailable = { status: "unavailable" } as const;
const available = <T,>(value: T): DatabaseMetadataValue<T> => ({ status: "available", value });

function table(name: string): DatabaseTableSummary {
  return {
    database: "app",
    schema: "public",
    name,
    kind: "table",
    estimatedRows: available(10),
    metadataRefreshedAt: unavailable,
    owner: unavailable,
    encoding: unavailable,
    collation: unavailable,
    sizeBytes: unavailable,
    engine: unavailable,
  };
}

const users = table("users");
const orders = table("orders");
const foreignKey: DatabaseConstraintSummary = {
  name: "orders_user_fkey",
  kind: "foreign_key",
  columns: ["user_id"],
  referencedSchema: available("public"),
  referencedTable: available("users"),
  referencedColumns: ["id"],
  onUpdate: available("CASCADE"),
  onDelete: available("CASCADE"),
  checkExpression: unavailable,
  deferrable: unavailable,
  initiallyDeferred: unavailable,
};

function graphWith(constraints: Record<string, DatabaseConstraintSummary[]>) {
  return buildErDiagram({
    tables: [users, orders],
    constraintsByTable: constraints,
    limit: 50,
  });
}

describe("DatabaseErDiagramCanvas", () => {
  it("renders table cards with relationship counts", async () => {
    const graph = graphWith({
      [erDiagramTableKey("app", "public", "orders")]: [foreignKey],
    });
    render(
      <DatabaseErDiagramCanvas
        graph={graph}
        selectedKey={null}
        database="app"
        schema="public"
        colorMode="dark"
        onSelectTable={vi.fn()}
      />,
    );
    expect(await screen.findByText("orders")).toBeInTheDocument();
    expect(screen.getByText("users")).toBeInTheDocument();
    expect(
      screen.getByLabelText("public.orders table, 1 outgoing and 0 incoming relationships"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Entity relationship diagram canvas" }),
    ).toBeInTheDocument();
  });

  it("still mounts a readable grid when the schema has no foreign keys", async () => {
    render(
      <DatabaseErDiagramCanvas
        graph={graphWith({})}
        selectedKey={null}
        database="app"
        schema="public"
        colorMode="light"
        onSelectTable={vi.fn()}
      />,
    );
    expect(await screen.findByText("users")).toBeInTheDocument();
    expect(screen.getByText("orders")).toBeInTheDocument();
    expect(screen.getAllByText("0 rel")).toHaveLength(2);
  });
});
