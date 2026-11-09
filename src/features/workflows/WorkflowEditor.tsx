import { AutoField } from "../../shared/forms/auto-form";
import { EditorFormActionsContext } from "../../shared/forms/auto-form/form-actions";
import { EmptyWorkflowCanvas } from "../../shared/editor/EmptyWorkflowCanvas";
import { useQueryValue } from "../../shared/routing/url-state";
import { FieldCaption } from "../../shared/forms/FieldHelp";
import {
  Background,
  Controls,
  Handle,
  Position,
  ReactFlow,
  useUpdateNodeInternals,
  type Connection,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ContextMenu, Dialog } from "radix-ui";
import {
  GitBranch,
  Eye,
  History,
  LayoutGrid,
  Maximize2,
  Play,
  Plus,
  Redo2,
  Save,
  Trash2,
  Undo2,
  Workflow,
  X,
  Zap,
} from "lucide-react";
import {
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { api, errorText } from "../../shared/api/api";
import { InsertEdge } from "../../shared/editor/InsertEdge";
import { FitCanvas } from "../../shared/editor/FitCanvas";
import {
  commonText,
  useCommonTranslation,
  workflowText as t,
} from "../../shared/i18n/i18n";
import type { ListConfig } from "../../shared/resources/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import {
  arrangeGraph,
  addAdjacentNode,
  connectionDirection,
  editGraphNode,
  canConnect,
  insertNode,
  insertionPorts,
  newNode,
  parseGraph,
  ports,
  type Endpoint,
  type Graph,
  type GraphNode,
} from "./graph";
import { nodeTitle, outputPortLabel, WorkflowNodePicker } from "./WorkflowNodePicker";
import { WorkflowNodeInspector } from "./WorkflowNodeInspector";
import { WorkflowExecutionTarget } from "./WorkflowExecutionTarget";
import { WorkflowExecutionView, executionStateText } from "./WorkflowExecutionView";
import { integrationManifest } from "./integrations";
import { nodeModes } from "./workflow-data";
import { WorkflowAssetContext, WorkflowOutputPreview } from "./WorkflowAssets";
import {
  validRunInputs,
  WorkflowInputFields,
  WorkflowNodeParameters,
  WorkflowRunInputs,
} from "./WorkflowNodeParameters";

export interface WorkflowRow extends Record<string, unknown> {
  id: string;
  name: string;
  definition: string;
  version: number;
  draft_revision_id: string;
  published_revision_id: string | null;
  trigger_run_as: string | null;
  lifecycle: string;
}
const edgeTypes = { insert: InsertEdge };
const control =
  "w-full rounded-md border border-input bg-background px-3 py-2 text-sm";
type CardNode = Node<
  {
    node: GraphNode;
    state?: string;
    bottomPorts: string[];
    topPorts: string[];
    addAdjacent?: (direction: "right" | "down", output?: { port: string; index?: number }) => void;
  },
  "workflow"
>;
function NodeCard({ id, data, selected }: NodeProps<CardNode>) {
  const updateInternals = useUpdateNodeInternals();
  const inputs = ports(data.node, "inputs"),
    outputs = ports(data.node, "outputs");
  const handleLayout = JSON.stringify([data.bottomPorts, data.topPorts, inputs, outputs]);
  useEffect(() => {
    updateInternals(id);
  }, [id, handleLayout, updateInternals]);
  const Icon = data.node.type.startsWith("core.trigger.")
    ? Zap
    : data.node.type === "core.if" || data.node.type === "core.switch"
      ? GitBranch
      : Workflow;
  return (
    <div
      className={`relative w-56 rounded-xl border-2 bg-card p-4 shadow-sm ${data.state === "failed" ? "border-destructive" : data.state === "running" ? "border-primary ring-4 ring-primary/10" : data.state === "succeeded" ? "border-green-600" : selected ? "border-primary ring-4 ring-primary/10" : "border-border"}`}
      style={{
        minHeight: Math.max(
          104,
          Math.max(inputs.length, outputs.length) * 30 + 40,
        ),
      }}
    >
      <div className="flex items-start gap-3">
        <span className="rounded-lg bg-muted p-2">
          <Icon className="size-5 text-primary" />
        </span>
        <div className="min-w-0">
          <strong className="block break-words text-sm">
            {data.node.name}
          </strong>
          <span className="text-xs text-muted-foreground">
            {nodeTitle(data.node.type)}
          </span>
        </div>
      </div>
      {data.state && (
        <span className={`mt-3 block text-xs ${data.state === "failed" ? "text-destructive" : data.state === "running" ? "text-primary" : "text-muted-foreground"}`}>
          {executionStateText(data.state)}
        </span>
      )}
      {data.node.settings?.mode && data.node.settings.mode !== "active" ? <span className="mt-2 block text-xs text-muted-foreground">{t(`inspector.mode.${String(data.node.settings.mode)}` as Parameters<typeof t>[0])}</span> : null}
      {inputs.map((port, index) => (
        <Handle
          key={`${port.port}:${port.index ?? 0}`}
          type="target"
          position={Position.Left}
          id={`${port.port}:${port.index ?? 0}`}
          style={{ top: `${((index + 1) * 100) / (inputs.length + 1)}%` }}
          title={`${port.port} ${port.index ?? 0}`}
        />
      ))}
      {outputs.length === 1 && outputs.map((port, index) => (
        <Handle
          key={`${port.port}:${port.index ?? 0}`}
          type="source"
          position={Position.Right}
          id={`${port.port}:${port.index ?? 0}`}
          style={{
            top: `${((index + 1) * 100) / (outputs.length + 1)}%`,
            right: -36,
          }}
          title={outputPortLabel(data.node, port)}
        />
      ))}
      {outputs.length === 1 && outputs.map((port, index) => (
        <span
          key={`stem-${port.port}:${port.index ?? 0}`}
          aria-hidden
          className="pointer-events-none absolute -right-8 w-8 border-t border-muted-foreground/50"
          style={{ top: `${((index + 1) * 100) / (outputs.length + 1)}%` }}
        />
      ))}
      {outputs.length > 1 && (
        <div className="mt-3 border-t pt-1 text-xs">
          {outputs.map((port) => (
            <div key={`${port.port}:${port.index ?? 0}`} className="relative flex min-h-11 min-w-0 items-center justify-end border-b border-border/50 last:border-0">
              <span className="max-w-full truncate pr-2 font-medium" title={outputPortLabel(data.node, port)}>{outputPortLabel(data.node, port)}</span>
              <Handle type="source" position={Position.Right} id={`${port.port}:${port.index ?? 0}`} style={{right:-52,top:"50%"}} title={outputPortLabel(data.node, port)} />
              <span aria-hidden className="pointer-events-none absolute -right-12 top-1/2 w-8 border-t border-muted-foreground/50" />
              {data.addAdjacent && <button type="button" className="nodrag nopan absolute -right-[34px] top-1/2 z-10 grid size-8 -translate-y-1/2 place-items-center rounded-full border bg-background text-primary shadow-sm hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring" aria-label={`${t("addRight")} · ${outputPortLabel(data.node, port)}`} title={`${t("addRight")} · ${outputPortLabel(data.node, port)}`} onClick={event=>{event.stopPropagation();data.addAdjacent?.("right",port);}}><Plus className="size-4" /></button>}
            </div>
          ))}
        </div>
      )}
      {data.bottomPorts.map((key, index) => (
        <Handle
          key={`${key}:bottom`}
          type="source"
          position={Position.Bottom}
          id={`${key}:bottom`}
          className="!opacity-0 !pointer-events-none"
          style={{
            left: `${((index + 1) * 100) / (data.bottomPorts.length + 1)}%`,
          }}
        />
      ))}
      {data.topPorts.map((key, index) => (
        <Handle
          key={`${key}:top`}
          type="target"
          position={Position.Top}
          id={`${key}:top`}
          className="!opacity-0 !pointer-events-none"
          style={{
            left: `${((index + 1) * 100) / (data.topPorts.length + 1)}%`,
          }}
        />
      ))}
      {data.addAdjacent &&
        ([...(outputs.length <= 1 ? [{ direction: "right" as const, output: outputs[0] }] : []), { direction: "down" as const, output: undefined }]).map(({direction, output}) => (
          <button
            key={`${direction}:${output?.port ?? ""}:${output?.index ?? 0}`}
            type="button"
            className={`nodrag nopan absolute z-10 grid size-8 place-items-center rounded-full border bg-background text-primary shadow-sm hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring ${direction === "right" ? "right-0 top-1/2 -translate-y-1/2 translate-x-1/2" : "bottom-0 left-1/2 -translate-x-1/2 translate-y-1/2"}`}
            aria-label={`${t(direction === "right" ? "addRight" : "addDown")}${output && outputs.length > 1 ? ` · ${outputPortLabel(data.node, output)}` : ""}`}
            title={`${t(direction === "right" ? "addRight" : "addDown")}${output && outputs.length > 1 ? ` · ${outputPortLabel(data.node, output)}` : ""}`}
            onClick={(event) => {
              event.stopPropagation();
              data.addAdjacent?.(direction, output);
            }}
          >
            <Plus className="size-4" />
          </button>
        ))}
    </div>
  );
}
const nodeTypes = { workflow: NodeCard };

