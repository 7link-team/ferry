import { hostRepositoryText, useCommonTranslation } from "../i18n";
import { currentLocale } from "../locale";
import { AutoField, FieldControl, selectDisplay } from "./auto-form";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { parseDocument, visit } from "yaml";
import { Button } from "./ui/button";
import { Trash2 } from "lucide-react";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { Input } from "./ui/input";
import { Checkbox } from "./ui/checkbox";
import { listResource, named } from "../resource";

type Path = (string | number)[];
type Row = Record<string, unknown>;
const object = (value: unknown): value is Row =>
  !!value && typeof value === "object" && !Array.isArray(value);
const area =
  "min-h-24 w-full rounded-md border border-input bg-transparent p-3 font-mono text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
const grid = "grid gap-3 sm:grid-cols-2";

/** Keep the YAML document, rather than rebuilding a partial manifest from form fields.
 * Unedited fields, comments, scalar types and mode-specific overrides survive edits.
 */
export function readManifest(source: string) {
  const doc = parseDocument(source, { uniqueKeys: true });
  if (doc.errors.length) throw new Error(doc.errors[0]!.message);
  let alias = false;
  visit(doc, {
    Alias: () => {
      alias = true;
    },
  });
  if (alias)
    throw new Error(
      hostRepositoryText("manifest.001"),
    );
  const value: unknown = doc.toJS({ maxAliasCount: 50 });
  if (
    !object(value) ||
    !object(value.metadata) ||
    !Array.isArray(value.services) ||
    !value.services.every(object)
  )
    throw new Error(
      hostRepositoryText("manifest.002"),
    );
  const optionalObject = (parent: Row, key: string) => {
    if (parent[key] != null && !object(parent[key]))
      throw new Error(hostRepositoryText("manifest.003", {value1: key}));
  };
  optionalObject(value, "strategy");
  optionalObject(value, "overrides");
  optionalObject(value.metadata, "labels");
  // Do not reinterpret malformed collection shapes as empty lists and lose data.
  for (const service of value.services as Row[]) {
    for (const name of ["ports", "env", "volumes", "files"]) {
      const rows = service[name];
      if (rows !== undefined && (!Array.isArray(rows) || !rows.every(object)))
        throw new Error(hostRepositoryText("manifest.004", {value1: name}));
    }
    for (const name of ["command", "dependsOn"]) {
      const rows = service[name];
      if (
        rows != null &&
        (!Array.isArray(rows) || !rows.every((v) => typeof v === "string"))
      )
        throw new Error(hostRepositoryText("manifest.005", {value1: name}));
    }
    for (const name of ["artifact", "resources", "health"])
      optionalObject(service, name);
    if (object(service.resources))
      for (const key of ["requests", "limits"])
        optionalObject(service.resources, key);
    if (object(service.health))
      for (const key of ["readiness", "liveness"]) {
        optionalObject(service.health, key);
        const probe = service.health[key];
        if (object(probe)) optionalObject(probe, "httpGet");
      }
    for (const file of (service.files ?? []) as Row[])
      optionalObject(file, "values");
  }
  return { doc, value };
}

type Choice = { value: string; label: string };

/** A pipeline or build run chosen by name, keeping a stored ID that no longer resolves. */
function ReferencePicker({ label, value, empty, required, hint, readOnly, load, onChange }: {
  label: string; value: string; empty: string; required?: boolean; hint?: string; readOnly: boolean;
  load: () => Promise<Choice[]>; onChange: (value: string) => void;
}) {
  useCommonTranslation();
  const [choices, setChoices] = useState<Choice[]>();
  useEffect(() => {
    let live = true;
    setChoices(undefined);
    load().then(rows => { if (live) setChoices(rows); }, () => { if (live) setChoices([]); });
    return () => { live = false; };
  }, [load]);
  const known = choices ?? [];
  const selected = value === "0" ? "" : value;
  const options = [{ value: "", label: choices ? empty : hostRepositoryText("manifest.006") }, ...known,
    ...(selected && !known.some(choice => choice.value === selected) ? [{ value: selected, label: named(undefined, selected) }] : [])];
  return <AutoField label={label} required={required} help={hint}>
    <FieldControl name={label} field={{ name: label, type: "select", config: { options } }} value={selected} required={required} disabled={readOnly || !choices} onChange={next => onChange(String(next))} />
  </AutoField>;
}

