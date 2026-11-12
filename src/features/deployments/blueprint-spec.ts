/** Deployment spec (`ferry/deploy-v1`) ↔ YAML for the blueprint wizard.
 * The YAML document stays the source of truth: form edits patch it, so comments and
 * key order survive. Validation mirrors the server's `deny_unknown_fields` shape so the
 * wizard never submits keys the server rejects; business rules stay server-side.
 */
import { Document, parse, parseDocument, stringify } from "yaml";
import { commonText } from "../../shared/i18n/i18n";

export const SPEC_API_VERSION = "ferry/deploy-v1";

export interface HostSelector { tags?: string[]; names?: string[]; group?: string; spread?: { prefix: string } }
export interface RoleSpec { instances: number; hosts?: HostSelector }
export interface InstanceBinding { hostId: string; address: string }
export interface DeploySpec {
  template: { id: string; version: number };
  driver: string;
  manager?: string;
  values: Record<string, unknown>;
  roles: Record<string, RoleSpec>;
  placement?: Record<string, InstanceBinding[]>;
}
export type SpecChange = { path: (string | number)[]; value: unknown };

const text = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:wizard.${key}`, values);
type Row = Record<string, unknown>;
const isRow = (value: unknown): value is Row => !!value && typeof value === "object" && !Array.isArray(value);

class SpecError extends Error {}
const fail = (key: string, values?: Record<string, unknown>): never => { throw new SpecError(text(key, values)); };

function only(row: Row, allowed: string[], at: string) {
  const extra = Object.keys(row).find(key => !allowed.includes(key));
  if (extra !== undefined) fail("unknownKey", { key: at ? `${at}.${extra}` : extra });
}
function mapping(value: unknown, field: string): Row {
  if (value == null) return {};
  if (!isRow(value)) fail("invalidField", { field });
  return value as Row;
}
function string(value: unknown, field: string): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isInteger(value)) return String(value);
  return fail("invalidField", { field });
}
function strings(value: unknown, field: string): string[] | undefined {
  if (value == null) return undefined;
  if (!Array.isArray(value) || !value.every(item => typeof item === "string")) fail("invalidField", { field });
  return (value as string[]).length ? value as string[] : undefined;
}

function toSpec(value: unknown): DeploySpec {
  if (!isRow(value)) return fail("notMapping");
  const root = value as Row;
  only(root, ["apiVersion", "template", "driver", "manager", "values", "roles", "placement"], "");
  if (root.apiVersion !== SPEC_API_VERSION) fail("apiVersion", { expected: SPEC_API_VERSION });
  if (!isRow(root.template)) fail("invalidField", { field: "template" });
  const template = root.template as Row;
  only(template, ["id", "version"], "template");
  if (!Number.isInteger(template.version)) fail("invalidField", { field: "template.version" });
  const roles: Record<string, RoleSpec> = {};
  if (!isRow(root.roles)) fail("invalidField", { field: "roles" });
  for (const [name, raw] of Object.entries(root.roles as Row)) {
    if (!isRow(raw)) fail("invalidField", { field: `roles.${name}` });
    const role = raw as Row;
    only(role, ["instances", "hosts"], `roles.${name}`);
    if (!Number.isInteger(role.instances) || (role.instances as number) < 0) fail("invalidField", { field: `roles.${name}.instances` });
    const hosts = mapping(role.hosts, `roles.${name}.hosts`);
    only(hosts, ["tags", "names", "group", "spread"], `roles.${name}.hosts`);
    const selector: HostSelector = {};
    const tags = strings(hosts.tags, `roles.${name}.hosts.tags`), names = strings(hosts.names, `roles.${name}.hosts.names`);
    if (tags) selector.tags = tags;
    if (names) selector.names = names;
    if (hosts.group != null) selector.group = string(hosts.group, `roles.${name}.hosts.group`);
    if (hosts.spread != null) {
      const spread = mapping(hosts.spread, `roles.${name}.hosts.spread`);
      only(spread, ["prefix"], `roles.${name}.hosts.spread`);
      selector.spread = { prefix: string(spread.prefix, `roles.${name}.hosts.spread.prefix`) };
    }
    roles[name] = { instances: role.instances as number, ...(Object.keys(selector).length ? { hosts: selector } : {}) };
  }
  const spec: DeploySpec = {
    template: { id: string(template.id, "template.id"), version: template.version as number },
    driver: string(root.driver, "driver"),
    values: mapping(root.values, "values"),
    roles,
  };
  if (root.manager != null) spec.manager = string(root.manager, "manager");
  if (root.placement != null) {
    spec.placement = {};
    for (const [name, list] of Object.entries(mapping(root.placement, "placement"))) {
      if (!Array.isArray(list)) return fail("invalidField", { field: `placement.${name}` });
      spec.placement[name] = list.map((item: unknown, index) => {
        const at = `placement.${name}.${index}`;
        if (!isRow(item)) return fail("invalidField", { field: at });
        only(item, ["hostId", "address"], at);
        return { hostId: string(item.hostId, `${at}.hostId`), address: string(item.address, `${at}.address`) };
      });
    }
  }
  return spec;
}

/** Parses the YAML text. `spec` is present only when there are no errors. */
export function readSpec(source: string): { doc?: Document; spec?: DeploySpec; errors: string[] } {
  const doc = parseDocument(source, { uniqueKeys: true });
  if (doc.errors.length) return { errors: doc.errors.map(error => text("syntax", { detail: error.message })) };
  try { return { doc, spec: toSpec(doc.toJS({ maxAliasCount: 50 })), errors: [] }; }
  catch (error) {
    if (error instanceof SpecError) return { doc, errors: [error.message] };
    return { doc, errors: [text("syntax", { detail: error instanceof Error ? error.message : String(error) })] };
  }
}

const empty = (value: unknown) => value === undefined || value === "" || (Array.isArray(value) && !value.length);

/** Applies form changes to the document; empty values remove their key. */
export function writeSpec(doc: Document, changes: SpecChange[]): string {
  const next = doc.clone();
  for (const { path, value } of changes) {
    if (empty(value)) { if (next.hasIn(path)) next.deleteIn(path); continue; }
    for (let length = 1; length < path.length; length++) {
      const parent = path.slice(0, length);
      if (next.getIn(parent) == null) next.setIn(parent, next.createNode(typeof path[length] === "number" ? [] : {}));
    }
    next.setIn(path, typeof value === "object" && value !== null ? next.createNode(value) : value);
  }
  return next.toString();
}

/** Canonical first document, `apiVersion` first like the server's `to_yaml`. */
export function initialSpec(spec: DeploySpec): string {
  return stringify({ apiVersion: SPEC_API_VERSION, ...spec });
}

export interface InstanceBounds { min: number; max: number; default: number; odd: boolean }

export function instancesProblem(count: number, bounds: InstanceBounds): "range" | "odd" | undefined {
  if (!Number.isInteger(count) || count < bounds.min || count > bounds.max) return "range";
  if (bounds.odd && count % 2 === 0) return "odd";
  return undefined;
}

/** One stepper click: odd-only roles move between odd counts; the result stays in bounds. */
export function stepInstances(count: number, delta: 1 | -1, bounds: InstanceBounds): number {
  let next = count + delta;
  if (bounds.odd && next % 2 === 0) next += delta;
  if (next > bounds.max || next < bounds.min) return instancesProblem(count, bounds) ? Math.min(Math.max(count, bounds.min), bounds.max) : count;
  return next;
}

export interface BlueprintRole { name: string; instances: InstanceBounds; identity: string; distinctHosts: boolean; colocateWith: string | undefined }
export interface BlueprintSummary { name: string; roles: BlueprintRole[]; drivers: string[] }

const KIND_LINE = /^\s*"?kind"?\s*:\s*"?Blueprint"?\s*,?\s*$/m;

/** Templates may hold unquoted placeholders that are not valid YAML until rendered. */
export function isBlueprint(content: string): boolean {
  try {
    const value: unknown = parse(content, { logLevel: "error" });
    if (isRow(value)) return value.kind === "Blueprint";
  } catch { /* fall back to the kind line */ }
  return KIND_LINE.test(content);
}

/** Roles and selectable drivers of a blueprint template (k8s is not available yet). */
export function readBlueprint(content: string): BlueprintSummary | undefined {
  let value: unknown;
  try { value = parse(content, { logLevel: "error" }); } catch { return undefined; }
  if (!isRow(value) || value.kind !== "Blueprint" || !Array.isArray(value.roles)) return undefined;
  const roles: BlueprintRole[] = [];
  for (const raw of value.roles) {
    if (!isRow(raw) || typeof raw.name !== "string" || !isRow(raw.instances)) return undefined;
    const { min, max, default: initial, odd } = raw.instances;
    if (![min, max, initial].every(Number.isInteger)) return undefined;
    const placement = isRow(raw.placement) ? raw.placement : {};
    roles.push({
      name: raw.name,
      instances: { min: min as number, max: max as number, default: initial as number, odd: odd === true },
      identity: String(raw.identity ?? ""),
      distinctHosts: placement.distinctHosts === true,
      colocateWith: typeof placement.colocateWith === "string" ? placement.colocateWith : undefined,
    });
  }
  const drivers = isRow(value.drivers)
    ? Object.entries(value.drivers).filter(([name, support]) => name !== "k8s" && isRow(support) && support.supported === true).map(([name]) => name)
    : [];
  const metadata = isRow(value.metadata) ? value.metadata : {};
  return { name: typeof metadata.name === "string" ? metadata.name : "", roles, drivers };
}
