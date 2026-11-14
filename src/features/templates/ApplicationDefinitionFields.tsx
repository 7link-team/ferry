import { useCallback, useEffect, useState, type ReactNode } from "react";
import { commonText, hostRepositoryText } from "../../shared/i18n/i18n";
import { object, type Path, type Row } from "../deployments/manifest/model";
import { AutoField } from "../../shared/forms/auto-form";
import { Input } from "../../shared/ui/input";
import { Button } from "../../shared/ui/button";
import { definitionChoices, definitionItem, definitionShape } from "./application-definition-schema";
import { type TemplateParameter } from "./TemplateParametersEditor";
import { ReferencePicker, RunPicker, pipelineChoices } from "../deployments/manifest/pickers";

const labelKeys: Record<string, string> = {
  constraints: "deployment.placementConstraints", maxReplicasPerNode: "deployment.maxReplicasPerNode",
  apiVersion: "deployment.definitionVersion", kind: "blueprintDetails.kind", metadata: "blueprintDetails.metadata",
  allowSingleHost: "blueprintDetails.allowSingleHost", drivers: "wizard.driver", supported: "blueprintDetails.supported",
  memberAddressing: "blueprintDetails.memberAddressing", roles: "wizard.roles", identity: "wizard.identity",
  instances: "blueprintDetails.instances", min: "blueprintDetails.min", max: "blueprintDetails.max",
  default: "blueprintDetails.default", odd: "blueprintDetails.odd", placement: "blueprintDetails.placement",
  distinctHosts: "wizard.distinctHosts", colocateWith: "blueprintDetails.colocateWith", service: "blueprintDetails.service",
  endpoints: "blueprintDetails.endpoints", role: "wizard.role", port: "blueprintDetails.portReference",
  update: "blueprintDetails.update", maxParallel: "blueprintDetails.maxParallel", minHealthySeconds: "blueprintDetails.minHealthySeconds",
  healthTimeout: "blueprintDetails.healthTimeout", autoRevert: "blueprintDetails.autoRevert", hooks: "blueprintDetails.hooks",
  join: "blueprintDetails.join", leave: "blueprintDetails.leave", healthy: "blueprintDetails.healthy", failover: "blueprintDetails.failover",
  replace: "blueprintDetails.replace", strategy: "blueprintDetails.strategy", type: "blueprintDetails.type",
  cpu: "blueprintDetails.cpu", memory: "blueprintDetails.memory", httpGet: "blueprintDetails.httpGet",
  periodSeconds: "blueprintDetails.periodSeconds", timeoutSeconds: "blueprintDetails.timeoutSeconds",
  failureThreshold: "blueprintDetails.failureThreshold", maxUnavailable: "blueprintDetails.maxUnavailable",
};
const manifestKeys: Record<string, string> = {
  image: "036", ports: "046", container: "047", published: "048", protocol: "050",
  env: "051", value: "058", valueFrom: "056", volumes: "059", files: "062", fromTemplate: "064", values: "065",
  command: "068", dependsOn: "075", readiness: "077", liveness: "078", resources: "084", requests: "085", limits: "086",
  labels: "095", artifact: "035", pipeline: "042", run: "044", replicas: "032", overrides: "097",
};
const order = ["name", "identity", "instances", "placement", "ports", "service", "image", "artifact", "command", "env", "volumes", "files", "health", "resources", "failover", "hooks"];
const dictionary = (path: Path) => ["labels", "values"].includes(String(path.at(-1)));

function fieldLabel(key: string, parent: Path) {
  if (dictionary(parent)) return key;
  if (parent.at(-1) === "drivers") {
    const drivers: Record<string, string> = { native: "driverNative", docker: "driverDocker", compose: "driverCompose", stack: "driverStack" };
    const driver = drivers[key];
    return driver ? commonText(`resource-domain:wizard.${driver}`) : key === "k8s" ? "Kubernetes" : key;
  }
  if (key === "name") return commonText("common.name");
  if (key === "services") return commonText("resource-domain:deployment.servicesSection");
  if (parent.at(-1) === "artifact" && key === "path") return hostRepositoryText("manifest.043");
  if (parent.at(-1) === "artifact" && key === "source") return hostRepositoryText("manifest.033");
  if (key === "path") return parent.includes("files") ? hostRepositoryText("manifest.063") : parent.includes("volumes") ? hostRepositoryText("manifest.061") : commonText("resource-domain:blueprintDetails.path");
  if (key === "port" && parent.at(-1) === "httpGet") return hostRepositoryText("manifest.047");
  if (key === "health") return commonText("monitoring:healthTab");
  return labelKeys[key] ? commonText(`resource-domain:${labelKeys[key]}`)
    : manifestKeys[key] ? hostRepositoryText(`manifest.${manifestKeys[key]}`) : key;
}

