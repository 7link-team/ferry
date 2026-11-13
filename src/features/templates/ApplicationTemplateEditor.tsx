import { lazy, Suspense, useCallback, useEffect, useId, useRef, useState } from "react";
import { parseDocument, stringify, visit } from "yaml";
import { commonText, hostRepositoryText, useCommonTranslation } from "../../shared/i18n/i18n";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { Button } from "../../shared/ui/button";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import type { TemplateParameter } from "./TemplateParametersEditor";
import { ApplicationDefinitionFields } from "./ApplicationDefinitionFields";
import { appendManifest, changeManifest, object, removeManifest, type Path } from "../deployments/manifest/model";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";

const TemplateCodeEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));

/** Format loaded JSON only; preserve authored YAML, comments and incomplete input. */
export function applicationTemplateSource(value: string) {
  if (!value.trimStart().startsWith("{")) return value;
  try { return stringify(JSON.parse(value)); } catch { return value; }
}

export function ApplicationTemplateEditor({ value, readOnly, onChange, onValidityChange, workspace, templateParameters, mode = "" }: {
  value: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onValidityChange: (blocked: boolean) => void;
  workspace: string;
  templateParameters?: TemplateParameter[];
  mode?: string;
}) {
  const { i18n } = useCommonTranslation();
  const [capabilities, setCapabilities] = useState<{ mode: string; supported: boolean; reason?: string }[]>();
  const [capabilityError, setCapabilityError] = useState("");
  const [checking, setChecking] = useState(false);
  const capabilityRequest = useRef<AbortController | undefined>(undefined);
  useEffect(() => {
    capabilityRequest.current?.abort();setCapabilities(undefined);setCapabilityError("");setChecking(false);
    return () => capabilityRequest.current?.abort();
  }, [value, workspace, i18n.resolvedLanguage]);
  const checkCapabilities = async () => {
    capabilityRequest.current?.abort();const controller = new AbortController();capabilityRequest.current = controller;
    setChecking(true);setCapabilityError("");
    try {
      const result = await api<{ checks: { mode: string; supported: boolean; reason?: string }[] }>(base(workspace, "deployment-capabilities"), {
        method: "POST", body: JSON.stringify({ manifest: value }), signal: controller.signal,
      });
      if (!controller.signal.aborted) setCapabilities(result.checks);
    } catch (error) { if (!controller.signal.aborted) setCapabilityError(errorText(error)); }
    finally { if (!controller.signal.aborted) setChecking(false); }
  };
  const authored = useRef<string | undefined>(undefined);
  const [sourceMode, setSourceMode] = useState(false);
  const [invalidFields, setInvalidFields] = useState<Record<string, boolean>>({});
  const [notice, setNotice] = useState("");
  const [removal, setRemoval] = useState<{ path: Path; index?: number }>();
  const reportInvalid = useCallback((key: string, invalid: boolean) => setInvalidFields(current => current[key] === invalid ? current : { ...current, [key]: invalid }), []);
  const editorId = useId();
  const source = authored.current === value ? value : applicationTemplateSource(value);
  const doc = parseDocument(source);
  const syntaxError = doc.errors[0]?.message;
  let preview: Record<string, unknown> | undefined;
  let previewError = syntaxError;
  let aliases = false;
  visit(doc, { Alias: () => { aliases = true; } });
  if (aliases) previewError = hostRepositoryText("manifest.001");
  if (!syntaxError && !aliases) {
    try { const parsed: unknown = doc.toJS({ maxAliasCount: 50 }); if (object(parsed)) preview = parsed; }
    catch (error) { previewError = error instanceof Error ? error.message : String(error); }
  }
  const invalid = !!syntaxError || Object.values(invalidFields).some(Boolean);
  useEffect(() => { onValidityChange(invalid);return () => onValidityChange(false); }, [invalid, onValidityChange]);
  const change = (next: string) => { authored.current = next; onChange(next); };
  const set = (path: Path, next: unknown) => {
    if (readOnly || !preview || syntaxError) return;
    setNotice("");
    if (path.length === 1 && path[0] === "apiVersion" && next === "ferry/v1" && Array.isArray(preview.services) && preview.services.some(service => object(service) && ((service.image && service.artifact) || service.placement))) {
      setNotice(commonText("resource-domain:deployment.downgradeBlocked"));return;
    }
    const changes = [{ path, value: next }];
    const key = path.at(-1), parent = path.slice(0, -1);
    if (path.length === 3 && (path[0] === "services" || path[0] === "roles") && key === "name") {
      const items = preview[path[0]];
      if (Array.isArray(items)) {
        const oldName = items[Number(path[1])]?.name;
        if (items.some((item, index) => index !== path[1] && item?.name === next)) { setNotice(hostRepositoryText("manifest.031"));return; }
        items.forEach((item, index) => {
          if (Array.isArray(item?.dependsOn)) item.dependsOn.forEach((name: unknown, at: number) => {
            if (name === oldName) changes.push({ path: [path[0]!, index, "dependsOn", at], value: next });
          });
          if (item?.placement?.colocateWith === oldName) changes.push({ path: [path[0]!, index, "placement", "colocateWith"], value: next });
        });
        if (path[0] === "roles" && Array.isArray(preview.endpoints)) preview.endpoints.forEach((endpoint, index) => {
          if (endpoint?.role === oldName) changes.push({ path: ["endpoints", index, "role"], value: next });
        });
      }
    }
    if (path.at(-2) === "artifact" && key === "pipeline") changes.push({ path: [...parent, "run"], value: undefined });
    if (path.includes("env") && (key === "value" || key === "valueFrom")) changes.push({ path: [...parent, key === "value" ? "valueFrom" : "value"], value: undefined });
    if (key === "artifact" && preview.apiVersion === "ferry/v1") changes.push({ path: [...parent, "image"], value: undefined });
    if (key === "image" && preview.apiVersion === "ferry/v1") changes.push({ path: [...parent, "artifact"], value: undefined });
    change(changeManifest(doc, changes));
  };
  const remove = (path: Path, index?: number) => {
    if (readOnly || !preview) return;
    if (index !== undefined && path.length === 1 && (path[0] === "services" || path[0] === "roles")) {
      const items = preview[path[0]];
      if (Array.isArray(items)) {
        const name = items[index]?.name;
        if (items.some((item, at) => at !== index && (item?.dependsOn?.includes(name) || item?.placement?.colocateWith === name)) || Array.isArray(preview.endpoints) && preview.endpoints.some(endpoint => endpoint?.role === name)) {
          setNotice(hostRepositoryText("manifest.028"));return;
        }
      }
    }
    setRemoval({ path, ...(index === undefined ? {} : { index }) });
  };
  return <div className="min-w-0 space-y-3" data-application-editor data-definition-mode={readOnly ? "view" : "edit"} data-deployment-mode={mode}>
    <div className="flex justify-end"><Button type="button" variant="outline" className="min-h-11" disabled={!preview || Object.values(invalidFields).some(Boolean)}
      onClick={() => setSourceMode(current => !current)}>{hostRepositoryText(readOnly ? sourceMode || !preview ? "manifest.formPreview" : "manifest.sourcePreview" : sourceMode || !preview ? "manifest.018" : "manifest.019")}</Button></div>
    {!readOnly && mode && ["native", "docker", "compose", "stack"].includes(mode) && <p className="text-sm text-muted-foreground">{commonText(`resource-domain:deployment.capability.${mode}`)}</p>}
    {!readOnly && workspace && templateParameters === undefined && preview?.apiVersion !== "ferry/blueprint-v1" && <section className="space-y-3" aria-label={commonText("resource-domain:deployment.checkCompatibility")}>
      <Button type="button" variant="outline" className="min-h-11" disabled={checking || invalid} onClick={() => void checkCapabilities()}>{commonText(`resource-domain:deployment.${checking ? "checkingCompatibility" : "checkCompatibility"}`)}</Button>
      <p className="text-xs text-muted-foreground">{commonText("resource-domain:deployment.compatibilityHelp")}</p>
      <div aria-live="polite" className="grid min-w-0 gap-3 sm:grid-cols-2">{capabilities?.map(result => <div key={result.mode} className="min-w-0 rounded-md border p-3 text-sm">
        <span>{result.mode} · {commonText(`resource-domain:deployment.${result.supported ? "compatible" : "incompatible"}`)}</span>
        {result.reason && <p className="mt-2 break-words text-destructive">{result.reason}</p>}
      </div>)}</div>{capabilityError && <p role="alert" className="text-sm text-destructive">{capabilityError}</p>}
    </section>}
    {!sourceMode && preview ? <ApplicationDefinitionFields value={preview} readOnly={readOnly} workspace={workspace} parameters={templateParameters ?? []} mode={mode}
      onSet={set} onSourceChange={(path, source) => {
        if (readOnly) return;
        let service: unknown = preview;for (const part of path) service = (service as Record<string | number, unknown>)?.[part];
        if (!object(service)) return;
        change(changeManifest(doc, [{ path: [...path, "image"], value: source === "artifact" ? undefined : service.image ?? "" },
          { path: [...path, "artifact"], value: source === "image" ? undefined : service.artifact ?? { source: "pipeline", pipeline: "", path: "" } }]));
      }} onAppend={(path, value) => { if (!readOnly) change(appendManifest(doc, path, value)); }} onRemove={remove} onInvalidChange={reportInvalid} /> : <div className="h-[28rem] min-w-0 overflow-hidden rounded-md border">
      <Suspense fallback={<LoadingSkeleton variant="editor" />}><TemplateCodeEditor value={source} language="yaml" path={`${workspace}/application-template-${editorId}.yaml`} label={commonText("resource-domain:template.appContent")} readOnly={readOnly} onChange={change} /></Suspense>
    </div>}
    {previewError && <p role="alert" className="whitespace-pre-wrap text-sm text-destructive">{previewError}</p>}
    {notice && <p role="alert" className="text-sm text-destructive">{notice}</p>}
    {removal && <DeleteConfirmDialog action={commonText("common.delete")} target={String(removal.path.at(-1)) + (removal.index === undefined ? "" : ` ${removal.index + 1}`)} onClose={() => setRemoval(undefined)} onConfirm={() => {
      if (!readOnly) change(removal.index === undefined ? changeManifest(doc, [{ path: removal.path, value: undefined }]) : removeManifest(doc, removal.path, removal.index));setRemoval(undefined);
    }} />}
  </div>;
}
