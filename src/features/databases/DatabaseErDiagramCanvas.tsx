import { memo, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  Panel,
  Position,
  ReactFlow,
  ReactFlowProvider,
  getNodesBounds,
  getViewportForBounds,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import dagre from "@dagrejs/dagre";
import "@xyflow/react/dist/style.css";
import { CircleNotchIcon, DownloadSimpleIcon, FrameCornersIcon } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  ER_DIAGRAM_MAX_RELATIONSHIPS,
  erDiagramCardinalityDescription,
  erDiagramCardinalityLabel,
  type ErDiagramGraph,
  type ErDiagramRelationship,
  type ErDiagramTable,
} from "@/features/databases/databaseErDiagramModel";

const NODE_WIDTH = 230;
const NODE_HEIGHT = 98;
const NODE_GAP = 32;
const RANK_GAP = 88;

type ErTableNodeData = {
  table: ErDiagramTable;
};

type ErFlowNode = Node<ErTableNodeData, "erTable">;
type ErFlowEdge = Edge<{ relationship: ErDiagramRelationship }, "smoothstep">;

function relationshipLabel(relationship: ErDiagramRelationship): string {
  return `${erDiagramCardinalityLabel(relationship.cardinality)} · ${relationship.cardinality === "one-to-one" ? "one-to-one" : "many-to-one"}`;
}

const ErTableNode = memo(function ErTableNode({ data, selected }: NodeProps<ErFlowNode>) {
  const { table } = data;
  return (
    <div
      className={cn(
        "w-[230px] overflow-hidden rounded-md border bg-card text-card-foreground shadow-sm transition-colors",
        selected
          ? "border-primary ring-2 ring-primary/30"
          : "border-border hover:border-input hover:shadow-md",
      )}
      data-testid={`er-table-${table.key}`}
    >
      <Handle
        type="target"
        position={Position.Top}
        isConnectable={false}
        className="!size-2 !border-card !bg-muted-foreground"
        aria-hidden="true"
      />
      <div className="flex items-start justify-between gap-2 border-b border-border px-3 py-2">
        <div className="min-w-0">
          <p className="truncate font-mono text-[12px] font-semibold tracking-[-0.01em]">
            {table.name}
          </p>
          <p className="truncate text-[10px] text-muted-foreground">
            {table.schema ? `${table.schema} schema` : (table.database ?? "database")}
          </p>
        </div>
        <span className="shrink-0 rounded-sm border border-border bg-muted/60 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
          {table.outgoing + table.incoming} rel
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 px-3 py-2 text-[10px] text-muted-foreground">
        <span>
          Out <span className="font-mono text-foreground">{table.outgoing}</span>
        </span>
        <span>
          In <span className="font-mono text-foreground">{table.incoming}</span>
        </span>
      </div>
      <div className="truncate border-t border-border/70 px-3 py-1.5 text-[9px] text-muted-foreground">
        {table.estimatedRows === null
          ? "Row estimate unavailable"
          : `~${table.estimatedRows.toLocaleString()} estimated rows`}
      </div>
      <Handle
        type="source"
        position={Position.Bottom}
        isConnectable={false}
        className="!size-2 !border-card !bg-primary"
        aria-hidden="true"
      />
    </div>
  );
});

const nodeTypes = { erTable: ErTableNode };

type LayoutResult = {
  nodes: ErFlowNode[];
  edges: ErFlowEdge[];
  signature: string;
};