function scalar(value: unknown, path: Path): ReactNode {
  if (value === null) return <span className="text-muted-foreground">null</span>;
  if (typeof value === "boolean") return commonText(value ? "common.yes" : "common.no");
  if (value === "") return <span className="text-muted-foreground">{commonText("resource-domain:blueprintDetails.emptyString")}</span>;
  if (dictionary(path.slice(0, -1))) return String(value);
  const key = path.at(-1);
  if (key === "identity" && (value === "stable" || value === "stateless"))
    return commonText(`resource-domain:wizard.${value === "stable" ? "identityStable" : "identityStateless"}`);
  const enums: Record<string, Record<string, string>> = {
    kind: { Blueprint: "blueprint", direct: "direct", vip: "vip", discovery: "discovery" },
    replace: { supported: "supportedValue", unsupported: "unsupportedValue" },
  };
  const translated = enums[String(key)]?.[String(value)];
  if (translated) return commonText(`resource-domain:blueprintDetails.${translated}`);
  if (key === "type" && (value === "recreate" || value === "rolling"))
    return hostRepositoryText(value === "recreate" ? "manifest-extra.001" : "manifest-extra.002");
  return String(value);
}

type DefinitionProps = {
  value: Row; readOnly?: boolean; workspace?: string; parameters?: TemplateParameter[]; mode?: string;
  onSet?: (path: Path, value: unknown) => void;
  onSourceChange?: (path: Path, value: string) => void;
  onAppend?: (path: Path, value: unknown) => void;
  onRemove?: (path: Path, index?: number) => void;
  onInvalidChange?: ((key: string, invalid: boolean) => void) | undefined;
};

