import { parseDocument, visit } from "yaml";
import { hostRepositoryText } from "../../../shared/i18n/i18n";

export type Path = (string | number)[];

export type Row = Record<string, unknown>;

export const object = (value: unknown): value is Row =>
  !!value && typeof value === "object" && !Array.isArray(value);

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
  if (alias) throw new Error(hostRepositoryText("manifest.001"));
  const value: unknown = doc.toJS({ maxAliasCount: 50 });
  if (
    !object(value) ||
    !object(value.metadata) ||
    !Array.isArray(value.services) ||
    !value.services.every(object)
  )
    throw new Error(hostRepositoryText("manifest.002"));
  const optionalObject = (parent: Row, key: string) => {
    if (parent[key] != null && !object(parent[key]))
      throw new Error(hostRepositoryText("manifest.003", { value1: key }));
  };
  optionalObject(value, "strategy");
  optionalObject(value, "overrides");
  optionalObject(value.metadata, "labels");
  // Do not reinterpret malformed collection shapes as empty lists and lose data.
  for (const service of value.services as Row[]) {
    for (const name of ["ports", "env", "volumes", "files"]) {
      const rows = service[name];
      if (rows !== undefined && (!Array.isArray(rows) || !rows.every(object)))
        throw new Error(hostRepositoryText("manifest.004", { value1: name }));
    }
    for (const name of ["command", "dependsOn"]) {
      const rows = service[name];
      if (
        rows != null &&
        (!Array.isArray(rows) || !rows.every((v) => typeof v === "string"))
      )
        throw new Error(hostRepositoryText("manifest.005", { value1: name }));
    }
    for (const name of ["artifact", "resources", "health", "placement"])
      optionalObject(service, name);
    if (object(service.placement) && service.placement.constraints !== undefined
      && (!Array.isArray(service.placement.constraints) || !service.placement.constraints.every(value => typeof value === "string")))
      throw new Error(hostRepositoryText("manifest.005", { value1: "placement.constraints" }));
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

export type ManifestDocument = ReturnType<typeof readManifest>["doc"];
export type ManifestChange = { path: Path; value: unknown };
export function changeManifest(
  doc: ManifestDocument,
  changes: ManifestChange[],
) {
  const next = doc.clone();
  for (const change of changes) {
    // Optional objects may be explicitly null in existing documents.
    if (change.value !== undefined)
      for (let length = 1; length < change.path.length; length++) {
        const parent = change.path.slice(0, length);
        if (next.getIn(parent) == null)
          next.setIn(
            parent,
            next.createNode(typeof change.path[length] === "number" ? [] : {}),
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
  return next.toString();
}

export function removeManifest(
  doc: ManifestDocument,
  path: Path,
  index: number,
) {
  const next = doc.clone();
  next.deleteIn([...path, index]);
  return next.toString();
}

export function appendManifest(
  doc: ManifestDocument,
  path: Path,
  value: unknown,
) {
  const next = doc.clone();
  if (!next.hasIn(path)) next.setIn(path, next.createNode([]));
  next.addIn(path, next.createNode(value));
  return next.toString();
}