function Modal({
  title,
  children,
  footer,
  close,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  close: () => void;
  wide?: boolean;
}) {
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
        <Dialog.Content className={`fixed left-1/2 top-1/2 z-50 max-h-[90dvh] ${wide ? "w-[min(90rem,calc(100vw-2rem))]" : "w-[min(48rem,calc(100vw-2rem))]"} -translate-x-1/2 -translate-y-1/2 dialog-shell rounded-xl border bg-background p-4 sm:p-6 shadow-xl`}>
          <div className="mb-5 flex items-center justify-between">
            <Dialog.Title className="text-lg font-semibold">
              {title}
            </Dialog.Title>
            <Dialog.Close asChild>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={t("close")}
              >
                <X />
              </Button>
            </Dialog.Close>
          </div>
          <Dialog.Description className="sr-only">{title}</Dialog.Description>
          <div className="dialog-body">{children}</div>
          {footer && <footer className="dialog-footer mt-4">{footer}</footer>}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
export function WorkflowEditor({
  workspace,
  draft,
  readOnly: rowReadOnly,
  permissions,
  onPatch,
  onValidityChange,
  renderTable,
  templateMode = false,
}: {
  workspace: string;
  draft: Partial<WorkflowRow>;
  readOnly: boolean;
  permissions: Set<string>;
  templateMode?: boolean;
  onPatch: (value: Partial<WorkflowRow>) => void;
  onValidityChange: (invalid: boolean) => void;
  renderTable?: (
    config: ListConfig<Record<string, unknown>>,
    refreshKey?: number,
  ) => ReactNode;
}) {
  useCommonTranslation();
  const readOnly =
    rowReadOnly ||
    (!templateMode && !permissions.has(draft.id ? "workflows.update" : "workflows.create"));
  const parsed = useMemo(() => {
    try {
      return { graph: parseGraph(draft.definition ?? ""), error: "" };
    } catch (error) {
      return { graph: undefined, error: errorText(error) };
    }
  }, [draft.definition]);
  const graph = parsed.graph;
  const [legacyGitNodes] = useState(() => {
    const ids = new Set<string>();
    const collect = (graph: Graph | undefined) => {
      for (const node of graph?.nodes ?? []) {
        if (node.type.startsWith("integration.git.") && !node.params.connection_id && !node.params.auth_mode) ids.add(node.id);
        if (node.type === "core.loop") collect(node.params.body as Graph);
      }
    };
    if (draft.id) collect(graph);
    return ids;
  });
  const formActions = useContext(EditorFormActionsContext);
  const [selected, setSelected] = useQueryValue("node", "");
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [resultsOpen, setResultsOpen] = useState(false);
  const [executionId, setExecutionId] = useQueryValue("execution", "");
  const [selectedLink, setSelectedLink] = useState<number>();
  const [dragging, setDragging] = useState<{
    id: string;
    x: number;
    y: number;
  }>();
  const [measurements, setMeasurements] = useState<
    Record<string, { width: number; height: number }>
  >({});
  const [view, setTab] = useQueryValue("view", "canvas");
  const tab = view === "history" || view === "executions" ? view : "canvas";
  const [pickerOpen, setPickerOpen] = useState(false);
  const [insertAt, setInsertAt] = useState<number>();
  const [adjacent, setAdjacent] = useState<{
    from: Endpoint;
    direction: "right" | "down";
  }>();
  const [dialog, setDialog] = useState<
    "run" | "publish" | "delete" | "unpublish" | null
  >(null);
  const [executionSettingsOpen, setExecutionSettingsOpen] = useState(false);
  const [input, setInput] = useState<Record<string, unknown>>({});
  const [inputInvalid, setInputInvalid] = useState(false);
  const [entry, setEntry] = useState("");
  const [runRevision, setRunRevision] = useState<"draft" | "published">(
    "draft",
  );
  const [publishedGraph, setPublishedGraph] = useState<Graph>();
  const [versionName, setVersionName] = useState("");
  const [identityConfirmed, setIdentityConfirmed] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [execution, setExecution] = useState<Record<string, unknown>>();
  const [executionGraph, setExecutionGraph] = useState<Graph>();
  const openExecution = (row: Record<string, unknown>) => {
    if (execution?.id !== row.id) { setExecution(row); setAttempts([]); setExecutionGraph(undefined); }
    setExecutionId(String(row.id)); setResultsOpen(true);
  };
  useEffect(() => {
    if (!executionId) return;
    const controller = new AbortController();
    void api<Record<string, unknown>>(`/workspaces/${workspace}/workflow-executions/${executionId}`, {signal:controller.signal})
      .then(row => { if (!controller.signal.aborted) { setExecution(row); setResultsOpen(true); } })
      .catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); });
    return () => controller.abort();
  }, [workspace, executionId]);
  const [attempts, setAttempts] = useState<Record<string, unknown>[]>([]);
  const [outputPreview, setOutputPreview] =
    useState<Parameters<typeof WorkflowOutputPreview>[0]["output"]>();
  const [invalidFields, setInvalidFields] = useState<Record<string, boolean>>(
    {},
  );
  const past = useRef<string[]>([]),
    future = useRef<string[]>([]);
  const [historyVersion, setHistoryVersion] = useState(0);
  const endpoint = `/workspaces/${workspace}/workflows/${draft.id}`;
  const invalid = !!parsed.error || Object.values(invalidFields).some(Boolean);
  useEffect(() => {
    onValidityChange(invalid);
    return () => onValidityChange(false);
  }, [invalid, onValidityChange]);
  const invalidField = useCallback(
    (key: string, value: boolean) =>
      setInvalidFields((current) =>
        current[key] === value ? current : { ...current, [key]: value },
      ),
    [],
  );
  const change = (next: Graph) => {
    if (readOnly) return;
    past.current.push(draft.definition!);
    future.current = [];
    if (past.current.length > 100) past.current.shift();
    onPatch({ definition: JSON.stringify(next) });
    setError("");
    setHistoryVersion((version) => version + 1);
  };
  const stepHistory = (undo: boolean) => {
    const from = undo ? past.current : future.current,
      to = undo ? future.current : past.current;
    const value = from.pop();
    if (!value) return;
    to.push(draft.definition!);
    onPatch({ definition: value });
    setHistoryVersion((version) => version + 1);
  };
  void historyVersion;
  const node = graph?.nodes.find((node) => node.id === selected);
  const inputFields = useMemo(() => {
    if (!graph || !selected) return [];
    const seen = new Set<string>(),
      fields = new Set<string>();
    const visit = (id: string) => {
      if (seen.has(id)) return;
      seen.add(id);
      const node = graph.nodes.find((node) => node.id === id);
      if (!node) return;
      const schema = node.params.input_schema as
        { properties?: Record<string, unknown> } | undefined;
      for (const key of Object.keys(schema?.properties ?? {})) fields.add(key);
      if (node.type === "data.set")
        for (const field of node.params.fields as { path: string }[])
          fields.add(field.path);
      if (node.type === "data.rename")
        for (const field of node.params.mappings as { to: string }[])
          fields.add(field.to);
      for (const link of graph.connections.filter(
        (link) => link.to.node === id,
      ))
        visit(link.from.node);
    };
    for (const link of graph.connections.filter(
      (link) => link.to.node === selected,
    ))
      visit(link.from.node);
    return [...fields];
  }, [graph, selected]);
  const updateNode = (value: Partial<GraphNode>) => {
    if (graph && node) change(editGraphNode(graph, node.id, value));
  };
  const duplicateNode = () => {
    if (!graph || !node || readOnly) return;
    const fresh = newNode(graph, node.type, node.name);
    const copy = { ...structuredClone(node), id: fresh.id, position: fresh.position };
    change({ ...graph, nodes: [...graph.nodes, copy] });
    setSelected(copy.id);
  };
  const toggleNode = () => {
    if (!node || readOnly) return;
    const mode = node.settings?.mode === "bypass" || node.settings?.mode === "mute" ? "active" : nodeModes(node).includes("bypass") ? "bypass" : "mute";
    updateNode({settings:{...node.settings,mode}});
  };
  const add = (type: string) => {
    if (!graph) return;
    try {
      const created = newNode(graph, type, nodeTitle(type));
      change(
        adjacent
          ? addAdjacentNode(
              graph,
              adjacent.from,
              adjacent.direction,
              created,
              measurements,
            )
          : insertAt === undefined
            ? { ...graph, nodes: [...graph.nodes, created] }
            : insertNode(graph, insertAt, created, measurements),
      );
      setSelected(created.id);
      setInsertAt(undefined);
      setAdjacent(undefined);
      setPickerOpen(false);
    } catch (error) {
      setError(errorText(error));
    }
  };
  const fromHandle = (
    node: string,
    handle: string | null | undefined,
  ): Endpoint => {
    const [port = "main", index = "0"] = (handle ?? "").split(":");
    return { node, port, index: Number(index) };
  };
  const connect = (connection: Connection) => {
    if (!graph) return;
    const from = fromHandle(connection.source, connection.sourceHandle),
      to = fromHandle(connection.target, connection.targetHandle);
    if (!canConnect(graph, from, to)) {
      setError(t("invalidConnection"));
      return;
    }
    change({ ...graph, connections: [...graph.connections, { from, to }] });
  };
  const nodes: CardNode[] = (graph?.nodes ?? []).map((node) => {
    const vertical = graph!.connections.filter(
      (link) => connectionDirection(graph!, link) === "down",
    );
    const attempt =
      execution?.revision_id === draft.draft_revision_id
        ? attempts.find((attempt) => attempt.node_id === node.id)
        : undefined;
    return {
      id: node.id,
      type: "workflow",
      measured: measurements[node.id] ?? {},
      position:
        dragging?.id === node.id
          ? { x: dragging.x, y: dragging.y }
          : { x: node.position[0], y: node.position[1] },
      selected: selected === node.id,
      data: {
        node,
        bottomPorts: [
          ...new Set(
            vertical
              .filter((link) => link.from.node === node.id)
              .map((link) => `${link.from.port}:${link.from.index}`),
          ),
        ],
        topPorts: [
          ...new Set(
            vertical
              .filter((link) => link.to.node === node.id)
              .map((link) => `${link.to.port}:${link.to.index}`),
          ),
        ],
        ...(attempt ? { state: String(attempt.state) } : {}),
        ...(!readOnly
          ? {
              addAdjacent: (direction: "right" | "down", selectedOutput?: { port: string; index?: number }) => {
                const output = selectedOutput ?? ports(node, "outputs")[0];
                setAdjacent({
                  from: {
                    node: node.id,
                    port: output?.port ?? "",
                    index: output?.index ?? 0,
                  },
                  direction,
                });
                setInsertAt(undefined);
                setPickerOpen(true);
              },
            }
          : {}),
      },
    };
  });
  const edges = (graph?.connections ?? []).map((link, index) => ({
    id: `link-${index}`,
    source: link.from.node,
    target: link.to.node,
    sourceHandle: `${link.from.port}:${link.from.index}${connectionDirection(graph!, link) === "down" ? ":bottom" : ""}`,
    targetHandle: `${link.to.port}:${link.to.index}${connectionDirection(graph!, link) === "down" ? ":top" : ""}`,
    type: "insert",
    data: {
      ...(graph?.nodes.find(node => node.id === link.from.node && (node.type === "core.if" || node.type === "core.switch" || node.type === "host.preview")) ? {label: outputPortLabel(graph.nodes.find(node => node.id === link.from.node)!, link.from)} : {}),
      ...(!readOnly ? {
          insert: () => {
            setAdjacent(undefined);
            setInsertAt(index);
            setPickerOpen(true);
          },
        } : {}),
    },
  }));
  const runnable =
    permissions.has("workflows.run") &&
    !!draft.id &&
    draft.lifecycle !== "archived";
  const manualEntries = (runRevision === "published" ? publishedGraph : graph)?.nodes.filter(node => node.type === "core.trigger.manual") ?? [];
  const inputSchema = ((runRevision === "published"
    ? publishedGraph
    : graph
  )?.nodes.find((node) => node.id === entry)?.params.input_schema ?? {
    type: "object",
  }) as Record<string, unknown>;
  useEffect(() => {
    if (dialog !== "run") return;
    if (runRevision === "draft") {
      setEntry(
        graph?.nodes.find((node) => node.type === "core.trigger.manual")?.id ??
          "",
      );
      return;
    }
    if (!draft.published_revision_id) return;
    let live = true;
    const controller = new AbortController();
    setEntry("");
    setPublishedGraph(undefined);
    void api<{ definition: string }>(
      `${endpoint}/revisions/${draft.published_revision_id}`,
      { signal: controller.signal },
    )
      .then((revision) => {
        if (!live) return;
        const graph = parseGraph(revision.definition);
        setPublishedGraph(graph);
        setEntry(
          graph.nodes.find((node) => node.type === "core.trigger.manual")?.id ??
            "",
        );
      })
      .catch((error) => {
        if (live && !controller.signal.aborted) setError(errorText(error));
      });
    return () => {
      live = false;
      controller.abort();
    };
  }, [dialog, runRevision, draft.published_revision_id, endpoint, graph]);
  async function saved(requireClean = true) {
    const row = await api<WorkflowRow>(endpoint);
    if (
      row.version !== draft.version ||
      (requireClean &&
        ["definition", "name", "description", "group_id"].some(
          (key) => row[key] !== draft[key],
        ))
    )
      throw new Error(t("saveFirst"));
    return row;
  }
  async function perform() {
    setBusy(true);
    setError("");
    try {
      const saveBeforeRun = dialog === "run" && runRevision === "draft" && !readOnly && formActions?.saveAndWait;
      const row = saveBeforeRun ? await saveBeforeRun() as WorkflowRow | undefined : await saved(dialog !== "run" || runRevision === "draft");
      if (!row) throw new Error(t("saveBlocked"));
      if (dialog === "run") {
        const inputs = input;
        if (!inputs || Array.isArray(inputs) || typeof inputs !== "object")
          throw new Error(t("jsonError"));
        const run = await api<Record<string, unknown>>(
          `${endpoint}/executions`,
          {
            method: "POST",
            body: JSON.stringify({
              mode: "manual",
              revision: runRevision,
              version: row.version,
              entry_node_id: entry,
              inputs,
            }),
          },
        );
        setExecution(run);
        setExecutionId(String(run.id));
        setInspectorOpen(false);
        setResultsOpen(true);
      } else if (dialog === "publish") {
        await api(`${endpoint}/publications`, {
          method: "POST",
          body: JSON.stringify({
            version: row.version,
            revision_id: row.draft_revision_id,
            name: versionName,
            ...(identityConfirmed && row.trigger_run_as
              ? { previous_run_as: row.trigger_run_as }
              : {}),
          }),
        });
        onPatch(await api<WorkflowRow>(endpoint));
      } else if (dialog === "unpublish") {
        await api(`${endpoint}/unpublication`, {
          method: "POST",
          body: JSON.stringify({ version: row.version }),
        });
        onPatch(await api<WorkflowRow>(endpoint));
      }
      setRefresh((value) => value + 1);
      setDialog(null);
    } catch (error) {
      setError(errorText(error));
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    if (!execution?.id) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    let loadedRevision = false;
    const controller = new AbortController();
    async function load() {
      try {
        const base = `/workspaces/${workspace}/workflow-executions/${execution!.id}`;
        const [run, nodes] = await Promise.all([
          api<Record<string, unknown>>(base, { signal: controller.signal }),
          api<{ items: Record<string, unknown>[] }>(`${base}/node-runs`, {
            signal: controller.signal,
          }),
        ]);
        if (!loadedRevision) {
          const revision = await api<{ definition: string }>(
            `${endpoint}/revisions/${run.revision_id}`,
            { signal: controller.signal },
          );
          if (!live) return;
          setExecutionGraph(parseGraph(revision.definition));
          loadedRevision = true;
        }
        if (!live) return;
        setExecution(run);
        setAttempts(nodes.items);
        if (["queued", "running", "waiting", "cancelling"].includes(String(run.state)))
          timer = setTimeout(() => void load(), 1000);
        else setRefresh((value) => value + 1);
      } catch (error) {
        if (live && !controller.signal.aborted) setError(errorText(error));
      }
    }
    void load();
    return () => {
      live = false;
      clearTimeout(timer);
      controller.abort();
    };
  }, [execution?.id, workspace, endpoint]);
  const historyConfig: ListConfig<Record<string, unknown>> = {
    resource: "workflows",
    endpoint: `${endpoint}/revisions`,
    permissionResource: "workflows",
    queryPrefix: "revisions",
    rowKey: (row) => String(row.id),
    toolbar: { create: false },
    columns: [
      { key: "number", label: t("history.version"), render: row => `v${row.number}` },
      { key: "node_count", label: t("history.nodeCount") },
      { key: "status", label: t("state"), render: row => [row.is_draft ? t("draft") : "", row.is_published ? t("published") : ""].filter(Boolean).join(" · ") || "—" },
      { key: "message", label: commonText("common.description") },
      { key: "latest_execution", label: t("history.lastRun"), render: row => {
        const run = row.latest_execution as Record<string, unknown> | null;
        return run ? <Button type="button" variant="link" className={run.state === "failed" ? "text-destructive" : ""} onClick={() => openExecution(run)}>#{String(run.number)} · {executionStateText(run.state)}</Button> : t("noRuns");
      } },
      { key: "last_run_at", label: t("history.lastRunAt"), render: row => {
        const run = row.latest_execution as Record<string, unknown> | null;
        return run ? new Date(Number(run.created_at)).toLocaleString() : "—";
      } },
      {
        key: "created_at",
        label: t("createdAt"),
        render: (row) => new Date(Number(row.created_at)).toLocaleString(),
      },
    ],
  };
  const executionsConfig: ListConfig<Record<string, unknown>> = {
    resource: "workflows",
    endpoint: `${endpoint}/executions`,
    permissionResource: "workflows",
    queryPrefix: "executions",
    rowKey: (row) => String(row.id),
    toolbar: { create: false },
    empty: t("noRuns"),
    onRowClick: openExecution,
    filters: [{key:"state", label:t("state"), type:"select", options:["queued","running","waiting","succeeded","failed","cancelled"].map(value=>({value,label:executionStateText(value)}))}],
    columns: [
      { key: "number", label: "#", sortable:true },
      { key: "state", label: t("state"), render: row => <span className={row.state === "failed" ? "text-destructive" : ""}>{executionStateText(row.state)}</span> },
      { key: "revision_number", label: t("history.version"), render: row => row.revision_number ? `v${row.revision_number}` : "—" },
      { key: "trigger_kind", label: t("history.trigger"), render: row => row.trigger_kind === "manual" ? t("manual") : String(row.trigger_kind || "—") },
      { key: "triggered_by_name", label: t("history.actor") },
      { key: "created_at", label: t("createdAt"), sortable:true, render: row => new Date(Number(row.created_at)).toLocaleString() },
      { key: "started_at", label: t("execution.startedAt"), sortable:true, render: row => Number(row.started_at)>0 ? new Date(Number(row.started_at)).toLocaleString() : "—" },
      { key: "finished_at", label: t("execution.finishedAt"), sortable:true, render: row => Number(row.finished_at)>0 ? new Date(Number(row.finished_at)).toLocaleString() : "—" },
      { key: "duration", label: t("execution.duration"), render: row => Number(row.started_at)>0 ? `${Math.max(0,(Number(row.finished_at)||Date.now())-Number(row.started_at))} ms` : "—" },
      { key: "end_reason", label: t("history.endReason"), render: row => row.end_reason === "node_wait" ? t("execution.state.waiting") : String(row.end_reason ?? "") },
    ],
    actions: [
      {
        label: t("history.details"), icon:<Eye className="size-4" />, showLabel:true,
        action: async (row) => {
          openExecution(row);
        },
      },
    ],
  };
  const resultConfig: ListConfig<Record<string, unknown>> | undefined =
    execution
      ? {
          resource: "workflows",
          endpoint: `/workspaces/${workspace}/workflow-executions/${execution.id}/node-runs`,
          permissionResource: "workflows",
          queryPrefix: "nodes",
          rowKey: (row) => String(row.id),
          toolbar: { create: false },
          columns: [
            {
              key: "node_id",
              label: t("library"),
              render: (row) =>
                executionGraph?.nodes.find((node) => node.id === row.node_id)
                  ?.name ?? String(row.node_id),
            },
            { key: "state", label: t("state") },
            { key: "attempt", label: t("attempt") },
            { key: "items_in", label: t("itemsIn") },
            { key: "items_out", label: t("itemsOut") },
          ],
          actions: [
            {
              label: t("viewOutput"),
              action: async (row) => {
                try {
                  const output = await api<Parameters<typeof WorkflowOutputPreview>[0]["output"]>(
                      `/workspaces/${workspace}/workflow-executions/${execution.id}/node-runs/${row.id}/output`,
                  );
                  setOutputPreview({...output, state:String(row.state), ...(row.end_reason ? {end_reason:String(row.end_reason)} : {})});
                } catch (error) {
                  setError(errorText(error));
                }
              },
            },
          ],
        }
      : undefined;

  const executionResults = execution && resultConfig && (
              <section className="rounded-xl border p-4">
                <h3 role="status" className={`mb-4 text-sm font-semibold ${execution.state === "failed" ? "text-destructive" : ""}`}>
                  {t("results")} #{String(execution.number)} ·{" "}
                  {executionStateText(execution.state)}
                </h3>
                {permissions.has("workflows.run") && ["queued", "running", "waiting", "cancelling"].includes(String(execution.state)) && (
                  <div className="mb-4 flex justify-end">
                    <Button type="button" variant="outline" disabled={busy || execution.state === "cancelling"} onClick={async () => {
                      setBusy(true); setError("");
                      try {
                        const run = await api<Record<string, unknown>>(`/workspaces/${workspace}/workflow-executions/${execution.id}/cancellations`, { method: "POST" });
                        setExecution(run); setRefresh(value => value + 1);
                      } catch (error) { setError(errorText(error)); }
                      finally { setBusy(false); }
                    }}>{t(execution.state === "cancelling" ? "execution.cancelling" : "execution.cancel")}</Button>
                  </div>
                )}
                {execution.end_reason && execution.end_reason !== "node_wait" ? (
                  <p className="mb-3 text-sm text-destructive">
                    {execution.end_reason === "integration_unknown" ? t("integration.failure.unknown") : execution.end_reason === "integration_remote" ? t("integration.failure.remote") : execution.end_reason === "integration_invalid" ? t("integration.failure.invalid") : String(execution.end_reason)}
                  </p>
                ) : null}
                <WorkflowExecutionView workspace={workspace} execution={execution} graph={executionGraph} attempts={attempts} nodeTypes={nodeTypes} permissions={permissions} />
                <details className="mt-4"><summary className="cursor-pointer text-sm">{t("execution.records")}</summary>{renderTable?.(resultConfig, refresh)}</details>
              </section>
            );
  return (
    <WorkflowAssetContext.Provider
      value={{ workspace, workflow: draft.id ?? "", legacyGitNodes, canCreateRepositoryConnection:permissions.has("repositories.create") }}
    >
      <div
        className="flex min-h-0 min-w-0 flex-1 flex-col gap-3"
        aria-label={t("canvas")}
        onKeyDown={(event) => {
          const target = event.target as HTMLElement;
          if (inspectorOpen || dialog || pickerOpen || tab !== "canvas" || event.nativeEvent.isComposing ||
            target.closest("input, textarea, select, [contenteditable], [role='textbox'], [role='dialog'], [role='menu']")) return;
          if (node && event.key === "Enter") {event.preventDefault();setInspectorOpen(true);return;}
          if (readOnly) return;
          if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "d" && node) {event.preventDefault();duplicateNode();return;}
          if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {event.preventDefault();stepHistory(!event.shiftKey);return;}
          if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "y") {event.preventDefault();stepHistory(false);return;}
          if (node && event.key === " ") {event.preventDefault();toggleNode();return;}
          if (node && (event.key === "Delete" || event.key === "Backspace")) {
            event.preventDefault();
            event.stopPropagation();
            setDialog("delete");
          }
        }}
      >
        <div className="flex flex-wrap items-center gap-2">
          <span className="rounded-full border px-2.5 py-1 text-xs">{t("state")}:
            {draft.lifecycle === "archived"
              ? t("archived")
              : draft.published_revision_id
                ? draft.published_revision_id === draft.draft_revision_id
                  ? t("published")
                  : t("changes")
                : t("draft")}
          </span>
          {rowReadOnly && draft.id && <div
            className="flex flex-wrap gap-1"
            role="tablist"
            aria-label={t("workflows")}
          >
            {(["canvas", "history", "executions"] as const).map((key) => (
              <Button
                key={key}
                type="button"
                role="tab"
                aria-selected={tab === key}
                variant={tab === key ? "secondary" : "ghost"}
                size="sm"
                onClick={() => setTab(key)}
              >
                {key === "canvas" ? (
                  <Workflow />
                ) : key === "history" ? (
                  <History />
                ) : (
                  <Play />
                )}
                {t(key)}
              </Button>
            ))}
          </div>}
          <div className="ml-auto flex flex-wrap gap-2">
            {tab === "canvas" && <Button type="button" variant="outline" onClick={() => setExecutionSettingsOpen(true)}>{t("execution.defaults")}</Button>}
            {!readOnly && tab === "canvas" && (
              <>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setInsertAt(undefined);
                    setPickerOpen(true);
                  }}
                >
                  <Plus />
                  {t("add")}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  disabled={!graph?.nodes.length}
                  onClick={() => {
                    if (graph) change(arrangeGraph(graph, measurements));
                  }}
                >
                  <LayoutGrid />
                  {t("arrange")}
                </Button>
              </>
            )}
            {!readOnly && (
              <>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={t("undo")}
                  disabled={!past.current.length}
                  onClick={() => stepHistory(true)}
                >
                  <Undo2 />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={t("redo")}
                  disabled={!future.current.length}
                  onClick={() => stepHistory(false)}
                >
                  <Redo2 />
                </Button>
              </>
            )}
            {execution && <Button type="button" variant="outline" onClick={() => setResultsOpen(true)}>{t("results")} #{String(execution.number)}</Button>}
            {!readOnly && formActions && <Button type="button" variant="outline" disabled={busy || invalid || formActions.saving || formActions.blocked} onClick={formActions.save}><Save />{commonText("common.save")}</Button>}
            {!templateMode && <Button
              type="button"
              variant={readOnly ? "outline" : "default"}
              disabled={!runnable || invalid || busy || formActions?.saving || (!readOnly && formActions?.blocked)}
              onClick={() => {
                setRunRevision("draft");
                setEntry(
                  graph?.nodes.find(
                    (node) => node.type === "core.trigger.manual",
                  )?.id ?? "",
                );
                setDialog("run");
              }}
            >
              <Play />
              {t(!readOnly && formActions?.saveAndWait ? "saveAndRun" : "run")}
            </Button>}
            {!readOnly && runnable && (
              <Button
                type="button"
                variant="outline"
                className="ml-2"
                disabled={invalid}
                onClick={() => {
                  setVersionName("");
                  setIdentityConfirmed(false);
                  setDialog("publish");
                }}
              >
                {t("publish")}
              </Button>
            )}
            {!readOnly && draft.published_revision_id && (
              <Button
                type="button"
                variant="outline"
                onClick={() => setDialog("unpublish")}
              >
                {t("unpublish")}
              </Button>
            )}
          </div>
        </div>
        {(error || parsed.error) && (
          <p
            role="alert"
            className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive"
          >
            {error || parsed.error}
          </p>
        )}
        {tab === "canvas" && (
          <div
            className={`grid min-h-[560px] flex-1 overflow-hidden rounded-xl border bg-background lg:min-h-0 ${node ? "lg:grid-cols-[minmax(0,1fr)_360px]" : "grid-cols-1"}`}
          >
            <ContextMenu.Root>
            <ContextMenu.Trigger asChild disabled={readOnly}>
            <div className="relative min-h-[480px] min-w-0 bg-muted/10 lg:min-h-0" tabIndex={0}>
              <ReactFlow
                nodes={nodes}
                edges={edges}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                fitView
                fitViewOptions={{ padding: 0.25, maxZoom: 1 }}
                minZoom={0.2}
                nodesDraggable={!readOnly}
                nodesConnectable={!readOnly}
                deleteKeyCode={null}
                onNodesChange={(changes) => {
                  const dimensions = changes.flatMap((change) =>
                    change.type === "dimensions" && change.dimensions
                      ? [[change.id, change.dimensions] as const]
                      : [],
                  );
                  if (dimensions.length)
                    setMeasurements((current) => {
                      if (
                        dimensions.every(
                          ([id, size]) =>
                            current[id]?.width === size.width &&
                            current[id]?.height === size.height,
                        )
                      )
                        return current;
                      return { ...current, ...Object.fromEntries(dimensions) };
                    });
                }}
                onNodeClick={(_, node) => {
                  setSelected(node.id);
                  setSelectedLink(undefined);
                }}
                onNodeDoubleClick={(_, node) => {setSelected(node.id);setInspectorOpen(true);}}
                onNodeContextMenu={(_, node) => {
                  setSelected(node.id);
                  setSelectedLink(undefined);
                }}
                onEdgeClick={(_, edge) =>
                  setSelectedLink(Number(edge.id.slice(5)))
                }
                onPaneClick={() => {
                  setSelected("");
                  setSelectedLink(undefined);
                }}
                onPaneContextMenu={() => {
                  setSelected("");
                  setSelectedLink(undefined);
                }}
                onConnect={connect}
                onNodeDrag={(_, moved) =>
                  setDragging({ id: moved.id, ...moved.position })
                }
                onNodeDragStop={(_, moved) => {
                  if (graph && !readOnly)
                    change({
                      ...graph,
                      nodes: graph.nodes.map((node) =>
                        node.id === moved.id
                          ? {
                              ...node,
                              position: [moved.position.x, moved.position.y],
                            }
                          : node,
                      ),
                    });
                  setDragging(undefined);
                }}
              >
                <FitCanvas
                  topology={
                    JSON.stringify(nodes.map((node) => node.id)) +
                    JSON.stringify(
                      edges.map((edge) => [edge.source, edge.target]),
                    )
                  }
                />
                <Background gap={24} />
                <Controls showInteractive={false} />
              </ReactFlow>
              {selectedLink !== undefined && !readOnly && (
                <Button
                  type="button"
                  variant="outline"
                  className="absolute right-3 top-3"
                  onClick={() => {
                    if (graph)
                      change({
                        ...graph,
                        connections: graph.connections.filter(
                          (_, index) => index !== selectedLink,
                        ),
                      });
                    setSelectedLink(undefined);
                  }}
                >
                  <Trash2 />
                  {t("removeLink")}
                </Button>
              )}
              {!nodes.length && <EmptyWorkflowCanvas title={t("empty")} description={t("emptyHelp")} action={t("startWorkflow")} disabled={readOnly} onAdd={() => {setInsertAt(undefined);setPickerOpen(true);}} />}
            </div>
            </ContextMenu.Trigger>
            {node && !readOnly && <ContextMenu.Portal><ContextMenu.Content className="z-50 min-w-48 rounded-md border bg-popover p-1 text-popover-foreground shadow-md" collisionPadding={8}>
              <ContextMenu.Item className="cursor-default rounded px-3 py-2 text-sm outline-none focus:bg-accent" onSelect={() => {
                setInspectorOpen(true);
              }}>{t("parameters")}</ContextMenu.Item>
              <ContextMenu.Item className="flex cursor-default justify-between gap-4 rounded px-3 py-2 text-sm outline-none focus:bg-accent" onSelect={duplicateNode}>{t("duplicateNode")}<span className="text-xs">Ctrl / ⌘ D</span></ContextMenu.Item>
              <ContextMenu.Item className="flex cursor-default justify-between gap-4 rounded px-3 py-2 text-sm outline-none focus:bg-accent" onSelect={toggleNode}>{t(node.settings?.mode === "bypass" || node.settings?.mode === "mute" ? "inspector.enable" : nodeModes(node).includes("bypass") ? "inspector.bypass" : "inspector.disable")}<span className="text-xs">Space</span></ContextMenu.Item>
              {(["right", "down"] as const).map(direction => <ContextMenu.Item key={direction} className="cursor-default rounded px-3 py-2 text-sm outline-none focus:bg-accent" onSelect={() => nodes.find(card => card.id === node.id)?.data.addAdjacent?.(direction)}>{t(direction === "right" ? "addRight" : "addDown")}</ContextMenu.Item>)}
              <ContextMenu.Separator className="my-1 h-px bg-border" />
              <ContextMenu.Item className="flex cursor-default justify-between gap-4 rounded px-3 py-2 text-sm text-destructive outline-none focus:bg-accent" onSelect={() => setDialog("delete")}>{t("delete")}<span className="text-xs">Del / ⌫</span></ContextMenu.Item>
            </ContextMenu.Content></ContextMenu.Portal>}
            </ContextMenu.Root>
            {node && (
              <aside
                data-workflow-parameters
                aria-label={t("parameters")}
                className="max-h-[70dvh] overflow-y-auto border-t bg-background p-4 lg:border-l lg:border-t-0"
              >
                {
                  <div key={node.id} className="space-y-4">
                    <div className="flex items-start justify-between gap-2">
                      <div>
                        <h2 className="font-semibold">
                          {nodeTitle(node.type)}
                        </h2>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {node.id}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                      {!readOnly && <Button type="button" variant="ghost" size="icon" className="text-destructive" aria-label={t("delete")} onClick={() => setDialog("delete")}><Trash2 /></Button>}
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label={t("close")}
                        onClick={() => setSelected("")}
                      >
                        <X />
                      </Button>
                      </div>
                    </div>
                    <Button type="button" variant="outline" className="w-full" onClick={()=>setInspectorOpen(true)}><Maximize2 />{t("inspector.open")}</Button>
                    <label className="block space-y-2 text-sm">
                      <span><FieldCaption label={<>{commonText("common.name")}</>} /></span>
                      <Input
                        value={node.name}
                        readOnly={readOnly}
                        maxLength={60}
                        onChange={(event) =>
                          updateNode({ name: event.target.value })
                        }
                      />
                    </label>
                    <h3 className="border-t pt-4 text-sm font-semibold">
                      {t("parameters")}
                    </h3>
                    {(integrationManifest(node.type) || node.type === "host.command") && <WorkflowExecutionTarget workspace={workspace} value={node.settings ?? {}} change={settings => updateNode({settings})} readOnly={readOnly} hostOnly={node.type === "host.command" || !!integrationManifest(node.type)?.git} inherit />}
                    <WorkflowInputFields.Provider value={inputFields}>
                      <WorkflowNodeParameters
                        key={node.id}
                        node={node}
                        readOnly={readOnly}
                        update={(params) => updateNode({ params })}
                        invalid={(key, value) =>
                          invalidField(`${node.id}.${key}`, value)
                        }
                      />
                    </WorkflowInputFields.Provider>
                  </div>
                }
              </aside>
            )}
          </div>
        )}
        {tab === "history" && draft.id && renderTable?.(historyConfig, refresh)}
        {tab === "executions" && draft.id && (
          <div className="space-y-5">
            {renderTable?.(executionsConfig, refresh)}
          </div>
        )}
        {pickerOpen && !readOnly && (
          <WorkflowNodePicker
            inserting={insertAt !== undefined}
            connection={
              adjacent && graph
                ? {
                    ports: ports(
                      graph.nodes.find(
                        (node) => node.id === adjacent.from.node,
                      )!,
                      "outputs",
                    ),
                    value: `${adjacent.from.port}:${adjacent.from.index}`,
                    label: port => outputPortLabel(graph.nodes.find(node => node.id === adjacent.from.node)!, port),
                    change: (value) => {
                      const [port, index] = value.split(":");
                      setAdjacent({
                        ...adjacent,
                        from: {
                          ...adjacent.from,
                          port: port!,
                          index: Number(index),
                        },
                      });
                    },
                  }
                : undefined
            }
            accepts={(type) =>
              adjacent && graph
                ? ports(
                    graph.nodes.find((node) => node.id === adjacent.from.node)!,
                    "outputs",
                  ).length === 0 ||
                  ports(newNode(graph, type, type), "inputs").filter(
                    (input) =>
                      input.type ===
                      ports(
                        graph.nodes.find(
                          (node) => node.id === adjacent.from.node,
                        )!,
                        "outputs",
                      ).find(
                        (port) =>
                          port.port === adjacent.from.port &&
                          (port.index ?? 0) === adjacent.from.index,
                      )?.type,
                  ).length === 1
                : insertAt === undefined ||
                  (!!graph?.connections[insertAt] &&
                    !!insertionPorts(graph, graph.connections[insertAt]!, type))
            }
            select={add}
            close={() => {
              setPickerOpen(false);
              setInsertAt(undefined);
              setAdjacent(undefined);
            }}
          />
        )}
        {inspectorOpen && graph && node && <WorkflowNodeInspector
          workspace={workspace} workflow={draft.id ?? ""} graph={graph} node={node}
          draftRevision={draft.draft_revision_id ?? ""} readOnly={readOnly} canRun={runnable && !invalid}
          {...(execution?.id ? {initialExecution:String(execution.id)} : {})}
          inputFields={inputFields} update={updateNode} invalid={(key,value)=>invalidField(`${node.id}.${key}`,value)}
          changeGraph={change} close={()=>setInspectorOpen(false)} select={setSelected} duplicate={duplicateNode}
          remove={()=>{setInspectorOpen(false);setDialog("delete");}}
          run={()=>{setInspectorOpen(false);setRunRevision("draft");setInput({});setDialog("run");}}
        />}
        {outputPreview && (
          <Modal
            title={t("viewOutput")}
            close={() => setOutputPreview(undefined)}
          >
            <WorkflowOutputPreview output={outputPreview} />
          </Modal>
        )}
        {resultsOpen && execution && <Modal wide title={t("results")} close={() => {setResultsOpen(false);setExecutionId("");}}>{executionResults}</Modal>}
        {executionSettingsOpen && graph && <Modal title={t("execution.defaults")} footer={<Button type="button" onClick={()=>setExecutionSettingsOpen(false)}>{t("close")}</Button>} close={() => setExecutionSettingsOpen(false)}>
          <WorkflowExecutionTarget workspace={workspace} value={((graph.settings as Record<string,unknown> | undefined)?.execution as Record<string,unknown>) ?? {}} readOnly={readOnly}
            change={execution => change({...graph, settings:{...(graph.settings as Record<string,unknown> ?? {}), execution}})} />

        </Modal>}
        {dialog && (
          <Modal
            footer={dialog === "delete" ? (<div className="flex justify-end gap-2">
                <Button type="button" variant="outline" onClick={() => setDialog(null)}>{t("cancel")}</Button>
                <Button
                  type="button"
                  variant="destructive"
                  onClick={() => {
                    if (graph && node)
                      change({
                        ...graph,
                        nodes: graph.nodes.filter(
                          (existing) => existing.id !== node.id,
                        ),
                        connections: graph.connections.filter(
                          (link) =>
                            link.from.node !== node.id &&
                            link.to.node !== node.id,
                        ),
                      });
                    setSelected("");
                    setDialog(null);
                  }}
                >
                  {t("confirm")}
                </Button>
                </div>) : (<div className="flex justify-end gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    disabled={busy}
                    onClick={() => setDialog(null)}
                  >
                    {t("cancel")}
                  </Button>
                  <Button
                    type="button"
                    disabled={
                      busy ||
                      (dialog === "run" &&
                        (!entry ||
                          inputInvalid ||
                          !validRunInputs(inputSchema, input))) ||
                      (dialog === "publish" && !versionName.trim())
                    }
                    onClick={() => void perform()}
                  >
                    {t("confirm")}
                  </Button>
                </div>)}
            title={t(dialog === "delete" ? "delete" : dialog)}
            close={() => {
              if (!busy) setDialog(null);
            }}
          >
            {dialog === "delete" ? (
              <>
                <p className="mb-5 text-sm">
                  {node?.name} · {t("deleteHelp")}
                </p>

              </>
            ) : (
              <div className="space-y-4">
                {dialog === "run" && (
                  <>
                    <select
                      aria-label={t("history")}
                      className={control}
                      value={runRevision}
                      onChange={(event) =>
                        setRunRevision(
                          event.target.value as "draft" | "published",
                        )
                      }
                    >
                      <option value="draft">{t("draft")}</option>
                      <option
                        value="published"
                        disabled={!draft.published_revision_id}
                      >
                        {t("published")}
                      </option>
                    </select>
                    <AutoField label={t("entry")} help={t("entryHelp")}>
                      {manualEntries.length ? <select aria-label={t("entry")} className={control} value={entry} onChange={event => setEntry(event.target.value)}>
                        {manualEntries.map(node => <option key={node.id} value={node.id}>{node.name || node.id}</option>)}
                      </select> : <div className="space-y-3 rounded-md border bg-muted/30 p-3"><p role="status" className="text-sm text-muted-foreground">{t("entryMissing")}</p>{!readOnly && runRevision === "draft" && <Button type="button" variant="outline" onClick={() => {
                        if (!graph) return;
                        const created = newNode(graph, "core.trigger.manual", nodeTitle("core.trigger.manual"));
                        change({...graph,nodes:[...graph.nodes,created]});setSelected(created.id);setInspectorOpen(true);setDialog(null);
                      }}>{t("entryAdd")}</Button>}</div>}
                    </AutoField>
                    <WorkflowRunInputs
                      schema={inputSchema}
                      value={input}
                      change={setInput}
                      invalid={setInputInvalid}
                    />
                  </>
                )}
                {dialog === "publish" && (
                  <>
                    <p className="text-sm text-muted-foreground">
                      {t("publishHelp")}
                    </p>
                    <label className="block space-y-2 text-sm">
                      <span><FieldCaption label={<>{t("versionName")}</>} /></span>
                      <Input
                        value={versionName}
                        maxLength={64}
                        onChange={(event) => setVersionName(event.target.value)}
                      />
                    </label>
                    {draft.trigger_run_as && (
                      <label className="flex items-center gap-2 text-sm">
                        <input
                          type="checkbox"
                          checked={identityConfirmed}
                          onChange={(event) =>
                            setIdentityConfirmed(event.target.checked)
                          }
                        />
                        <FieldCaption label={t("identity")} /> ({draft.trigger_run_as})
                      </label>
                    )}
                  </>
                )}
                {dialog === "unpublish" && (
                  <p className="text-sm">{t("unpublish")}?</p>
                )}
                {error && (
                  <p role="alert" className="text-sm text-destructive">
                    {error}
                  </p>
                )}

              </div>
            )}
          </Modal>
        )}
      </div>
    </WorkflowAssetContext.Provider>
  );
}