/** One schema-aware field tree for catalog, template authoring and concrete instances. */
export function ApplicationDefinitionFields({ value, readOnly = true, workspace = "", parameters = [], mode = "", onSet, onSourceChange, onAppend, onRemove, onInvalidChange }: DefinitionProps) {
  const blueprint = value.apiVersion === "ferry/blueprint-v1";
  const set = (path: Path, next: unknown) => { if (!readOnly) onSet?.(path, next); };
  const add = (item: Row, path: Path) => !readOnly && <DefinitionFieldAdd value={item} path={path} blueprint={blueprint} onSet={set} />;
  const removeButton = (path: Path, index?: number) => !readOnly && <Button type="button" variant="outline" className="min-h-11" onClick={() => onRemove?.(path, index)}>
    {commonText("common.delete")}
  </Button>;
  const field = (item: unknown, path: Path): ReactNode => readOnly
    ? <span data-blueprint-value data-readonly-field className="block min-h-11 w-full min-w-0 whitespace-pre-wrap break-words rounded-md border border-input bg-background px-3 py-2 text-sm [overflow-wrap:anywhere]">{scalar(item, path)}</span>
    : <DefinitionScalar key={JSON.stringify(path)} value={item} path={path} root={value} workspace={workspace} parameters={parameters}
      onSet={set} onInvalidChange={onInvalidChange} />;
  const contents = (item: unknown, path: Path, omitName = false): ReactNode => {
    if (Array.isArray(item)) {
      const append = !readOnly && <Button type="button" variant="outline" className="min-h-11" disabled={(path.at(-1) === "roles" && item.length >= 10) || (path.at(-1) === "services" && item.length >= 50)}
        onClick={() => onAppend?.(path, definitionItem(path, item, blueprint, mode))}>{hostRepositoryText("manifest-extra.003", { value1: fieldLabel(String(path.at(-1)), path.slice(0, -1)) })}</Button>;
      if (!item.length) return <div className="space-y-3"><p className="text-sm text-muted-foreground">[]</p>{append}</div>;
      if (item.every(child => child === null || typeof child !== "object")) return <div className="space-y-3"><ol className="min-w-0 list-decimal space-y-3 ps-5 text-sm">
        {item.map((child, index) => <li key={index} className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]"><div className="flex min-w-0 items-start gap-2"><div className="min-w-0 flex-1">{field(child, [...path, index])}</div>{removeButton(path, index)}</div></li>)}
      </ol>{append}</div>;
      return <div className="min-w-0 space-y-5">{item.map((child, index) => {
        const name = object(child) && typeof child.name === "string" && child.name ? child.name : undefined;
        return <article key={index} data-blueprint-item={path.join(".")} className="min-w-0 space-y-3 border-s-2 ps-4">
          <div className="flex min-w-0 flex-wrap items-center justify-between gap-2"><h4 className="break-words text-sm font-semibold [overflow-wrap:anywhere]">{name
            ? <span data-blueprint-value>{name}</span>
            : commonText("resource-domain:blueprintDetails.item", { name: fieldLabel(String(path.at(-1)), path.slice(0, -1)), index: index + 1 })}</h4>{removeButton(path, index)}</div>
          {contents(child, [...path, index])}
        </article>;
      })}{append}</div>;
    }
    if (!object(item)) return field(item, path);
    const entries = Object.entries(item).filter(([key]) => !omitName || key !== "name");
    if (!entries.length) return <div className="space-y-3">{!omitName && <p className="text-sm text-muted-foreground">{"{}"}</p>}{add(item, path)}</div>;
    const simple = entries.filter(([, child]) => child === null || typeof child !== "object");
    const nested = entries.filter(([, child]) => child !== null && typeof child === "object");
    const service = !blueprint && path.length === 2 && path[0] === "services";
    const source = item.image !== undefined && object(item.artifact) ? "both" : object(item.artifact) ? "artifact" : "image";
    const sources = [...(value.apiVersion === "ferry/v2" ? ["both"] : []), ...(mode !== "native" ? ["image"] : []), ...(!["docker", "compose", "stack"].includes(mode) ? ["artifact"] : [])];
    if (!dictionary(path)) nested.sort(([a], [b]) => (order.indexOf(a) < 0 ? order.length : order.indexOf(a)) - (order.indexOf(b) < 0 ? order.length : order.indexOf(b)));
    return <div className="min-w-0 space-y-4">
      {service && <AutoField label={hostRepositoryText("manifest.033")}>
        {readOnly ? <span className="block min-h-11 w-full rounded-md border border-input bg-background px-3 py-2 text-sm">{source === "both" ? commonText("resource-domain:deployment.dualSource") : hostRepositoryText(source === "image" ? "manifest.034" : "manifest.035")}</span> : <select className="min-h-11 w-full rounded-md border bg-background px-3 text-sm" aria-label={hostRepositoryText("manifest.033")} required value={sources.includes(source) ? source : ""} onChange={event => onSourceChange?.(path, event.target.value)}>
          <option value="" disabled>{commonText("common.select")}</option>{sources.map(source => <option key={source} value={source}>{source === "both" ? commonText("resource-domain:deployment.dualSource") : hostRepositoryText(source === "image" ? "manifest.034" : "manifest.035")}</option>)}
        </select>}
      </AutoField>}
      {!!simple.length && <dl className="grid min-w-0 gap-x-6 gap-y-4 sm:grid-cols-2 lg:grid-cols-3">{simple.map(([key, child]) =>
        <div key={key} className="min-w-0"><dt className="break-words text-xs text-muted-foreground [overflow-wrap:anywhere]">{fieldLabel(key, path)}</dt>
          <dd className="mt-1 whitespace-pre-wrap break-words text-sm [overflow-wrap:anywhere]" data-blueprint-value>{field(child, [...path, key])}</dd>
          {!readOnly && dictionary(path) && removeButton([...path, key])}</div>)}</dl>}
      {nested.map(([key, child]) => <section key={key} aria-label={fieldLabel(key, path)} className="min-w-0 space-y-3 border-t pt-4">
        <div className="flex min-w-0 flex-wrap items-center justify-between gap-2"><h5 className="break-words text-sm font-medium">{fieldLabel(key, path)}</h5>
          {!readOnly && removeButton([...path, key])}</div>{contents(child, [...path, key])}
      </section>)}{add(item, path)}
    </div>;
  };
  const groups = [
    { key: "definition", title: commonText("resource-domain:blueprintDetails.definition"), keys: ["apiVersion", "kind", "allowSingleHost", "metadata"] },
    ...["drivers", "roles", "services", "endpoints", "update", "strategy", "hooks", "overrides"].map(key => ({ key, title: fieldLabel(key, []), keys: [key] })),
  ];
  const known = new Set(groups.flatMap(group => group.keys));
  const other = Object.keys(value).filter(key => !known.has(key));
  if (other.length) groups.push({ key: "other", title: commonText("common.other"), keys: other });
  return <section aria-label={commonText("resource-domain:template.appContent")} data-application-definition data-blueprint-form={value.apiVersion === "ferry/blueprint-v1" ? "" : undefined} className="min-w-0 space-y-6">
    {groups.filter(group => group.keys.some(key => Object.hasOwn(value, key))).map(group => <section key={group.key} data-blueprint-group={group.key}
      aria-label={group.title} className="min-w-0 space-y-4 border-b pb-6 last:border-b-0 last:pb-0">
      <h3 className="break-words text-base font-semibold">{group.title}</h3>
      {group.keys.length === 1 && group.key === group.keys[0] ? contents(value[group.keys[0]!], [group.keys[0]!])
        : <div className="space-y-4"><dl className="grid min-w-0 gap-x-6 gap-y-4 sm:grid-cols-2 lg:grid-cols-3">
          {group.keys.filter(key => Object.hasOwn(value, key) && (value[key] === null || typeof value[key] !== "object")).map(key => <div key={key} className="min-w-0">
            <dt className="break-words text-xs text-muted-foreground">{fieldLabel(key, [])}</dt><dd className="mt-1 min-w-0 text-sm">{field(value[key], [key])}</dd>
          </div>)}
        </dl>{group.keys.filter(key => Object.hasOwn(value, key) && value[key] !== null && typeof value[key] === "object").map(key => <section key={key} className="space-y-3 border-t pt-4">
          <h4 className="text-sm font-medium">{fieldLabel(key, [])}</h4>{contents(value[key], [key])}
        </section>)}</div>}
    </section>)}
    {add(value, [])}
  </section>;
}



function DefinitionFieldAdd({ value, path, blueprint, onSet }: { value: Row; path: Path; blueprint: boolean; onSet: (path: Path, value: unknown) => void }) {
  const [key, setKey] = useState("");
  const [type, setType] = useState("string");
  const shape = definitionShape(path, blueprint);
  const choices = Object.keys(shape).filter(key => !Object.hasOwn(value, key));
  const freeKey = dictionary(path) || path.includes("overrides") || path.length > 0 && !Object.keys(shape).length;
  if (!freeKey && !choices.length) return null;
  return <div className="flex min-w-0 flex-wrap items-end gap-2 border-t pt-3" data-definition-add={JSON.stringify(path)}>
    <div className="min-w-0 flex-1"><AutoField label={commonText("resource-domain:application.addField")}>
      {freeKey ? <Input className="min-h-11" value={key} onChange={event => setKey(event.target.value)} aria-label={commonText("resource-domain:application.fieldKey")} />
        : <select className="min-h-11 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm" value={key} onChange={event => setKey(event.target.value)}>
          <option value="">{commonText("common.select")}</option>{choices.map(key => <option key={key} value={key}>{fieldLabel(key, path)}</option>)}
        </select>}
    </AutoField></div>
    {freeKey && path.at(-1) !== "labels" && <AutoField label={commonText("resource-domain:template.parameterType")}><select className="min-h-11 rounded-md border bg-background px-3 text-sm" value={type} onChange={event => setType(event.target.value)}>
      {["string", "integer", "boolean", "object", "array", "null"].map(type => <option key={type} value={type}>{commonText(type === "object" || type === "array" || type === "null" ? `resource-domain:application.type${type[0]!.toUpperCase() + type.slice(1)}` : `resource-domain:template.${type === "string" ? "typeString" : type === "integer" ? "typeInteger" : "typeBoolean"}`)}</option>)}
    </select></AutoField>}
    <Button type="button" variant="outline" className="min-h-11" disabled={!key.trim() || Object.hasOwn(value, key) || (!freeKey && !Object.hasOwn(shape, key))} onClick={() => {
      onSet([...path, key], freeKey ? type === "integer" ? 0 : type === "boolean" ? false : type === "object" ? {} : type === "array" ? [] : type === "null" ? null : "" : shape[key]);setKey("");
    }}>{commonText("resource-domain:application.addField")}</Button>
  </div>;
}

function DefinitionScalar({ value, path, root, workspace, parameters, onSet, onInvalidChange }: {
  value: unknown; path: Path; root: Row; workspace: string; parameters: TemplateParameter[];
  onSet: (path: Path, value: unknown) => void; onInvalidChange?: ((key: string, invalid: boolean) => void) | undefined;
}) {
  const key = JSON.stringify(path), label = fieldLabel(String(path.at(-1)), path.slice(0, -1));
  const [numberText, setNumberText] = useState(String(value ?? ""));
  useEffect(() => setNumberText(String(value ?? "")), [value]);
  useEffect(() => () => onInvalidChange?.(key, false), [key, onInvalidChange]);
  const pipelines = useCallback(() => pipelineChoices(workspace), [workspace]);
  const isExpression = typeof value === "string" && /\{\{\s*\.Values\./.test(value);
  const shape = definitionShape(path.slice(0, -1), root.apiVersion === "ferry/blueprint-v1");
  const numeric = typeof value === "number" || typeof shape[String(path.at(-1))] === "number";
  const bindings = parameters.filter(parameter => path.at(-1) === "valueFrom" ? parameter.secret : !parameter.secret && (numeric ? parameter.type === "integer" : typeof value === "boolean" ? parameter.type === "boolean" : parameter.type === "string"));
  const choices = path.length === 1 && (path.at(-1) === "kind" || root.apiVersion === "ferry/blueprint-v1" && path.at(-1) === "apiVersion") ? undefined : definitionChoices(path);
  let parent: unknown = root;
  for (const part of path.slice(0, -1)) parent = (parent as Row | undefined)?.[part];
  const artifact = path.at(-2) === "artifact" && object(parent) ? parent : undefined;
  const set = (next: unknown) => onSet(path, next);
  let control: ReactNode;
  if (value === null && (object(shape[String(path.at(-1))]) || Array.isArray(shape[String(path.at(-1))]))) control = <Button type="button" variant="outline" className="min-h-11" onClick={() => set(shape[String(path.at(-1))])}>{commonText("resource-domain:application.addField")} · {label}</Button>;
  else if (path.length === 1 && (path.at(-1) === "kind" || root.apiVersion === "ferry/blueprint-v1" && path.at(-1) === "apiVersion")) control = <Input className="min-h-11" value={String(value ?? "")} readOnly aria-label={label} />;
  else if (artifact && workspace && !isExpression && path.at(-1) === "pipeline" && !parameters.length) control = <ReferencePicker label={label} value={String(value ?? "")} empty={hostRepositoryText("manifest.041")} readOnly={false} load={pipelines}
    onChange={set} />;
  else if (artifact && workspace && path.at(-1) === "run" && typeof artifact.pipeline === "string" && !artifact.pipeline.includes("{{")) control = <RunPicker workspace={workspace} pipeline={artifact.pipeline} value={String(value ?? "")} readOnly={false} onChange={set} />;
  else if (typeof value === "boolean") control = <select className="min-h-11 w-full rounded-md border bg-background px-3 text-sm" aria-label={label} value={String(value)} onChange={event => set(event.target.value === "true")}>
    <option value="true">{commonText("common.yes")}</option><option value="false">{commonText("common.no")}</option>
  </select>;
  else if (choices && !dictionary(path.slice(0, -1))) control = <select className="min-h-11 w-full min-w-0 rounded-md border bg-background px-3 text-sm" aria-label={label} value={String(value ?? "")} onChange={event => set(event.target.value)}>
    {[...new Set([String(value ?? ""), ...choices])].map(choice => <option key={choice} value={choice}>{scalar(choice, path)}</option>)}
  </select>;
  else if (numeric && !isExpression) control = <Input type="number" step="any" className="min-h-11" aria-label={label} value={numberText} onChange={event => {
    const text = event.target.value;setNumberText(text);const invalid = text.trim() === "" || !Number.isFinite(Number(text));onInvalidChange?.(key, invalid);if (!invalid) set(Number(text));
  }} />;
  else control = <textarea className="min-h-11 w-full min-w-0 resize-y rounded-md border border-input bg-background px-3 py-2 text-sm" rows={typeof value === "string" && value.includes("\n") ? 4 : 1} aria-label={label} value={value === null ? "" : String(value ?? "")} placeholder={value === null ? "null" : undefined} onChange={event => set(numeric && event.target.value.trim() && Number.isFinite(Number(event.target.value)) ? Number(event.target.value) : event.target.value)} />;
  return <div className="min-w-0 space-y-2" data-definition-path={key}>{control}
    {bindings.length > 0 && !(path.length === 1 && (path[0] === "apiVersion" || path[0] === "kind")) && <select className="min-h-11 w-full min-w-0 rounded-md border bg-background px-3 text-xs" aria-label={commonText("resource-domain:template.bindField", { field: label })} value="" onChange={event => { if (event.target.value) set(`{{ .Values.${event.target.value} }}`); }}>
      <option value="">{commonText("resource-domain:template.bindParameter")}</option>{bindings.map(parameter => <option key={parameter.name} value={parameter.name}>{parameter.label || parameter.name}</option>)}
    </select>}
  </div>;
}