function layoutErDiagram(graph: ErDiagramGraph): LayoutResult {
  const layout = new dagre.graphlib.Graph({ multigraph: true });
  layout.setGraph({
    rankdir: "TB",
    nodesep: NODE_GAP,
    ranksep: RANK_GAP,
    marginx: 16,
    marginy: 16,
    ranker: "network-simplex",
  });
  layout.setDefaultEdgeLabel(() => ({}));

  for (const table of graph.tables) {
    layout.setNode(table.key, { width: NODE_WIDTH, height: NODE_HEIGHT });
  }
  for (const relationship of graph.relationships) {
    layout.setEdge(relationship.sourceKey, relationship.targetKey, {}, relationship.id);
  }
  if (graph.relationships.length > 0) {
    dagre.layout(layout);
  }

  // Dagre puts every disconnected node in one long row. A compact grid is
  // easier to read when a schema has no foreign keys at all.
  const gridColumns = Math.max(1, Math.min(4, graph.tables.length));
  const nodes: ErFlowNode[] = graph.tables.map((table, index) => {
    const positioned =
      graph.relationships.length > 0
        ? (layout.node(table.key) as { x: number; y: number } | undefined)
        : {
            x: (index % gridColumns) * (NODE_WIDTH + NODE_GAP) + NODE_WIDTH / 2,
            y: Math.floor(index / gridColumns) * (NODE_HEIGHT + NODE_GAP) + NODE_HEIGHT / 2,
          };
    return {
      id: table.key,
      type: "erTable",
      position: {
        x: (positioned?.x ?? 0) - NODE_WIDTH / 2,
        y: (positioned?.y ?? 0) - NODE_HEIGHT / 2,
      },
      data: { table },
      selected: false,
      draggable: false,
      connectable: false,
      deletable: false,
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
      ariaLabel: `${table.qualifiedName} table, ${table.outgoing} outgoing and ${table.incoming} incoming relationships`,
    };
  });

  const edges: ErFlowEdge[] = graph.relationships
    .slice(0, ER_DIAGRAM_MAX_RELATIONSHIPS)
    .map((relationship) => ({
      id: relationship.id,
      source: relationship.sourceKey,
      target: relationship.targetKey,
      type: "smoothstep",
      label: relationshipLabel(relationship),
      labelStyle: {
        fill: "var(--muted-foreground)",
        fontFamily: "var(--font-mono)",
        fontSize: 9,
      },
      labelBgStyle: {
        fill: "var(--card)",
        stroke: "var(--border)",
        strokeWidth: 0.5,
      },
      labelBgPadding: [4, 2],
      labelBgBorderRadius: 3,
      style: { stroke: "var(--muted-foreground)", strokeWidth: 1.2 },
      markerEnd: {
        type: MarkerType.ArrowClosed,
        width: 14,
        height: 14,
        color: "var(--muted-foreground)",
      },
      ariaLabel: `${relationship.sourceTable} references ${relationship.targetTable}; ${erDiagramCardinalityDescription(relationship.cardinality)}`,
      data: { relationship },
      selectable: false,
      focusable: false,
    }));

  const signature = `${graph.tables
    .map((table) => table.key)
    .join("|")}::${graph.relationships.map((relationship) => relationship.id).join("|")}`;

  return { nodes, edges, signature };
}

function safeFilePart(value: string): string {
  return value.replace(/[^a-zA-Z0-9._-]+/g, "-").replace(/-+/g, "-");
}

type CanvasProps = {
  graph: ErDiagramGraph;
  selectedKey: string | null;
  database: string | null;
  schema: string | null;
  colorMode: "light" | "dark";
  onSelectTable: (table: ErDiagramTable) => void;
};