function RunPicker({ workspace, pipeline, value, readOnly, onChange }: { workspace: string; pipeline: string; value: string; readOnly: boolean; onChange: (value: string) => void }) {
  const { i18n } = useCommonTranslation();
  const load = useCallback(() => runChoices(workspace, pipeline), [workspace, pipeline, i18n.resolvedLanguage]);
  return <ReferencePicker label={hostRepositoryText("manifest.007")} value={value} empty={hostRepositoryText("manifest.008")} readOnly={readOnly} load={load} onChange={onChange}
    hint={hostRepositoryText("manifest.009")} />;
}

async function pipelineChoices(workspace: string): Promise<Choice[]> {
  const choices: Choice[] = [];
  for (let page = 1; ; page += 1) {
    const result = await listResource<{ id: string; name: string }>(workspace, "pipelines", { page, page_size: 200 });
    choices.push(...result.items.map(row => ({ value: row.id, label: named(row.name, row.id) })));
    if (!result.items.length || page * result.page_size >= result.total) return choices;
  }
}

async function runChoices(workspace: string, pipeline: string): Promise<Choice[]> {
  if (!pipeline || pipeline === "0") return [];
  const result = await listResource<{ id: string; number: number; finished_at: number }>(workspace, "pipeline-runs",
    { pipeline_id: pipeline, state: "succeeded", sort: "-number", page_size: 50 });
  return result.items.map(run => ({ value: run.id, label: `#${run.number} · ${run.finished_at > 0 && Number.isFinite(run.finished_at) ? new Date(run.finished_at).toLocaleString(currentLocale()) : "—"}` }));
}

