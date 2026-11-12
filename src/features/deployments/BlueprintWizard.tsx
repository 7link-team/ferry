import { lazy, Suspense, useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { cn } from "cn";
import { Minus, Plus } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { base, named } from "../../shared/api/resource";
import { AutoField, FieldControl } from "../../shared/forms/auto-form";
import { FormActions } from "../../shared/forms/auto-form/form-actions";
import { commonText, useCommonTranslation } from "../../shared/i18n/i18n";
import { namedOptions } from "../../shared/resources/config";
import type { Option } from "../../shared/resources/types";
import type { Navigation } from "../../shared/routing/types";
import { Badge } from "../../shared/ui/badge";
import { Button } from "../../shared/ui/button";
import { AutoTable } from "../../shared/tables/auto-table";
import { Input } from "../../shared/ui/input";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { tagOptions } from "../tags/resource-config";
import { templateParameterFields, type TemplateParameter } from "../templates/TemplateParametersEditor";
import {
  initialSpec, instancesProblem, readBlueprint, readSpec, stepInstances, topologyKey, writeSpec,
  type BlueprintSummary, type InstanceBinding, type SpecChange,
} from "./blueprint-spec";
import { allHosts, type HostRow } from "./blueprint-hooks";
import { deploymentDeployed, type Deployment } from "./deployment";

const SpecEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));
const w = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:wizard.${key}`, values);
const STEPS = ["stepTemplate", "stepValues", "stepRoles", "stepPlacement", "stepConfirm"] as const;
const DRIVER_KEYS: Record<string, string> = { native: "driverNative", docker: "driverDocker", compose: "driverCompose", stack: "driverStack" };
const driverLabel = (driver: string) => DRIVER_KEYS[driver] ? w(DRIVER_KEYS[driver]) : driver;
const selectClass = "min-h-11 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm lg:min-h-9";
const touch = "min-h-11 lg:min-h-9";

interface Template { id: string; name: string; content: string; parameters: string; version: number }
interface Placement { manifest: string; binding: { driver: string; manager?: string; roles: Record<string, InstanceBinding[]> }; warnings: { key: string; message: string }[] }
interface PlacementRow { instance: string; role: string; index: number; hostId: string; address: string }
interface Loaded { template: Template; blueprint: BlueprintSummary | undefined; parameters: TemplateParameter[] }

/** Instances in blueprint role order, named like the compiled instances (`<role>-<index>`). */
const placementRows = (blueprint: BlueprintSummary, result: Placement): PlacementRow[] =>
  blueprint.roles.flatMap(role => (result.binding.roles[role.name] ?? []).map((item, index) => ({ instance: `${role.name}-${index}`, role: role.name, index, ...item })));

function readParameters(text: string): TemplateParameter[] {
  try { const rows: unknown = JSON.parse(text); return Array.isArray(rows) ? rows.filter(row => row && typeof row.name === "string" && typeof row.type === "string") : []; }
  catch { return []; }
}

/**
 * Multi-step creation of a blueprint deployment; the YAML spec is the single source of truth.
 * With `deployment` it edits that deployment's stored spec instead: every step before its
 * first deploy, afterwards only the parameters (the server freezes the topology).
 */
export function BlueprintWizard({ workspace, templateId, version, permissions, navigation, deployment, onDone, onCancel }: {
  workspace: string; templateId: string; version: number; permissions: Set<string>; navigation: Navigation;
  /** A snapshot of the deployment being edited; keep it stable while the wizard is open. */
  deployment?: Deployment; onDone?: () => void; onCancel?: () => void;
}) {
  useCommonTranslation();
  const editorId = useId();
  const editing = !!deployment;
  const frozen = !!deployment && deploymentDeployed(deployment);
  const steps: readonly (typeof STEPS)[number][] = frozen ? ["stepValues"] : STEPS;
  const allowed = permissions.has(editing ? "deployments.update" : "deployments.create");
  const [loaded, setLoaded] = useState<Loaded>();
  const [loadError, setLoadError] = useState("");
  const [reload, setReload] = useState(0);
  const [text, setText] = useState("");
  const [lastValid, setLastValid] = useState("");
  const [mode, setMode] = useState<"form" | "yaml">("form");
  const [step, setStep] = useState(0);
  const [hosts, setHosts] = useState<HostRow[]>();
  const [tags, setTags] = useState<Option[]>([]);
  const [groups, setGroups] = useState<Option[]>([]);
  const [optionsError, setOptionsError] = useState("");
  const [placement, setPlacement] = useState<{ for: string; busy: boolean; result?: Placement; error?: string }>();
  const placementRequest = useRef<AbortController | undefined>(undefined);
  const [name, setName] = useState("");
  const [workdir, setWorkdir] = useState("/opt/deployments");
  const [creating, setCreating] = useState(false);
  const [submitError, setSubmitError] = useState("");

  const change = useCallback((next: string) => {
    setText(next);
    if (!readSpec(next).errors.length) setLastValid(next);
  }, []);

  useEffect(() => {
    if (!allowed || !templateId) return;
    const controller = new AbortController();
    setLoadError(""); setLoaded(undefined);
    api<Template>(`${base(workspace, "app-templates")}/${encodeURIComponent(templateId)}`, { signal: controller.signal }).then(template => {
      const blueprint = readBlueprint(template.content);
      const parameters = readParameters(template.parameters);
      setLoaded({ template, blueprint, parameters });
      if (!blueprint) return;
      if (deployment) {
        setName(deployment.name); setWorkdir(deployment.workdir); change(deployment.deploy_spec ?? "");
        return;
      }
      setName(blueprint.name || template.name);
      change(initialSpec({
        template: { id: template.id, version },
        driver: blueprint.drivers[0] ?? "",
        values: Object.fromEntries(parameters.filter(p => p.default !== undefined).map(p => [p.name, p.default])),
        roles: Object.fromEntries(blueprint.roles.map(role => [role.name, { instances: role.instances.default }])),
      }));
    }).catch(error => { if (!controller.signal.aborted) setLoadError(errorText(error)); });
    return () => controller.abort();
  }, [workspace, templateId, version, allowed, reload, change, deployment]);

  useEffect(() => {
    if (!allowed) return;
    let live = true;
    setOptionsError("");
    Promise.all([allHosts(workspace), tagOptions(workspace)(), namedOptions(workspace, "host-groups")()]).then(([hostRows, tagRows, groupRows]) => {
      if (!live) return;
      setHosts(hostRows);
      // Selectors match tag and group names, not IDs.
      setTags(tagRows.map(tag => ({ ...tag, value: tag.label })));
      setGroups(groupRows.map(group => ({ value: group.label, label: group.label })));
    }).catch(error => { if (live) setOptionsError(errorText(error)); });
    return () => { live = false; };
  }, [workspace, allowed, reload]);
  useEffect(() => () => placementRequest.current?.abort(), []);

  const current = useMemo(() => readSpec(text), [text]);
  const form = useMemo(() => readSpec(lastValid), [lastValid]);
  const spec = form.spec;
  const invalid = current.errors.length > 0;
  const stored = useMemo(() => frozen ? readSpec(deployment?.deploy_spec ?? "").spec : undefined, [frozen, deployment]);
  // After the first deploy only `values` and `template.version` may change; caught before submit.
  const locked = !!stored && !!current.spec && topologyKey(current.spec) !== topologyKey(stored);
  const key = steps[step] ?? steps[0]!;
  // While the YAML is invalid the form is locked: it never overwrites text the user is fixing.
  const patch = (changes: SpecChange[]) => { if (!invalid && form.doc) change(writeSpec(form.doc, changes)); };
  /** Removes a role's manual placement; the whole key goes when no role is left. */
  const dropPlacement = (role: string): SpecChange[] => !spec?.placement?.[role] ? []
    : [{ path: Object.keys(spec.placement).length > 1 ? ["placement", role] : ["placement"], value: undefined }];

  const preview = useCallback(async (source: string) => {
    placementRequest.current?.abort();
    const controller = new AbortController(); placementRequest.current = controller;
    setPlacement({ for: source, busy: true });
    try {
      const result = await api<Placement>(base(workspace, "deployment-placements"), { method: "POST", body: JSON.stringify({ spec: source }), signal: controller.signal });
      if (!controller.signal.aborted) setPlacement({ for: source, busy: false, result });
    } catch (error) { if (!controller.signal.aborted) setPlacement({ for: source, busy: false, error: errorText(error) }); }
  }, [workspace]);
  // The form recalculates on every change; YAML typing waits for an explicit recalculation.
  useEffect(() => {
    if (key === "stepPlacement" && mode === "form" && !invalid && text && placement?.for !== text) void preview(text);
  }, [key, mode, invalid, text, placement?.for, preview]);

  // Editing is embedded in the detail page: every early state keeps a way back.
  const cancel = onCancel && <Button type="button" variant="outline" className={touch} onClick={onCancel}>{w("cancel")}</Button>;
  const early = (body: ReactNode) => cancel ? <div className="space-y-3">{body}<div>{cancel}</div></div> : body;
  if (!allowed) return early(<p role="alert">{w("forbidden")}</p>);
  if (!templateId) return early(<p role="alert">{w("missingTemplate")}</p>);
  if (loadError) return <div role="alert" className="space-y-3"><p>{loadError}</p><div className="flex flex-wrap gap-2"><Button variant="outline" className={touch} onClick={() => setReload(n => n + 1)}>{commonText("common-domain:common.retry")}</Button>{cancel}</div></div>;
  if (!loaded) return early(<LoadingSkeleton variant="page" label={w("loading")} />);
  const { template, blueprint, parameters } = loaded;
  if (!blueprint) return early(<p role="alert">{w("notBlueprint")}</p>);

  const problems = (at: string): string[] => {
    if (!spec) return [];
    if (at === "stepTemplate") return spec.driver ? [] : [w("noDrivers")];
    if (at === "stepValues") return parameters.filter(p => p.default === undefined && (spec.values[p.name] === undefined || spec.values[p.name] === ""))
      .map(p => commonText("form.required", { name: p.label || p.name }));
    if (at === "stepRoles") return blueprint.roles.flatMap(role => {
      const count = spec.roles[role.name]?.instances;
      if (count === undefined) return [w("roleMissing", { role: role.name })];
      const problem = instancesProblem(count, role.instances);
      return problem ? [w(problem === "odd" ? "instancesOdd" : "instancesRange", { role: role.name, min: role.instances.min, max: role.instances.max })] : [];
    }).concat(spec.driver === "stack" && !spec.manager ? [commonText("form.required", { name: w("manager") })] : []);
    if (at === "stepPlacement") return placement?.for === text && placement.result ? [] : [placement?.error ?? w("placementStale")];
    return [!name.trim() && commonText("form.required", { name: commonText("common-domain:common.name") }), !workdir.trim() && commonText("form.required", { name: commonText("resource-domain:deployment.workdir") })].filter((item): item is string => !!item);
  };
  const blocked = invalid || locked || problems(key).length > 0;
  const submit = async () => {
    if (blocked || creating) return;
    setCreating(true); setSubmitError("");
    try {
      if (deployment) {
        await api(`${base(workspace, "deployments")}/${encodeURIComponent(deployment.id)}`, { method: "PATCH", body: JSON.stringify({ name: name.trim(), category: deployment.category ?? "", description: deployment.description, workdir: deployment.workdir, version: deployment.version, deploy_spec: text }) });
        onDone?.();
        return;
      }
      const created = await api<{ id: string }>(base(workspace, "deployments"), { method: "POST", body: JSON.stringify({ name: name.trim(), category: "", description: "", workdir: workdir.trim(), deploy_spec: text }) });
      navigation.navigate({ group: "deploy", item: "apps", id: created.id, mode: "view" }, {});
    } catch (error) { setSubmitError(errorText(error)); }
    finally { setCreating(false); }
  };

  const hostName = (id: string) => named(hosts?.find(h => h.id === id)?.name, id);
  const binding = (hostId: string): InstanceBinding => ({ hostId, address: hosts?.find(h => h.id === hostId)?.address ?? "" });
  const choose = (role: string, index: number, hostId: string) => {
    const list = (placement?.result?.binding.roles[role] ?? []).map(item => binding(item.hostId));
    const previous = list[index]?.hostId;
    // One instance per host: the instance already on the chosen host takes the old one (a swap).
    const other = blueprint.roles.find(r => r.name === role)?.distinctHosts ? list.findIndex((item, i) => i !== index && item.hostId === hostId) : -1;
    if (other >= 0 && previous !== undefined) list[other] = binding(previous);
    list[index] = binding(hostId);
    patch([{ path: ["placement", role], value: list }]);
  };
  const automatic = (role: string) => patch(dropPlacement(role));

  const versionChanged = template.version !== (spec?.template.version ?? version);
  return <div className="flex min-h-0 min-w-0 flex-col">
    <div className="flex min-w-0 flex-wrap items-start justify-between gap-3">
      <h2 className="min-w-0 text-lg font-semibold break-words">{w(editing ? "editTitle" : "title")}</h2>
      <div role="group" aria-label={w("editMode")} className="flex gap-1">
        <Button type="button" className={touch} variant={mode === "form" ? "secondary" : "outline"} aria-pressed={mode === "form"} onClick={() => setMode("form")}>{w("form")}</Button>
        <Button type="button" className={touch} variant={mode === "yaml" ? "secondary" : "outline"} aria-pressed={mode === "yaml"} onClick={() => setMode("yaml")}>{w("yaml")}</Button>
      </div>
    </div>
    <ol aria-label={w("steps")} className="mt-3 flex min-w-0 flex-wrap gap-x-4 gap-y-2 border-b pb-3">
      {steps.map((label, index) => <li key={label} aria-current={index === step ? "step" : undefined} className={cn("flex min-w-0 items-center gap-2 text-sm", index === step ? "font-medium text-foreground" : "text-muted-foreground")}>
        <span aria-hidden className={cn("grid size-7 shrink-0 place-items-center rounded-full border text-xs", index === step && "border-primary bg-primary text-primary-foreground", index < step && "border-primary text-primary")}>{index + 1}</span>
        <span className={cn("min-w-0 break-words", index !== step && "sr-only md:not-sr-only")}>{w(label)}</span>
      </li>)}
    </ol>
    <p className="sr-only" aria-live="polite">{w("stepOf", { current: step + 1, total: steps.length })}</p>

    <div className="min-w-0 flex-1 space-y-4 py-4">
      {frozen && <p className="rounded-md border bg-muted/30 p-3 text-sm text-muted-foreground">{w("frozenHelp")}</p>}
      {locked && <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">{w("topologyLocked")}</p>}
      {invalid && <div role="alert" className="space-y-2 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">
        {mode === "form" && <p>{w("yamlInvalid")}</p>}
        {current.errors.map((error, index) => <p key={index} className="break-words">{error}</p>)}
        {lastValid && <Button type="button" size="sm" variant="outline" className={touch} onClick={() => setText(lastValid)}>{w("discardYaml")}</Button>}
      </div>}
      {versionChanged && spec && <div role="alert" className="flex flex-wrap items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/5 p-3 text-sm">
        <span>{w("versionChanged", { current: template.version, requested: spec.template.version })}</span>
        <Button type="button" size="sm" variant="outline" className={touch} disabled={invalid} onClick={() => patch([{ path: ["template", "version"], value: template.version }])}>{w("useLatest", { current: template.version })}</Button>
      </div>}
      {optionsError && <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive"><span>{w("optionsFailed", { detail: optionsError })}</span><Button type="button" variant="outline" size="sm" className={touch} onClick={() => setReload(n => n + 1)}>{commonText("common-domain:common.retry")}</Button></div>}

      {mode === "yaml" ? <AutoField label={w("yamlLabel")} help={w("yamlHelp")}>
        <div className="h-[60vh] min-h-72 min-w-0 overflow-hidden rounded-md border lg:h-[28rem]">
          <Suspense fallback={<LoadingSkeleton variant="editor" />}><SpecEditor value={text} language="yaml" path={`deploy-spec-${editorId}.yaml`} label={w("yamlLabel")} onChange={change} /></Suspense>
        </div>
      </AutoField> : spec && <fieldset disabled={invalid} className="m-0 min-w-0 space-y-4 border-0 p-0">
        {key === "stepTemplate" && <section className="space-y-4" aria-label={w("stepTemplate")}>
          <dl className="grid min-w-0 gap-3 text-sm sm:grid-cols-2">
            <div className="min-w-0"><dt className="text-muted-foreground">{w("templateName")}</dt><dd className="break-words font-medium">{template.name}</dd></div>
            <div><dt className="text-muted-foreground">{w("templateVersion")}</dt><dd className="font-medium">v{spec.template.version}</dd></div>
          </dl>
          <AutoField label={w("driver")} required help={blueprint.drivers.length ? undefined : w("noDrivers")}>
            <select className={selectClass} value={spec.driver} required disabled={invalid} onChange={event => patch([{ path: ["driver"], value: event.target.value }, ...(event.target.value === "stack" ? [] : [{ path: ["manager"], value: undefined }])])}>
              {!blueprint.drivers.includes(spec.driver) && <option value={spec.driver} disabled>{spec.driver || commonText("common.select")}</option>}
              {blueprint.drivers.map(driver => <option key={driver} value={driver}>{driverLabel(driver)}</option>)}
            </select>
          </AutoField>
          <div className="min-w-0 space-y-2">
            <h3 className="text-sm font-medium">{w("roles")}</h3>
            <ul className="grid min-w-0 gap-2 md:grid-cols-2">{blueprint.roles.map(role => <li key={role.name} className="min-w-0 rounded-md border p-3 text-sm">
              <div className="flex min-w-0 flex-wrap items-center gap-2"><span className="break-all font-medium">{role.name}</span><Badge variant="outline">{w(role.identity === "stable" ? "identityStable" : "identityStateless")}</Badge></div>
              <p className="mt-1 text-muted-foreground">{w("instanceRange")}: {w("rangeValue", { min: role.instances.min, max: role.instances.max, value: role.instances.default })}{role.instances.odd ? ` · ${w("instancesHintOdd", { min: role.instances.min, max: role.instances.max })}` : ""}</p>
              {(role.distinctHosts || role.colocateWith) && <p className="text-muted-foreground">{[role.distinctHosts && w("distinctHosts"), role.colocateWith && w("colocate", { role: role.colocateWith })].filter(Boolean).join(" · ")}</p>}
            </li>)}</ul>
          </div>
        </section>}

        {key === "stepValues" && <section className="space-y-4" aria-label={w("stepValues")}>
          {!parameters.length && <p className="text-sm text-muted-foreground">{commonText("resource-domain:template.noParameters")}</p>}
          <div className="grid min-w-0 gap-4 md:grid-cols-2">{templateParameterFields(parameters).map((field, index) => {
            const parameter = parameters[index]!;
            const value = spec.values[field.key];
            return <AutoField key={field.key} label={field.label} help={field.help} required={!!field.required}>
              <FieldControl name={field.key} required={!!field.required} disabled={invalid}
                field={{ name: field.label, type: field.type === "select" ? "select" : field.type === "number" ? "number" : "text", config: { ...(Array.isArray(field.options) ? { options: field.options } : {}), ...(parameter.min === undefined ? {} : { min: parameter.min }), ...(parameter.max === undefined ? {} : { max: parameter.max }) } }}
                value={value === undefined ? "" : String(value)}
                onChange={next => patch([{ path: ["values", field.key], value: parameter.type === "boolean" ? (next === "" ? undefined : next === "true") : next }])} />
            </AutoField>;
          })}</div>
        </section>}

        {key === "stepRoles" && <section className="space-y-4" aria-label={w("stepRoles")}>
          {spec.driver === "stack" && <AutoField label={w("manager")} help={w("managerHelp")} required>
            <select className={selectClass} value={spec.manager ?? ""} required disabled={invalid} onChange={event => patch([{ path: ["manager"], value: event.target.value }])}>
              <option value="">{commonText("common.select")}</option>
              {spec.manager && !hosts?.some(h => h.name === spec.manager) && <option value={spec.manager}>{spec.manager}</option>}
              {hosts?.map(h => <option key={h.id} value={h.name}>{h.name}</option>)}
            </select>
          </AutoField>}
          {blueprint.roles.map(role => {
            const entry = spec.roles[role.name];
            const count = entry?.instances ?? role.instances.default;
            const selector = entry?.hosts ?? {};
            const problem = entry ? instancesProblem(count, role.instances) : undefined;
            // Instances and the selector decide where instances may run, so they void a manual placement.
            const set = (key: string, value: unknown) => patch([{ path: ["roles", role.name, "hosts", key], value }, ...(key === "spread" ? [] : dropPlacement(role.name))]);
            const resize = (delta: 1 | -1) => patch([{ path: ["roles", role.name, "instances"], value: stepInstances(count, delta, role.instances) }, ...dropPlacement(role.name)]);
            return <fieldset key={role.name} className="min-w-0 space-y-4 rounded-md border p-3 sm:p-4">
              <legend className="break-all px-1 text-sm font-medium">{w("role")}: {role.name}</legend>
              <AutoField label={w("instances")} help={w(role.instances.odd ? "instancesHintOdd" : "instancesHint", { min: role.instances.min, max: role.instances.max })}
                error={problem ? w(problem === "odd" ? "instancesOdd" : "instancesRange", { role: role.name, min: role.instances.min, max: role.instances.max }) : undefined}>
                <div className="flex items-center gap-2">
                  <Button type="button" variant="outline" size="icon" className="size-11 lg:size-9" aria-label={w("decrease", { role: role.name })} disabled={invalid || stepInstances(count, -1, role.instances) === count} onClick={() => resize(-1)}><Minus /></Button>
                  <output aria-label={w("instances")} className="min-w-10 text-center text-base font-medium tabular-nums">{count}</output>
                  <Button type="button" variant="outline" size="icon" className="size-11 lg:size-9" aria-label={w("increase", { role: role.name })} disabled={invalid || stepInstances(count, 1, role.instances) === count} onClick={() => resize(1)}><Plus /></Button>
                </div>
              </AutoField>
              <div className="grid min-w-0 gap-4 md:grid-cols-2">
                <AutoField label={w("tags")} help={tags.length ? w("tagsHelp") : w("noTags")}>
                  <FieldControl name={`${role.name}-tags`} field={{ name: w("tags"), type: "multi-select", config: { options: tags } }} value={selector.tags ?? []} disabled={invalid} onChange={value => set("tags", value)} />
                </AutoField>
                <AutoField label={w("names")} help={w("namesHelp")}>
                  <FieldControl name={`${role.name}-names`} field={{ name: w("names"), type: "multi-select", config: { options: (hosts ?? []).map(h => ({ value: h.name, label: h.name })) } }} value={selector.names ?? []} disabled={invalid} onChange={value => set("names", value)} />
                </AutoField>
                <AutoField label={w("group")}>
                  <select className={selectClass} value={selector.group ?? ""} disabled={invalid} onChange={event => set("group", event.target.value)}>
                    <option value="">{commonText("common.notSet")}</option>
                    {selector.group && !groups.some(g => g.value === selector.group) && <option value={selector.group}>{selector.group}</option>}
                    {groups.map(g => <option key={g.value} value={g.value}>{g.label}</option>)}
                  </select>
                </AutoField>
                <AutoField label={w("spread")} help={w("spreadHelp")}>
                  <Input className={touch} maxLength={30} disabled={invalid} value={selector.spread?.prefix ?? ""} onChange={event => set("spread", event.target.value ? { prefix: event.target.value } : undefined)} />
                </AutoField>
              </div>
            </fieldset>;
          })}
        </section>}

        {key === "stepPlacement" && <section className="space-y-4" aria-label={w("stepPlacement")}>
          <div className="flex flex-wrap items-center gap-2">
            <Button type="button" variant="outline" className={touch} disabled={invalid || placement?.busy} onClick={() => void preview(text)}>{w("refreshPlacement")}</Button>
            {placement?.busy && <span role="status" className="text-sm text-muted-foreground">{w("previewing")}</span>}
          </div>
          {Object.keys(spec.placement ?? {}).length > 0 && <div className="flex flex-wrap gap-2">{Object.keys(spec.placement ?? {}).map(role => <Button key={role} type="button" variant="outline" size="sm" className={touch} disabled={invalid} onClick={() => automatic(role)}>{w("autoPlacement")} · {role}</Button>)}</div>}
          {placement?.error && placement.for === text && <p role="alert" className="break-words text-sm text-destructive">{placement.error}</p>}
          {placement?.for !== text && !placement?.busy && <p className="text-sm text-muted-foreground">{w("placementStale")}</p>}
          {placement?.result && <>
            <p className="text-sm text-muted-foreground">{w("addressHelp")}</p>
            <div role="region" aria-label={w("placementTitle")} className="min-w-0 rounded-md border">
              <AutoTable<PlacementRow> data={placementRows(blueprint, placement.result)} rowKey={row => row.instance} pagination={false} minWidth={480} layout="auto" columns={[
                { field: "instance", name: w("instance"), render: (_, row) => <span className="inline-flex min-w-0 flex-wrap items-center gap-2"><span className="break-all">{row.instance}</span>{spec.placement?.[row.role] && row.index === 0 && <Badge variant="outline">{w("manual")}</Badge>}</span> },
                { field: "hostId", name: w("host"), render: (_, row) => <select aria-label={w("hostOf", { instance: row.instance })} className={selectClass} value={row.hostId} disabled={invalid || placement.busy} onChange={event => choose(row.role, row.index, event.target.value)}>
                  {!hosts?.some(h => h.id === row.hostId) && <option value={row.hostId}>{hostName(row.hostId)}</option>}
                  {hosts?.map(h => <option key={h.id} value={h.id}>{h.name}</option>)}
                </select> },
                { field: "address", name: w("address"), render: (_, row) => <span className="font-mono break-all">{row.address}</span> },
              ]} />
            </div>
            {placement.result.warnings.length > 0 && <div className="rounded-md border border-amber-500/40 bg-amber-500/5 p-3 text-sm">
              <h3 className="font-medium">{w("warnings")}</h3>
              <ul className="mt-1 list-disc space-y-1 ps-5">{placement.result.warnings.map((warning, index) => <li key={index} className="break-words">{warning.message}</li>)}</ul>
            </div>}
          </>}
        </section>}

        {key === "stepConfirm" && <section className="space-y-4" aria-label={w("stepConfirm")}>
          <div className="grid min-w-0 gap-4 md:grid-cols-2">
            <AutoField label={commonText("common-domain:common.name")} required><Input name="name" className={touch} required disabled={invalid} maxLength={60} value={name} onChange={event => setName(event.target.value)} /></AutoField>
            <AutoField label={commonText("resource-domain:deployment.workdir")} help={commonText("resource-domain:deployment.workdirHelp")} required><Input name="workdir" className={touch} required disabled={invalid || editing} value={workdir} onChange={event => setWorkdir(event.target.value)} /></AutoField>
          </div>
          <div className="rounded-md border p-3 text-sm">
            <h3 className="font-medium">{w("summary")}</h3>
            <ul className="mt-1 space-y-1 text-muted-foreground">
              <li className="break-words">{template.name} · v{spec.template.version} · {driverLabel(spec.driver)}</li>
              {Object.entries(spec.roles).map(([role, entry]) => <li key={role} className="break-all">{w("summaryRole", { role, count: entry.instances })}</li>)}
            </ul>
          </div>
        </section>}
      </fieldset>}
      {submitError && <p role="alert" className="break-words text-sm text-destructive">{submitError}</p>}
    </div>

    <FormActions>
      {onCancel && <Button type="button" variant="outline" className={touch} disabled={creating} onClick={onCancel}>{w("cancel")}</Button>}
      {steps.length > 1 && <Button type="button" variant="outline" className={touch} disabled={step === 0 || creating} onClick={() => setStep(n => Math.max(0, n - 1))}>{w("back")}</Button>}
      <span className="ms-auto" />
      {step < steps.length - 1
        ? <Button type="button" className={touch} disabled={blocked} title={problems(key)[0]} onClick={() => setStep(n => n + 1)}>{w("next")}</Button>
        : <Button type="button" className={touch} disabled={blocked || creating} onClick={() => void submit()}>{creating ? w(editing ? "saving" : "creating") : w(editing ? "save" : "create")}</Button>}
    </FormActions>
  </div>;
}

