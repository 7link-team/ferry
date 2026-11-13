import { lazy, Suspense, useEffect, useId, useRef } from "react";
import { parseDocument, stringify } from "yaml";
import { ManifestEditor } from "../deployments/ManifestEditor";
import { commonText } from "../../shared/i18n/i18n";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import type { TemplateParameter } from "./TemplateParametersEditor";

const TemplateCodeEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));

/** Format loaded JSON only; preserve authored YAML, comments and incomplete input. */
export function applicationTemplateSource(value: string) {
  if (!value.trimStart().startsWith("{")) return value;
  try { return stringify(JSON.parse(value)); } catch { return value; }
}

export function ApplicationTemplateEditor({ value, readOnly, onChange, onValidityChange, workspace, templateParameters }: {
  value: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onValidityChange: (blocked: boolean) => void;
  workspace: string;
  templateParameters: TemplateParameter[];
}) {
  const authored = useRef<string | undefined>(undefined);
  const editorId = useId();
  const source = authored.current === value ? value : applicationTemplateSource(value);
  const doc = parseDocument(source);
  const blueprint = doc.get("apiVersion") === "ferry/blueprint-v1";
  const syntaxError = doc.errors[0]?.message;
  useEffect(() => { if (blueprint) onValidityChange(!!syntaxError); }, [blueprint, syntaxError, onValidityChange]);
  const change = (next: string) => { authored.current = next; onChange(next); };
  if (!blueprint) return <ManifestEditor value={source} mode="" workspace={workspace} templateParameters={templateParameters} readOnly={readOnly} onChange={change} onValidityChange={onValidityChange} />;
  return <div className="min-w-0 space-y-3">
    <div className="h-[28rem] min-w-0 overflow-hidden rounded-md border">
      <Suspense fallback={<LoadingSkeleton variant="editor" />}><TemplateCodeEditor value={source} language="yaml" path={`${workspace}/application-template-${editorId}.yaml`} label={commonText("resource-domain:template.appContent")} readOnly={readOnly} onChange={change} /></Suspense>
    </div>
    {syntaxError && <p role="alert" className="whitespace-pre-wrap text-sm text-destructive">{syntaxError}</p>}
  </div>;
}