export function ManifestEditor({
  value,
  mode,
  readOnly,
  onChange,
  onValidityChange,
  workspace,
}: {
  value: string;
  mode: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onValidityChange: (blocked: boolean) => void;
  /** When known, artifact sources choose the pipeline and build run by name. */
  workspace?: string;
}) {
  const { i18n } = useCommonTranslation();
  const loadPipelines = useCallback(() => workspace ? pipelineChoices(workspace) : Promise.resolve([]), [workspace]);
  const [sourceMode, setSourceMode] = useState(false);
  const [notice, setNotice] = useState("");
  const [pendingRemoval, setPendingRemoval] = useState<{ path: Path; index: number; action: string; target: string }>();
  const [invalidFields, setInvalidFields] = useState<Record<string, boolean>>(
    {},
  );
  const reportJson = useCallback(
    (key: string, invalid: boolean) =>
      setInvalidFields((current) =>
        current[key] === invalid ? current : { ...current, [key]: invalid },
      ),
    [],
  );
  const invalidJson = Object.values(invalidFields).some(Boolean);
  const parsed = useMemo(() => {
    try {
      return { data: readManifest(value), error: "" };
    } catch (error) {
      return {
        data: undefined,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }, [value, i18n.resolvedLanguage]);
  // Syntax errors block saving. Valid YAML that needs source-only editing remains saveable;
  // the existing server validator remains authoritative for capability/business validation.
  const syntaxError = useMemo(
    () => parseDocument(value).errors.length > 0,
    [value],
  );
  useEffect(() => {
    onValidityChange(syntaxError || invalidJson);
    return () => onValidityChange(false);
  }, [syntaxError, invalidJson, onValidityChange]);
  const doc = parsed.data?.doc;
  const services = (parsed.data?.value.services ?? []) as Row[];
  const get = (path: Path): unknown => doc?.getIn(path, false);
  const plain = (path: Path): unknown => {
    let item: unknown = parsed.data?.value;
    for (const key of path) item = (item as Row | undefined)?.[key];
    return item;
  };
  const change = (changes: { path: Path; value: unknown }[]) => {
    if (!doc || readOnly) return;
    const next = doc.clone();
    for (const change of changes) {
      // Optional objects may be explicitly null in existing documents.
      if (change.value !== undefined)
        for (let length = 1; length < change.path.length; length++) {
          const parent = change.path.slice(0, length);
          if (next.getIn(parent) == null)
            next.setIn(
              parent,
              next.createNode(
                typeof change.path[length] === "number" ? [] : {},
              ),
            );
        }
      if (
        change.path[0] === "strategy" &&
        change.value !== undefined &&
        !next.getIn(["strategy", "type"])
      )
        next.setIn(["strategy", "type"], "recreate");
      if (change.value === undefined) {
        if (next.hasIn(change.path)) next.deleteIn(change.path);
      } else
        next.setIn(
          change.path,
          typeof change.value === "object" && change.value !== null
            ? next.createNode(change.value)
            : change.value,
        );
    }
    setNotice("");
    onChange(next.toString());
  };
  const set = (path: Path, value: unknown) => change([{ path, value }]);
  const remove = (path: Path, index: number) => {
    if (!doc || readOnly) return;
    const next = doc.clone();
    next.deleteIn([...path, index]);
    setNotice("");
    onChange(next.toString());
  };
  const append = (path: Path, value: unknown) => {
    if (!doc || readOnly) return;
    const next = doc.clone();
    if (!next.hasIn(path)) next.setIn(path, next.createNode([]));
    next.addIn(path, next.createNode(value));
    setNotice("");
    onChange(next.toString());
  };
  const text = (
    label: string,
    path: Path,
    options: {
      number?: boolean;
      min?: number;
      max?: number;
      hint?: string;
      fallback?: string | number;
      required?: boolean;
      multiline?: boolean;
    } = {},
  ) => {
    const current = get(path);
    return <AutoField label={label} required={options.required} help={options.hint}>
      <FieldControl name={label} field={{ name: label, type: options.multiline ? "textarea" : options.number ? "number" : "text", config: { ...(options.min === undefined ? {} : { min: options.min }), ...(options.max === undefined ? {} : { max: options.max }) } }} value={String(current ?? options.fallback ?? "")} required={options.required} disabled={readOnly}
        onChange={value => set(path, value === "" && !options.required ? undefined : value)} />
    </AutoField>;
  };
  const select = (label: string, path: Path, choices: string[], fallback: string) => <AutoField label={label}>
    <FieldControl name={label} field={{ name: label, type: "select", config: { options: [...new Set([...choices, String(get(path) ?? fallback)])].map(value => ({ value, label: !choices.includes(value) ? selectDisplay(value, {options: []}) : value === "recreate" ? hostRepositoryText("manifest-extra.001") : value === "rolling" ? hostRepositoryText("manifest-extra.002") : value })) } }} value={String(get(path) ?? fallback)} disabled={readOnly} onChange={value => set(path, value)} />
  </AutoField>;

  const rows = (
    title: string,
    path: Path,
    initial: Row,
    render: (path: Path, row: Row) => React.ReactNode,
  ) => {
    const items = (plain(path) ?? []) as Row[];
    return (
      <section aria-label={title} className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <h4 className="text-sm font-medium">{title}</h4>
          {!readOnly && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={invalidJson}
              onClick={() => append(path, initial)}
            >
              {hostRepositoryText("manifest-extra.003", {value1: title})}
            </Button>
          )}
        </div>
        {items.length === 0 && (
          <p className="text-sm text-muted-foreground">{hostRepositoryText("manifest.011")}</p>
        )}
        {items.map((row, index) => (
          <fieldset key={index} className="space-y-3 border-l-2 pl-3">
            <legend className="sr-only">
              {title} {index + 1}
            </legend>
            <div className={grid}>{render([...path, index], row)}</div>
            {!readOnly && (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={hostRepositoryText("manifest.012", {value1: title, value2: index + 1})}
                title={hostRepositoryText("manifest.012", {value1: title, value2: index + 1})}
                disabled={invalidJson}
                onClick={() => setPendingRemoval({ path, index, action: hostRepositoryText("manifest.013", {value1: title}), target: String(row.name ?? row.key ?? index + 1) })}
              >
                <Trash2 className="size-4" />
              </Button>
            )}
          </fieldset>
        ))}
      </section>
    );
  };
  return (
    <section aria-label={hostRepositoryText("manifest.014")} className="min-w-0 space-y-4">
      {pendingRemoval && <DeleteConfirmDialog action={pendingRemoval.action} target={pendingRemoval.target} description={hostRepositoryText("manifest.015")} onClose={() => setPendingRemoval(undefined)} onConfirm={() => remove(pendingRemoval.path, pendingRemoval.index)} />}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-medium">{hostRepositoryText("manifest.016")}</h2>
        <div className="flex gap-2" role="group" aria-label={hostRepositoryText("manifest.017")}>
          <Button
            type="button"
            variant={!sourceMode ? "secondary" : "outline"}
            aria-pressed={!sourceMode}
            onClick={() => setSourceMode(false)}
          >
            {hostRepositoryText("manifest.018")}</Button>
          <Button
            type="button"
            variant={sourceMode ? "secondary" : "outline"}
            aria-pressed={sourceMode}
            disabled={invalidJson}
            onClick={() => setSourceMode(true)}
          >
            {hostRepositoryText("manifest.019")}</Button>
        </div>
      </div>
      {sourceMode || !parsed.data ? (
        <>
          <AutoField label={hostRepositoryText("manifest.020")} className="flex flex-col gap-2 text-sm">
            <textarea
              className={`${area} min-h-80`}
              value={value}
              readOnly={readOnly}
              spellCheck={false}
              onChange={(e) => onChange(e.target.value)}
              aria-invalid={syntaxError}
            />
          </AutoField>
          {parsed.error && (
            <p
              role="alert"
              className="whitespace-pre-wrap text-sm text-destructive"
            >
              {parsed.error}
            </p>
          )}
          <p className="text-sm text-muted-foreground">
            {hostRepositoryText("manifest.021")}</p>
        </>
      ) : (
        <>
          <div className={grid}>
            {text(hostRepositoryText("manifest.022"), ["metadata", "name"], {
              required: true,
              hint: hostRepositoryText("manifest.023"),
            })}
          </div>
          <p className="text-sm text-muted-foreground">
            {hostRepositoryText("manifest.024")}</p>
          {services.map((service, index) => {
            const path: Path = ["services", index];
            const p = (...keys: Path) => [...path, ...keys];
            const dependencies = (service.dependsOn ?? []) as string[];
            const artifact = object(service.artifact);
            return (
              <section
                key={index}
                aria-label={hostRepositoryText("manifest.025", {value1: index + 1})}
                className="space-y-5 rounded-md border p-4"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <h3 className="font-medium">
                    {index + 1}. {String(service.name || hostRepositoryText("manifest.026"))}
                  </h3>
                  {!readOnly && (
                    <div className="flex flex-wrap gap-1">
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label={hostRepositoryText("manifest.027", {value1: String(service.name || index + 1)})}
                        title={hostRepositoryText("manifest.027", {value1: String(service.name || index + 1)})}
                        disabled={invalidJson || services.length <= 1}
                        onClick={() => {
                          if (
                            services.some(
                              (other, otherIndex) =>
                                otherIndex !== index &&
                                ((other.dependsOn ?? []) as string[]).includes(
                                  String(service.name),
                                ),
                            )
                          ) {
                            setNotice(
                              hostRepositoryText("manifest.028"),
                            );
                            return;
                          }
                          setPendingRemoval({ path: ["services"], index, action: hostRepositoryText("manifest.029"), target: String(service.name || index + 1) });
                        }}
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </div>
                  )}
                </div>
                <div className={grid}>
                  <AutoField label={hostRepositoryText("manifest.030")} className="flex flex-col gap-1.5 text-sm">
                    <Input
                      value={String(service.name ?? "")}
                      required
                      disabled={readOnly}
                      onChange={(e) => {
                        if (
                          services.some(
                            (other, otherIndex) =>
                              otherIndex !== index &&
                              other.name === e.target.value,
                          )
                        ) {
                          setNotice(hostRepositoryText("manifest.031"));
                          return;
                        }
                        const changes = [
                          { path: p("name"), value: e.target.value as unknown },
                        ];
                        services.forEach((other, i) => {
                          if (Array.isArray(other.dependsOn))
                            other.dependsOn.forEach((name, j) => {
                              if (name === service.name)
                                changes.push({
                                  path: ["services", i, "dependsOn", j],
                                  value: e.target.value,
                                });
                            });
                        });
                        change(changes);
                      }}
                    />
                  </AutoField>
                  {text(hostRepositoryText("manifest.032"), p("replicas"), {
                    number: true,
                    min: 1,
                    max: 100,
                    fallback: 1,
                  })}
                  <AutoField label={hostRepositoryText("manifest.033")} className="flex flex-col gap-1.5 text-sm">
                    <select
                      aria-label={hostRepositoryText("manifest.033")}
                      className="h-9 rounded-md border border-input bg-background px-2"
                      value={artifact ? "artifact" : "image"}
                      disabled={readOnly}
                      onChange={(e) => {
                        // Switching is an ordinary draft edit: it takes effect
                        // only when saved and can be undone by switching back.
                        change([
                          {
                            path: p("image"),
                            value: e.target.value === "image" ? "" : undefined,
                          },
                          {
                            path: p("artifact"),
                            value:
                              e.target.value === "artifact"
                                ? { source: "pipeline", pipeline: "", path: "" }
                                : undefined,
                          },
                        ]);
                      }}
                    >
                      <option value="image">{hostRepositoryText("manifest.034")}</option>
                      <option value="artifact">{hostRepositoryText("manifest.035")}</option>
                    </select>
                  </AutoField>
                  {!artifact &&
                    text(hostRepositoryText("manifest.036"), p("image"), {
                      required: true,
                      hint: hostRepositoryText("manifest.037"),
                    })}
                </div>
                {(mode === "native") !== artifact && (
                  <p role="status" className="text-sm text-destructive">
                    {mode === "native"
                      ? hostRepositoryText("manifest.038")
                      : hostRepositoryText("manifest.039")}
                  </p>
                )}
                {artifact && (
                  <div className={grid}>
                    {workspace ? (
                      <ReferencePicker label={hostRepositoryText("manifest.040")} required readOnly={readOnly} empty={hostRepositoryText("manifest.041")}
                        value={String(get(p("artifact", "pipeline")) ?? "")} load={loadPipelines}
                        // A pinned run belongs to the previous pipeline.
                        onChange={next => change([{ path: p("artifact", "pipeline"), value: next || undefined }, { path: p("artifact", "run"), value: undefined }])} />
                    ) : text(hostRepositoryText("manifest.042"), p("artifact", "pipeline"), {
                      required: true,
                    })}
                    {text(hostRepositoryText("manifest.043"), p("artifact", "path"), {
                      required: true,
                    })}
                    {workspace ? (
                      <RunPicker workspace={workspace} pipeline={String(get(p("artifact", "pipeline")) ?? "")} readOnly={readOnly}
                        value={String(get(p("artifact", "run")) ?? "")} onChange={next => set(p("artifact", "run"), next || undefined)} />
                    ) : text(hostRepositoryText("manifest.044"), p("artifact", "run"), {
                      hint: hostRepositoryText("manifest.045"),
                    })}
                  </div>
                )}
                {rows(
                  hostRepositoryText("manifest.046"),
                  p("ports"),
                  { container: 80, protocol: "tcp" },
                  (row) => (
                    <>
                      {text(hostRepositoryText("manifest.047"), [...row, "container"], {
                        number: true,
                        min: 1,
                        max: 65535,
                        required: true,
                      })}
                      {text(hostRepositoryText("manifest.048"), [...row, "published"], {
                        number: true,
                        min: 1,
                        max: 65535,
                      })}
                      {text(hostRepositoryText("manifest.049"), [...row, "name"])}
                      {select(
                        hostRepositoryText("manifest.050"),
                        [...row, "protocol"],
                        ["tcp", "udp"],
                        "tcp",
                      )}
                    </>
                  ),
                )}
                {rows(
                  hostRepositoryText("manifest.051"),
                  p("env"),
                  { name: "", value: "" },
                  (row, environment) => (
                    <>
                      {text(hostRepositoryText("manifest.052"), [...row, "name"], { required: true })}
                      <AutoField label={hostRepositoryText("manifest.053")} className="flex flex-col gap-1.5 text-sm">
                        <select
                          aria-label={hostRepositoryText("manifest.053")}
                          className="h-9 rounded-md border border-input bg-background px-2"
                          disabled={readOnly}
                          value={
                            environment.valueFrom !== undefined
                              ? "reference"
                              : "literal"
                          }
                          onChange={(e) =>
                            change([
                              {
                                path: [...row, "value"],
                                value:
                                  e.target.value === "literal" ? "" : undefined,
                              },
                              {
                                path: [...row, "valueFrom"],
                                value:
                                  e.target.value === "reference"
                                    ? "variable/"
                                    : undefined,
                              },
                            ])
                          }
                        >
                          <option value="literal">{hostRepositoryText("manifest.054")}</option>
                          <option value="reference">{hostRepositoryText("manifest.055")}</option>
                        </select>
                      </AutoField>
                      {environment.valueFrom !== undefined
                        ? text(hostRepositoryText("manifest.056"), [...row, "valueFrom"], {
                            required: true,
                            hint: hostRepositoryText("manifest.057"),
                          })
                        : text(hostRepositoryText("manifest.058"), [...row, "value"], {
                            required: true,
                            multiline: true,
                          })}
                    </>
                  ),
                )}
                {rows(hostRepositoryText("manifest.059"), p("volumes"), { name: "", path: "" }, (row) => (
                  <>
                    {text(hostRepositoryText("manifest.060"), [...row, "name"], { required: true })}
                    {text(hostRepositoryText("manifest.061"), [...row, "path"], { required: true })}
                  </>
                ))}
                {rows(
                  hostRepositoryText("manifest.062"),
                  p("files"),
                  { path: "", fromTemplate: "" },
                  (row, file) => (
                    <>
                      {text(hostRepositoryText("manifest.063"), [...row, "path"], {
                        required: true,
                      })}
                      {text(
                        hostRepositoryText("manifest.064"),
                        [...row, "fromTemplate"],
                        { required: true },
                      )}
                      <JsonField
                        fieldId={JSON.stringify(row)}
                        report={reportJson}
                        label={hostRepositoryText("manifest.065")}
                        value={file.values ?? {}}
                        readOnly={readOnly}
                        onChange={(value) => set([...row, "values"], value)}
                      />
                    </>
                  ),
                )}
                <details className="space-y-4">
                  <summary className="cursor-pointer text-sm font-medium">
                    {hostRepositoryText("manifest.066")}</summary>
                  <section aria-label={hostRepositoryText("manifest.067")} className="space-y-2">
                    <h4 className="text-sm font-medium">{hostRepositoryText("manifest.068")}</h4>
                    <p className="text-xs text-muted-foreground">
                      {hostRepositoryText("manifest.069")}</p>
                    {((service.command ?? []) as string[]).map((_, i) => (
                      <div key={i} className="flex items-end gap-2">
                        <div className="min-w-0 flex-1">
                          {text(hostRepositoryText("manifest.070", {value1: i + 1}), p("command", i), {
                            required: true,
                          })}
                        </div>
                        {!readOnly && (
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            aria-label={hostRepositoryText("manifest.071", {value1: i + 1})}
                            title={hostRepositoryText("manifest.071", {value1: i + 1})}
                            onClick={() => setPendingRemoval({ path: p("command"), index: i, action: hostRepositoryText("manifest.072"), target: hostRepositoryText("manifest.073", {value1: String(service.name || hostRepositoryText("manifest.026")), value2: i + 1}) })}
                          >
                            <Trash2 className="size-4" />
                          </Button>
                        )}
                      </div>
                    ))}
                    {!readOnly && (
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => {
                          if (service.command == null) set(p("command"), [""]);
                          else append(p("command"), "");
                        }}
                      >
                        {hostRepositoryText("manifest.074")}</Button>
                    )}
                  </section>
                  <fieldset className="space-y-2">
                    <legend className="text-sm font-medium">{hostRepositoryText("manifest.075")}</legend>
                    {mode === "stack" && (
                      <p className="text-xs text-muted-foreground">
                        {hostRepositoryText("manifest.076")}</p>
                    )}
                    {[
                      ...new Set([
                        ...services
                          .filter((_, i) => i !== index)
                          .map((s) => String(s.name)),
                        ...dependencies,
                      ]),
                    ].map((name) => (
                      <label
                        key={name}
                        className="mr-4 inline-flex items-center gap-2 text-sm"
                      >
                        <Checkbox
                          disabled={readOnly}
                          checked={dependencies.includes(name)}
                          onCheckedChange={(checked) =>
                            set(
                              p("dependsOn"),
                              checked
                                ? [...dependencies, name]
                                : dependencies.filter((n) => n !== name),
                            )
                          }
                        />
                        {name}
                      </label>
                    ))}
                  </fieldset>
                  {(
                    [
                      ["readiness", hostRepositoryText("manifest.077")],
                      ["liveness", hostRepositoryText("manifest.078")],
                    ] as const
                  ).map(([key, label]) => {
                    const healthPath = p("health", key);
                    const enabled = plain(healthPath) != null;
                    return (
                      <section key={key} className="space-y-3">
                        <label className="flex items-center gap-2 text-sm">
                          <Checkbox
                            disabled={readOnly}
                            checked={enabled}
                            onCheckedChange={(checked) =>
                              set(
                                healthPath,
                                checked
                                  ? {
                                      httpGet: { path: "/", port: 80 },
                                      periodSeconds: 10,
                                      timeoutSeconds: 1,
                                      failureThreshold: 3,
                                    }
                                  : undefined,
                              )
                            }
                          />
                          {label}（HTTP）
                        </label>
                        {enabled && (
                          <div className={grid}>
                            {text(
                              hostRepositoryText("manifest.079", {value1: label}),
                              [...healthPath, "httpGet", "path"],
                              { required: true },
                            )}
                            <AutoField label={hostRepositoryText("manifest-extra.004", {value1: label})} className="flex flex-col gap-1.5 text-sm">
                              <Input
                                disabled={readOnly}
                                value={String(
                                  get([...healthPath, "httpGet", "port"]) ?? "",
                                )}
                                onChange={(e) =>
                                  set(
                                    [...healthPath, "httpGet", "port"],
                                    /^\d+$/.test(e.target.value)
                                      ? Number(e.target.value)
                                      : e.target.value,
                                  )
                                }
                              />
                            </AutoField>
                            {text(
                              hostRepositoryText("manifest.081", {value1: label}),
                              [...healthPath, "periodSeconds"],
                              { number: true, min: 1, max: 3600, fallback: 10 },
                            )}
                            {text(
                              hostRepositoryText("manifest.082", {value1: label}),
                              [...healthPath, "timeoutSeconds"],
                              { number: true, min: 1, max: 600, fallback: 1 },
                            )}
                            {text(
                              hostRepositoryText("manifest.083", {value1: label}),
                              [...healthPath, "failureThreshold"],
                              { number: true, min: 1, max: 100, fallback: 3 },
                            )}
                          </div>
                        )}
                      </section>
                    );
                  })}
                  <section className="space-y-3">
                    <h4 className="text-sm font-medium">{hostRepositoryText("manifest.084")}</h4>
                    <div className={grid}>
                      {(
                        [
                          ["requests", hostRepositoryText("manifest.085")],
                          ["limits", hostRepositoryText("manifest.086")],
                        ] as const
                      ).map(([key, label]) => (
                        <div key={key} className="space-y-3">
                          {text(`CPU ${label}`, p("resources", key, "cpu"), {
                            hint: hostRepositoryText("manifest.087"),
                          })}
                          {text(hostRepositoryText("manifest.088", {value1: label}), p("resources", key, "memory"), {
                            hint: hostRepositoryText("manifest.089"),
                          })}
                        </div>
                      ))}
                    </div>
                  </section>
                </details>
              </section>
            );
          })}
          {!readOnly && (
            <Button
              type="button"
              variant="outline"
              disabled={invalidJson || services.length >= 50}
              onClick={() => {
                let number = services.length + 1;
                while (services.some((s) => s.name === `service-${number}`))
                  number++;
                append(["services"], {
                  name: `service-${number}`,
                  ...(mode === "native"
                    ? {
                        artifact: {
                          source: "pipeline",
                          pipeline: "",
                          path: "",
                        },
                      }
                    : { image: "" }),
                });
              }}
            >
              {hostRepositoryText("manifest.090")}</Button>
          )}
          <details className="space-y-3">
            <summary className="cursor-pointer text-sm font-medium">
              {hostRepositoryText("manifest.091")}</summary>
            <div className={grid}>
              {select(
                hostRepositoryText("manifest.092"),
                ["strategy", "type"],
                mode === "stack" ? ["recreate", "rolling"] : ["recreate"],
                "recreate",
              )}
              {text(hostRepositoryText("manifest.093"), ["strategy", "healthTimeout"], {
                number: true,
                min: 1,
                max: 600,
                fallback: 120,
              })}
              {mode === "stack" &&
                text(hostRepositoryText("manifest.094"), ["strategy", "maxUnavailable"], {
                  number: true,
                  min: 0,
                  max: 100,
                })}
            </div>
            <JsonField
              fieldId="labels"
              report={reportJson}
              label={hostRepositoryText("manifest.095")}
              value={plain(["metadata", "labels"]) ?? {}}
              readOnly={readOnly}
              onChange={(value) => set(["metadata", "labels"], value)}
            />
            <p className="text-xs text-muted-foreground">
              {hostRepositoryText("manifest.096")}</p>
            <JsonField
              fieldId="overrides"
              report={reportJson}
              label={hostRepositoryText("manifest.097")}
              value={plain(["overrides"]) ?? {}}
              readOnly={readOnly}
              onChange={(value) => set(["overrides"], value)}
            />
          </details>
        </>
      )}
      {notice && (
        <p role="status" className="text-sm">
          {notice}
        </p>
      )}
    </section>
  );
}