function DatabaseErDiagramCanvasInner({
  graph,
  selectedKey,
  database,
  schema,
  colorMode,
  onSelectTable,
}: CanvasProps) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const { fitView } = useReactFlow();
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  const [exported, setExported] = useState(false);
  const layout = useMemo(() => layoutErDiagram(graph), [graph]);
  const [nodes, setNodes, onNodesChange] = useNodesState<ErFlowNode>(layout.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<ErFlowEdge>(layout.edges);

  useEffect(() => {
    setNodes(layout.nodes);
    setEdges(layout.edges);
    const frame = window.requestAnimationFrame(() => {
      void fitView({ padding: 0.16, duration: 180, maxZoom: 1.1 });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [fitView, layout, setEdges, setNodes]);

  useEffect(() => {
    setNodes((current) =>
      current.map((node) => ({
        ...node,
        selected: node.id === selectedKey,
      })),
    );
  }, [selectedKey, setNodes]);

  async function handleExport() {
    const wrapper = wrapperRef.current;
    const viewport = wrapper?.querySelector<HTMLElement>(".react-flow__viewport");
    if (!wrapper || !viewport || nodes.length === 0) {
      setExportError("The diagram is not ready to export yet.");
      return;
    }

    setExporting(true);
    setExportError(null);
    setExported(false);
    try {
      const { toPng } = await import("html-to-image");
      const bounds = getNodesBounds(nodes);
      const imageWidth = Math.max(1_200, Math.min(3_200, Math.ceil(bounds.width + 160)));
      const imageHeight = Math.max(800, Math.min(2_400, Math.ceil(bounds.height + 160)));
      const viewportTransform = getViewportForBounds(bounds, imageWidth, imageHeight, 0.2, 2, 0.1);
      const background = getComputedStyle(wrapper).backgroundColor;
      const dataUrl = await toPng(viewport, {
        width: imageWidth,
        height: imageHeight,
        backgroundColor: background,
        pixelRatio: imageWidth * imageHeight > 4_000_000 ? 1 : 1.5,
        style: {
          width: `${imageWidth}px`,
          height: `${imageHeight}px`,
          transform: `translate(${viewportTransform.x}px, ${viewportTransform.y}px) scale(${viewportTransform.zoom})`,
        },
      });
      const anchor = document.createElement("a");
      const scope = [database ?? "database", schema]
        .filter((value): value is string => value !== null)
        .map(safeFilePart)
        .join("-");
      anchor.href = dataUrl;
      anchor.download = `er-diagram-${scope || "schema"}.png`;
      anchor.click();
      setExported(true);
    } catch {
      setExportError("The diagram image could not be generated in this environment.");
    } finally {
      setExporting(false);
    }
  }

  return (
    <div
      ref={wrapperRef}
      className="relative h-[32rem] min-h-[24rem] overflow-hidden rounded-md border border-border bg-background"
      style={{ colorScheme: colorMode }}
      role="region"
      aria-label="Entity relationship diagram canvas"
    >
      <ReactFlow<ErFlowNode, ErFlowEdge>
        key={layout.signature}
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={(_event, node) => onSelectTable(node.data.table)}
        nodesDraggable={false}
        nodesConnectable={false}
        edgesReconnectable={false}
        elementsSelectable
        minZoom={0.1}
        maxZoom={2}
        fitView
        fitViewOptions={{ padding: 0.16, maxZoom: 1.1 }}
        proOptions={{ hideAttribution: true }}
        aria-label="Entity relationship graph"
        className="[&_.react-flow__controls]:!border-border [&_.react-flow__controls]:!bg-card [&_.react-flow__controls-button]:!border-border [&_.react-flow__controls-button]:!bg-card [&_.react-flow__controls-button]:!fill-foreground"
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={18}
          size={1}
          color={colorMode === "dark" ? "#26332c" : "#cbd3cd"}
        />
        <Controls position="bottom-left" showInteractive={false} />
        <MiniMap
          position="bottom-right"
          pannable
          zoomable
          nodeStrokeWidth={2}
          nodeColor={colorMode === "dark" ? "#1b241f" : "#e7ebe7"}
          nodeStrokeColor={colorMode === "dark" ? "#435047" : "#a4b0a8"}
          maskColor={colorMode === "dark" ? "rgba(0,0,0,0.68)" : "rgba(242,244,242,0.66)"}
          style={{ width: 132, height: 92 }}
          ariaLabel="Entity relationship diagram minimap"
        />
        <Panel position="top-right" className="!m-2 flex flex-col items-end gap-1.5">
          <div className="flex items-center gap-1.5">
            <Button
              variant="secondary"
              size="sm"
              className="h-7 px-2 text-[11px]"
              onClick={() => void fitView({ padding: 0.16, duration: 180, maxZoom: 1.1 })}
              aria-label="Fit diagram to view"
            >
              <FrameCornersIcon className="size-3.5" aria-hidden="true" />
              Fit
            </Button>
            <Button
              variant="secondary"
              size="sm"
              className="h-7 px-2 text-[11px]"
              onClick={() => void handleExport()}
              disabled={exporting}
              aria-label="Export diagram as PNG"
            >
              {exporting ? (
                <CircleNotchIcon className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <DownloadSimpleIcon className="size-3.5" aria-hidden="true" />
              )}
              {exporting ? "Exporting" : exported ? "Exported" : "PNG"}
            </Button>
          </div>
          {exportError ? (
            <p
              role="alert"
              className="max-w-[16rem] rounded-sm border border-destructive/30 bg-card px-2 py-1 text-right text-[10px] text-destructive"
            >
              {exportError}
            </p>
          ) : null}
        </Panel>
      </ReactFlow>
      <p className="sr-only">
        Drag the background to pan, use the mouse wheel or zoom controls to zoom, and select a table
        card to open its structure. A complete keyboard-accessible table and relationship index
        follows the diagram.
      </p>
    </div>
  );
}

export default function DatabaseErDiagramCanvas(props: CanvasProps) {
  return (
    <ReactFlowProvider>
      <DatabaseErDiagramCanvasInner {...props} />
    </ReactFlowProvider>
  );
}
