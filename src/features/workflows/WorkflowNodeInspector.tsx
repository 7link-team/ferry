import { Switch } from "../../shared/ui/switch";
import { FieldCaption } from "../../shared/forms/FieldHelp";
import { Dialog, DropdownMenu } from "radix-ui";
import {
  ArrowLeft,
  ArrowRight,
  Braces,
  MoreHorizontal,
  Pin,
  Play,
  RefreshCw,
  X,
} from "lucide-react";
import { useContext, useEffect, useRef, useState } from "react";
import { EditorFormActionsContext } from "../../shared/forms/auto-form/form-actions";
import { api, errorText } from "../../shared/api/api";
import { commonText, workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { parseGraph, type Graph, type GraphNode } from "./graph";
import { WorkflowDataView } from "./WorkflowDataView";
import {
  WorkflowInputFields,
  WorkflowNodeParameters,
  StructuredValue,
} from "./WorkflowNodeParameters";
import { nodeTitle } from "./WorkflowNodePicker";
import { integrationManifest } from "./integrations";
import { WorkflowExecutionTarget } from "./WorkflowExecutionTarget";
import {
  canPinNode,
  insertFieldReference,
  nodeModes,
  parameterSelections,
  type NodeExecutionData,
  type WorkflowItem,
} from "./workflow-data";

type Run = { id: string; number: number; state: string; revision_id: string };
type Attempt = { id: string; node_id: string; state: string; attempt: number };
const control =
  "h-10 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
const itemClass =
  "flex cursor-default items-center gap-2 rounded px-3 py-2 text-sm outline-none focus:bg-accent data-[disabled]:opacity-50";

export function WorkflowNodeInspector({
  workspace,
  workflow,
  graph,
  node,
  draftRevision,
  readOnly,
  canRun,
  initialExecution,
  inputFields,
  update,
  invalid,
  changeGraph,
  close,
  select,
  duplicate,
  remove,
  run,
}: {
  workspace: string;
  workflow: string;
  graph: Graph;
  node: GraphNode;
  draftRevision: string;
  readOnly: boolean;
  canRun: boolean;
  initialExecution?: string;
  inputFields: string[];
  update: (value: Partial<GraphNode>) => void;
  invalid: (key: string, value: boolean) => void;
  changeGraph: (graph: Graph) => void;
  close: () => void;
  select: (id: string) => void;
  duplicate: () => void;
  remove: () => void;
  run: () => void;
}) {
  const endpoint = `/workspaces/${workspace}/workflows/${workflow}`;
  const formActions = useContext(EditorFormActionsContext);
  const [runs, setRuns] = useState<Run[]>([]);
  const [execution, setExecution] = useState(initialExecution ?? "");
  const [attempts, setAttempts] = useState<Attempt[]>([]);
  const [attempt, setAttempt] = useState("");
  const [data, setData] = useState<NodeExecutionData>();
  const [source, setSource] = useState("execution");
  const [sample, setSample] = useState<Record<string, unknown>>({});
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [tab, setTab] = useState("parameters");
  const [mobile, setMobile] = useState("parameters");
  const [preview, setPreview] = useState<Record<string, unknown>[]>();
  const [previewBusy, setPreviewBusy] = useState(false);
  const [previewError, setPreviewError] = useState("");
  const [outputMode, setOutputMode] = useState("execution");
  const [reference, setReference] = useState("");
  const [referenceStatus, setReferenceStatus] = useState("");
  const [frozenNode, setFrozenNode] = useState<GraphNode>();
  const [sourceNames, setSourceNames] = useState<Record<string, string>>({});
  const target = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null);
  const previewRequest = useRef<AbortController | null>(null);
  const pins = (graph.pin_data ?? {}) as Record<string, WorkflowItem[]>;
  const pinned = Object.hasOwn(pins, node.id);
  const inputItems: WorkflowItem[] =
    source === "sample" ? [{ json: sample }] : (data?.input.items ?? []);
  const index = graph.nodes.findIndex((candidate) => candidate.id === node.id);
  const dirtyData =
    !!data &&
    (data.revision_id !== draftRevision ||
      JSON.stringify(frozenNode?.params) !== JSON.stringify(node.params) ||
      JSON.stringify(frozenNode?.settings ?? {}) !==
        JSON.stringify(node.settings ?? {}));

  useEffect(() => {
    if (!workflow) return;
    const controller = new AbortController();
    void api<{ items: Run[] }>(`${endpoint}/executions?page_size=50`, {
      signal: controller.signal,
    })
      .then((result) => {
        if (!controller.signal.aborted) {
          setRuns(result.items);
          setExecution((current) => current || result.items[0]?.id || "");
        }
      })
      .catch((cause) => {
        if (!controller.signal.aborted) setError(errorText(cause));
      });
    return () => controller.abort();
  }, [endpoint, workflow, refresh]);
  useEffect(() => {
    setAttempts([]);
    setAttempt("");
    setData(undefined);
    setFrozenNode(undefined);
    setError("");
    if (!execution) return;
    const controller = new AbortController();
    setLoading(true);
    void api<{ items: Attempt[] }>(
      `/workspaces/${workspace}/workflow-executions/${execution}/node-runs`,
      { signal: controller.signal },
    )
      .then((result) => {
        if (!controller.signal.aborted) {
          const list = result.items.filter((item) => item.node_id === node.id);
          setAttempts(list);
          setAttempt(list.at(-1)?.id ?? "");
        }
      })
      .catch((cause) => {
        if (!controller.signal.aborted) setError(errorText(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [workspace, execution, node.id, refresh]);
  useEffect(() => {
    setData(undefined);
    setFrozenNode(undefined);
    if (!attempt) return;
    const controller = new AbortController();
    setLoading(true);
    setError("");
    void (async () => {
      const result = await api<NodeExecutionData>(
        `/workspaces/${workspace}/workflow-executions/${execution}/node-runs/${attempt}/data`,
        { signal: controller.signal },
      );
      const revision = await api<{ definition: string }>(
        `${endpoint}/revisions/${result.revision_id}`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) {
        setData(result);
        const frozenGraph = parseGraph(revision.definition);
        setFrozenNode(frozenGraph.nodes.find((item) => item.id === node.id));
        setSourceNames(
          Object.fromEntries(
            frozenGraph.nodes.map((item) => [item.id, item.name]),
          ),
        );
      }
    })()
      .catch((cause) => {
        if (!controller.signal.aborted) setError(errorText(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [attempt, workspace, execution, endpoint, node.id, refresh]);
  useEffect(() => {
    previewRequest.current?.abort();
    setPreview(undefined);
    setPreviewError("");
    setPreviewBusy(false);
    return () => previewRequest.current?.abort();
  }, [node.params, source, sample, data]);
  useEffect(() => {
    target.current = null;
    setReference("");
    setReferenceStatus("");
  }, [node.id, tab]);

  const referenceField = (expression: string) => {
    setReference(expression);
    if (
      target.current?.isConnected &&
      insertFieldReference(target.current, expression)
    )
      setReferenceStatus(t("inspector.referenceInserted"));
    else setReferenceStatus(t("inspector.chooseTarget"));
  };
  const settings = (value: Record<string, unknown>) =>
    update({ settings: { ...node.settings, ...value } });
  const pinOutput = () => {
    if (readOnly || !canPinNode(node)) return;
    if (pinned) {
      const next = { ...pins };
      delete next[node.id];
      changeGraph({ ...graph, pin_data: next });
    } else if (data?.output && !dirtyData && data.output.ports.length === 1) {
      const items = data.output.ports[0]!.items.map((item) => {
        const copy = { ...item };
        delete copy.paired;
        return copy;
      });
      changeGraph({ ...graph, pin_data: { ...pins, [node.id]: items } });
    }
  };
  const resolve = async () => {
    const controller = new AbortController();
    previewRequest.current?.abort();
    previewRequest.current = controller;
    setPreviewBusy(true);
    setPreviewError("");
    setPreview(undefined);
    setOutputMode("preview");
    try {
      const saved = await api<{ version: number; definition: string }>(
        endpoint,
        { signal: controller.signal },
      );
      const savedGraph = parseGraph(saved.definition);
      const savedIndex = savedGraph.nodes.findIndex(
        (item) =>
          item.id === node.id &&
          item.type === node.type &&
          item.type_version === node.type_version,
      );
      if (savedIndex < 0) throw new Error(t("inspector.saveNodeFirst"));
      const result = await api<{ items: Record<string, unknown>[] }>(
        `${endpoint}/expression-previews`,
        {
          method: "POST",
          signal: controller.signal,
          body: JSON.stringify({
            version: saved.version,
            node_path: `/nodes/${savedIndex}`,
            params: node.params,
            data: {
              items: inputItems,
              upstream: (source === "execution"
                ? (data?.input.sources ?? [])
                : []
              )
                .filter(
                  (item, index, items) =>
                    items.findIndex(
                      (other) =>
                        other.node === item.node &&
                        other.iteration_hash === item.iteration_hash,
                    ) === index,
                )
                .map((item) => ({
                  node: item.node,
                  name: sourceNames[item.node] ?? item.node,
                  iteration_hash: item.iteration_hash,
                  items: item.items,
                })),
            },
            selections: parameterSelections(node, inputItems.length),
          }),
        },
      );
      if (!controller.signal.aborted) setPreview(result.items);
    } catch (cause) {
      if (!controller.signal.aborted) setPreviewError(errorText(cause));
    } finally {
      if (!controller.signal.aborted) setPreviewBusy(false);
    }
  };
  const panel = (name: string) =>
    `min-h-0 min-w-0 flex-col overflow-hidden ${mobile === name ? "flex" : "hidden"} lg:flex`;
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby="workflow-inspector-description"
          className="fixed inset-2 z-50 flex min-h-0 flex-col overflow-hidden rounded-xl border bg-background shadow-xl lg:inset-5"
          onKeyDown={(event) => {
            if (event.key !== "Escape") event.stopPropagation();
          }}
        >
          <header className="flex shrink-0 flex-wrap items-center gap-3 border-b px-4 py-3 lg:px-6">
            <div className="min-w-0 basis-full lg:basis-0 lg:flex-1">
              <Dialog.Title className="truncate text-base font-semibold">
                {node.name}{" "}
                <span className="ml-2 text-xs font-normal text-muted-foreground">
                  {nodeTitle(node.type)}
                </span>
              </Dialog.Title>
              <Dialog.Description
                id="workflow-inspector-description"
                className="mt-1 text-xs text-muted-foreground"
              >
                {t("inspector.description")}
              </Dialog.Description>
            </div>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label={t("inspector.previousNode")}
              disabled={index === 0}
              onClick={() => select(graph.nodes[index - 1]!.id)}
            >
              <ArrowLeft />
            </Button>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label={t("inspector.nextNode")}
              disabled={index === graph.nodes.length - 1}
              onClick={() => select(graph.nodes[index + 1]!.id)}
            >
              <ArrowRight />
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!canRun || formActions?.saving}
              onClick={run}
            >
              <Play />
              {t(!readOnly && formActions?.saveAndWait ? "saveAndRun" : "inspector.runWorkflow")}
            </Button>
            {!readOnly && formActions && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={formActions.saving || formActions.blocked}
                onClick={formActions.save}
              >
                {commonText(
                  formActions.saving ? "common.saving" : "common.save",
                )}
              </Button>
            )}
            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  aria-label={t("inspector.actions")}
                >
                  <MoreHorizontal />
                </Button>
              </DropdownMenu.Trigger>
              <DropdownMenu.Portal>
                <DropdownMenu.Content
                  align="end"
                  className="z-[60] min-w-52 rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
                >
                  <DropdownMenu.Item
                    className={itemClass}
                    disabled={readOnly}
                    onSelect={duplicate}
                  >
                    {t("duplicateNode")}
                  </DropdownMenu.Item>
                  <DropdownMenu.Item
                    className={itemClass}
                    disabled={readOnly}
                    onSelect={() => {
                      setTab("settings");
                      setMobile("parameters");
                    }}
                  >
                    {t("inspector.settings")}
                  </DropdownMenu.Item>
                  <DropdownMenu.Item
                    className={itemClass}
                    disabled={
                      readOnly ||
                      !canPinNode(node) ||
                      (!pinned && (!data?.output || dirtyData))
                    }
                    onSelect={pinOutput}
                  >
                    <Pin />
                    {t(pinned ? "inspector.unpin" : "inspector.pin")}
                  </DropdownMenu.Item>
                  <DropdownMenu.Separator className="my-1 h-px bg-border" />
                  <DropdownMenu.Item
                    className={`${itemClass} text-destructive`}
                    disabled={readOnly}
                    onSelect={remove}
                  >
                    {t("delete")}
                  </DropdownMenu.Item>
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu.Root>
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
          </header>
          <div className="flex shrink-0 flex-wrap items-center gap-2 border-b bg-muted/20 px-4 py-2 text-xs">
            <label className="flex min-w-0 basis-full items-center gap-2 sm:basis-auto sm:flex-1 lg:flex-none">
              <span className="shrink-0 whitespace-nowrap"><FieldCaption label={t("inspector.execution")} /></span>
              <select
                aria-label={t("inspector.execution")}
                className="h-8 min-w-0 flex-1 rounded border bg-background px-2 text-xs sm:max-w-60"
                value={execution}
                onChange={(event) => setExecution(event.target.value)}
              >
                <option value="">{t("inspector.noExecution")}</option>
                {runs.map((item) => (
                  <option key={item.id} value={item.id}>
                    #{item.number} · {item.state}
                  </option>
                ))}
              </select>
            </label>
            {!!attempts.length && (
              <select
                aria-label={t("attempt")}
                className="h-8 min-w-0 max-w-full flex-1 rounded border bg-background px-2 text-xs sm:flex-none"
                value={attempt}
                onChange={(event) => setAttempt(event.target.value)}
              >
                {attempts.map((item) => (
                  <option key={item.id} value={item.id}>
                    {t("attempt")} {item.attempt} · {item.state}
                  </option>
                ))}
              </select>
            )}
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label={t("inspector.refresh")}
              disabled={loading}
              onClick={() => setRefresh((value) => value + 1)}
            >
              <RefreshCw className={loading ? "animate-spin" : ""} />
            </Button>
            {data && (
              <span className="text-muted-foreground">
                {t("inspector.frozenAt")}{" "}
                {new Date(data.frozen_at).toLocaleString()}
              </span>
            )}
            {pinned && (
              <span className="inline-flex items-center gap-1 rounded border px-2 py-1">
                <Pin className="size-3" />
                {t("inspector.pinned")}
              </span>
            )}
            {dirtyData && (
              <span
                role="status"
                className="text-amber-700 dark:text-amber-400"
              >
                {t("inspector.historical")}
              </span>
            )}
          </div>
          {error && (
            <p
              role="alert"
              className="shrink-0 border-b bg-destructive/5 px-4 py-2 text-sm text-destructive"
            >
              {error}
            </p>
          )}
          <div
            className="flex shrink-0 gap-1 border-b p-2 lg:hidden"
            role="group"
            aria-label={t("inspector.panes")}
          >
            {(["input", "parameters", "output"] as const).map((key) => (
              <Button
                key={key}
                type="button"
                className="flex-1"
                size="sm"
                variant={mobile === key ? "secondary" : "ghost"}
                aria-pressed={mobile === key}
                onClick={() => setMobile(key)}
              >
                {t(`inspector.${key}`)}
              </Button>
            ))}
          </div>
          <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(340px,1.12fr)_minmax(0,1fr)]">
            <section
              className={`${panel("input")} border-r`}
              aria-label={t("inspector.input")}
            >
              <div className="flex shrink-0 items-center justify-between border-b px-4 py-3">
                <h3 className="text-sm font-semibold">
                  {t("inspector.input")}
                </h3>
                <select
                  aria-label={t("inspector.inputSource")}
                  className="h-8 rounded border bg-background px-2 text-xs"
                  value={source}
                  onChange={(event) => setSource(event.target.value)}
                >
                  <option value="execution">
                    {t("inspector.executionData")}
                  </option>
                  <option value="sample">{t("inspector.sample")}</option>
                </select>
              </div>
              <div className="min-h-0 flex-1 overflow-auto p-4">
                {source === "sample" ? (
                  <div className="space-y-4">
                    <p className="text-xs leading-5 text-muted-foreground">
                      {t("inspector.sampleHelp")}
                    </p>
                    <StructuredValue
                      label={t("inspector.sample")}
                      value={sample}
                      readOnly={readOnly}
                      change={(value) =>
                        setSample(value as Record<string, unknown>)
                      }
                    />
                  </div>
                ) : (
                  <>
                    {!!data?.input.sources.length && (
                      <p className="mb-4 text-xs leading-5 text-muted-foreground">
                        {data.input.sources
                          .map(
                            (item) =>
                              `${graph.nodes.find((n) => n.id === item.node)?.name ?? item.node} · ${item.port === "main" ? t("mainPort") : item.port}${item.index > 0 ? ` #${item.index + 1}` : ""}`,
                          )
                          .join(" / ")}
                      </p>
                    )}
                    <WorkflowDataView
                      items={inputItems}
                      {...(!readOnly ? { reference: referenceField } : {})}
                      loading={loading} empty={t("inspector.noInput")}
                    />
                  </>
                )}
              </div>
            </section>
            <section
              className={`${panel("parameters")} border-r`}
              aria-label={t("inspector.parameters")}
            >
              <div className="flex shrink-0 items-center gap-1 border-b px-4 py-2">
                {(["parameters", "settings"] as const).map((key) => (
                  <Button
                    key={key}
                    type="button"
                    size="sm"
                    variant={tab === key ? "secondary" : "ghost"}
                    aria-pressed={tab === key}
                    onClick={() => setTab(key)}
                  >
                    {t(`inspector.${key}`)}
                  </Button>
                ))}
                <Button
                  className="ml-auto"
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={
                    readOnly ||
                    !workflow ||
                    previewBusy ||
                    (source === "execution" && !data)
                  }
                  onClick={() => {
                    void resolve();
                    setMobile("output");
                  }}
                >
                  <Braces />
                  {t("inspector.resolve")}
                </Button>
              </div>
              <div
                data-workflow-inspector-parameters
                className="min-h-0 flex-1 space-y-5 overflow-y-auto p-5"
                onFocusCapture={(event) => {
                  const field = event.target;
                  if (
                    (field instanceof HTMLInputElement ||
                      field instanceof HTMLTextAreaElement) &&
                    !field.readOnly &&
                    !(
                      field instanceof HTMLInputElement &&
                      field.type === "password"
                    )
                  )
                    target.current = field;
                }}
                onDragOver={(event) => {
                  if (
                    !readOnly &&
                    event.dataTransfer.types.includes(
                      "application/x-workflow-field",
                    )
                  )
                    event.preventDefault();
                }}
                onDrop={(event) => {
                  if (readOnly) return;
                  const expression = event.dataTransfer.getData(
                    "application/x-workflow-field",
                  );
                  if (!expression) return;
                  event.preventDefault();
                  const field = event.target;
                  if (
                    field instanceof HTMLInputElement ||
                    field instanceof HTMLTextAreaElement
                  ) {
                    if (insertFieldReference(field, expression))
                      setReferenceStatus(t("inspector.referenceInserted"));
                    else setReferenceStatus(t("inspector.chooseTarget"));
                  }
                  setReference(expression);
                }}
              >
                {tab === "parameters" ? (
                  <>
                    <label className="block space-y-2 text-sm">
                      <span><FieldCaption label={<>{commonText("common.name")}</>} /></span>
                      <Input
                        aria-label={t("inspector.nodeName")}
                        value={node.name}
                        maxLength={60}
                        readOnly={readOnly}
                        onChange={(event) =>
                          update({ name: event.target.value })
                        }
                      />
                    </label>
                    {!readOnly && (
                      <div className="space-y-2 rounded-md border bg-muted/20 p-3 text-xs leading-5 text-muted-foreground">
                        <p>{t("inspector.referenceHelp")}</p>
                        {reference && (
                          <>
                            <code className="block break-all text-primary">
                              {reference}
                            </code>
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              onClick={() => referenceField(reference)}
                            >
                              {t("inspector.insertReference")}
                            </Button>
                          </>
                        )}
                        {referenceStatus && (
                          <p role="status">{referenceStatus}</p>
                        )}
                      </div>
                    )}
                    {node.type === "host.command" && <WorkflowExecutionTarget workspace={workspace} value={node.settings ?? {}} change={settings => update({settings})} readOnly={readOnly} hostOnly inherit />}
                    <WorkflowInputFields.Provider value={inputFields}>
                      <WorkflowNodeParameters
                        key={node.id}
                        node={node}
                        readOnly={readOnly}
                        update={(params) => update({ params })}
                        invalid={invalid}
                      />
                    </WorkflowInputFields.Provider>
                  </>
                ) : (
                  <>
                    {integrationManifest(node.type) && <WorkflowExecutionTarget workspace={workspace} value={node.settings ?? {}} change={value => update({settings:value})} readOnly={readOnly} hostOnly={!!integrationManifest(node.type)?.git} inherit />}
                    <label className="block space-y-2 text-sm">
                      <span><FieldCaption label={<>{t("inspector.mode")}</>} /></span>
                      <select
                        aria-label={t("inspector.mode")}
                        className={control}
                        value={String(node.settings?.mode ?? "active")}
                        disabled={readOnly}
                        onChange={(event) =>
                          settings({ mode: event.target.value })
                        }
                      >
                        {nodeModes(node).map((mode) => (
                          <option key={mode} value={mode}>
                            {t(
                              `inspector.mode.${mode}` as Parameters<
                                typeof t
                              >[0],
                            )}
                          </option>
                        ))}
                      </select>
                    </label>
                    <p className="text-xs leading-5 text-muted-foreground">
                      {t(
                        `inspector.modeHelp.${String(node.settings?.mode ?? "active")}` as Parameters<
                          typeof t
                        >[0],
                      )}
                    </p>
                    <label className="flex items-center gap-2 text-sm">
                      <Switch
                        checked={node.settings?.execute_once === true}
                        disabled={readOnly}
                        onCheckedChange={(checked) =>
                          settings({ execute_once: checked })
                        }
                      />
                      <FieldCaption label={t("inspector.executeOnce")} />
                    </label>
                    {canPinNode(node) && (
                      <label className="flex items-center gap-2 text-sm">
                        <Switch
                          checked={node.settings?.always_output === true}
                          disabled={readOnly}
                          onCheckedChange={(checked) =>
                            settings({ always_output: checked })
                          }
                        />
                        <FieldCaption label={t("inspector.alwaysOutput")} />
                      </label>
                    )}
                    <label className="block space-y-2 text-sm">
                      <span><FieldCaption label={<>{t("inspector.notes")}</>} /></span>
                      <textarea
                        aria-label={t("inspector.notes")}
                        className={`${control} min-h-28 py-2`}
                        value={String(node.settings?.notes ?? "")}
                        maxLength={4000}
                        readOnly={readOnly}
                        onChange={(event) =>
                          settings({ notes: event.target.value })
                        }
                      />
                    </label>
                    <div className="space-y-3 border-t pt-4">
                      <h4 className="text-sm font-medium">
                        {t("inspector.pinTitle")}
                      </h4>
                      <p className="text-xs leading-5 text-muted-foreground">
                        {t("inspector.pinHelp")}
                      </p>
                      <Button
                        type="button"
                        variant="outline"
                        disabled={
                          readOnly ||
                          !canPinNode(node) ||
                          (!pinned && (!data?.output || dirtyData))
                        }
                        onClick={pinOutput}
                      >
                        <Pin />
                        {t(pinned ? "inspector.unpin" : "inspector.pin")}
                      </Button>
                      {!canPinNode(node) && (
                        <p className="text-xs text-muted-foreground">
                          {t("inspector.pinUnavailable")}
                        </p>
                      )}
                    </div>
                  </>
                )}
              </div>
              <p className="shrink-0 border-t px-5 py-3 text-xs leading-5 text-muted-foreground">
                {t("inspector.saveHelp")}
              </p>
            </section>
            <section
              className={panel("output")}
              aria-label={t("inspector.output")}
            >
              <div className="flex shrink-0 items-center gap-1 border-b px-4 py-2">
                {(["execution", "preview"] as const).map((key) => (
                  <Button
                    key={key}
                    type="button"
                    size="sm"
                    variant={outputMode === key ? "secondary" : "ghost"}
                    aria-pressed={outputMode === key}
                    onClick={() => setOutputMode(key)}
                  >
                    {t(`inspector.output.${key}`)}
                  </Button>
                ))}
              </div>
              <div className="min-h-0 flex-1 space-y-4 overflow-auto p-4">
                {outputMode === "preview" ? (
                  <>
                    <p className="text-xs leading-5 text-muted-foreground">
                      {t("inspector.previewHelp")}
                    </p>
                    {previewError && (
                      <p role="alert" className="text-sm text-destructive">
                        {previewError}
                      </p>
                    )}
                    <WorkflowDataView
                      items={(preview ?? []).map((json) => ({ json }))}
                      loading={previewBusy} empty={t("inspector.noPreview")}
                    />
                  </>
                ) : (
                  <>
                    {runs.find(item => item.id === execution)?.state === "failed" && <p role="status" className="text-sm text-destructive">{t("workflowRunFailed")}</p>}
                    {data && (data.state === "failed" || !data.output) && (
                      <p className="text-sm text-destructive">
                        {t("inspector.executionFailed")} · {data.end_reason}
                      </p>
                    )}
                    {(data?.output?.ports ?? []).map((port) => (
                      <section
                        key={`${port.name}:${port.index}`}
                        className="space-y-3"
                      >
                        <h4 className="text-sm font-medium">
                          {port.name === "main" ? t("mainPort") : port.name}{(data?.output?.ports.length ?? 0) > 1 ? ` · ${t("outputPort")} #${port.index + 1}` : ""}
                          {port.skipped ? ` · ${t("inspector.skipped")}` : ""}
                        </h4>
                        {port.name === "main" && <p className="text-xs text-muted-foreground">{t("defaultOutputHelp")}</p>}
                        <WorkflowDataView items={port.items} />
                      </section>
                    ))}
                    {!data?.output && (
                      <WorkflowDataView
                        items={[]}
                        loading={loading} empty={t("inspector.noOutput")}
                      />
                    )}
                    {data && (
                      <details className="border-t pt-3">
                        <summary className="cursor-pointer text-xs text-muted-foreground">
                          {t("inspector.resolvedParameters")}
                        </summary>
                        <div className="mt-4">
                          <WorkflowDataView
                            items={data.resolved_params.map((json) => ({
                              json,
                            }))}
                          />
                        </div>
                      </details>
                    )}
                  </>
                )}
              </div>
            </section>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