function JsonField({
  label,
  fieldId,
  value,
  readOnly,
  onChange,
  report,
}: {
  label: string;
  fieldId: string;
  value: unknown;
  readOnly: boolean;
  onChange: (value: Row) => void;
  report: (key: string, invalid: boolean) => void;
}) {
  useCommonTranslation();
  const serialized = JSON.stringify(value, null, 2);
  const [text, setText] = useState(serialized);
  const [error, setError] = useState("");
  const sent = useRef(serialized);
  useEffect(() => {
    if (sent.current !== serialized) {
      setText(serialized);
      setError("");
      sent.current = serialized;
    }
  }, [serialized]);
  useEffect(() => {
    report(fieldId, !!error);
    return () => report(fieldId, false);
  }, [fieldId, error, report]);
  return (
    <AutoField label={<>{label}{hostRepositoryText("manifest.099")}</>} className="flex min-w-0 flex-col gap-1.5 text-sm sm:col-span-2">
      <textarea
        className={area}
        aria-label={hostRepositoryText("manifest.098", {value1: label})}
        value={text}
        readOnly={readOnly}
        aria-invalid={!!error}
        ref={(element) => element?.setCustomValidity(error)}
        onChange={(e) => {
          setText(e.target.value);
          try {
            const result: unknown = JSON.parse(e.target.value);
            if (!object(result)) throw new Error(hostRepositoryText("manifest.100"));
            setError("");
            sent.current = JSON.stringify(result, null, 2);
            onChange(result);
          } catch {
            setError(hostRepositoryText("manifest.101"));
          }
        }}
      />
      {error && (
        <span role="alert" className="text-destructive">
          {error}
        </span>
      )}
    </AutoField>
  );
}
