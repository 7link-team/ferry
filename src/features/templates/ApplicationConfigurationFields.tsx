import { lazy, Suspense } from "react";
import { parse } from "yaml";
import { commonText } from "../../shared/i18n/i18n";
import type { AnyRow } from "../../shared/resources/config";
import type { FormField } from "../../shared/resources/types";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { ApplicationTemplateEditor } from "./ApplicationTemplateEditor";
import { TemplateParametersEditor, type TemplateParameter } from "./TemplateParametersEditor";

const CodeEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));
const Markdown = lazy(() => import("../maintenance/MarkdownValuePreview"));
const ignore = () => {};
const record = (value: unknown): value is AnyRow => !!value && typeof value === "object" && !Array.isArray(value);

function references(draft: AnyRow): { name: string; path: string }[] {
  try {
    const definition = parse(String(draft.content ?? draft.manifest ?? ""), { maxAliasCount: 50 });
    const services = definition?.kind === "Blueprint" ? definition.roles?.map((role: AnyRow) => role.service) : definition?.services;
    return (Array.isArray(services) ? services : []).flatMap(service => Array.isArray(service?.files) ? service.files : [])
      .filter(file => record(file) && typeof file.fromTemplate === "string")
      .map(file => ({ name: file.fromTemplate, path: typeof file.path === "string" ? file.path : "" }));
  } catch { return []; }
}

/** Package contents are shown only when supplied by the record, never fetched from a newer template. */
export function applicationHasFiles(draft: AnyRow) {
  return Array.isArray(draft.files) && draft.files.length > 0 || references(draft).length > 0;
}

export function ApplicationFiles({ draft }: { draft: AnyRow }) {
  const refs = references(draft);
  const names: string[] = Array.isArray(draft.files) ? draft.files.map(String) : [...new Set(refs.map(file => file.name))];
  const files = Array.isArray(draft.package_files) ? draft.package_files as { name: string; content: string }[] : [];
  return <div className="min-w-0 space-y-3" data-application-files>
    <p className="text-sm text-muted-foreground">{commonText("resource-domain:application.filesHelp")}</p>
    {names.map(name => {
      const file = files.find(item => item.name === name);
      const paths = refs.filter(item => item.name === name).map(item => item.path).filter(Boolean);
      return <details key={name} className="min-w-0 rounded-md border p-3">
        <summary className="min-h-11 cursor-pointer break-all py-2 font-medium">{name}</summary>
        <p className="mb-3 break-all text-sm text-muted-foreground">{commonText("resource-domain:market.fileTargets")}: {paths.length ? paths.join(", ") : commonText("resource-domain:market.fileUnreferenced")}</p>
        {file ? <div className="h-72 min-w-0 overflow-hidden rounded-md border"><Suspense fallback={<LoadingSkeleton variant="editor" />}>
          <CodeEditor value={file.content} language="plaintext" path={`application/${draft.id}/${name}`} label={name} readOnly onChange={ignore} />
        </Suspense></div> : <p role="status" className="text-sm text-muted-foreground">{commonText("resource-domain:market.fileUnavailable")}</p>}
      </details>;
    })}
  </div>;
}

/** Canonical content controls for all three resource forms. Resource-specific actions stay with their owners. */
export function applicationConfigurationField<T>(field: FormField<T>, workspace: string): FormField<T> {
  if (field.key === "content" || field.key === "manifest") return { ...field, labelKey: "resource-domain:template.definition", type: "custom", span: 2,
    render: ({ draft, readOnly, onPatch, onValidityChange }) => {
      let parameters: TemplateParameter[] = [];
      try { const parsed = JSON.parse(String((draft as AnyRow).parameters ?? "[]")); if (Array.isArray(parsed)) parameters = parsed; } catch { /* Parameter validation remains in its own control. */ }
      return <ApplicationTemplateEditor value={String((draft as AnyRow)[field.key] ?? "")} workspace={workspace} mode={String((draft as AnyRow).mode ?? "")}
        {...(field.key === "content" ? { templateParameters: parameters } : {})} readOnly={readOnly} onChange={value => onPatch({ [field.key]: value } as Partial<T>)} onValidityChange={onValidityChange} />;
    } };
  if (field.key === "parameters") return { ...field, type: "custom", span: 2,
    render: ({ draft, readOnly, onPatch, onValidityChange }) => <TemplateParametersEditor value={String((draft as AnyRow).parameters ?? "[]")} readOnly={readOnly}
      onChange={parameters => onPatch({ parameters } as unknown as Partial<T>)} onValidityChange={onValidityChange} /> };
  if (field.key === "files") return { ...field, type: "custom", span: 2,
    when: draft => applicationHasFiles(draft as AnyRow),
    render: ({ draft }) => <ApplicationFiles draft={draft as AnyRow} /> };
  if (field.key === "values") return { ...field, type: "custom", span: 2, when: draft => record((draft as AnyRow).values),
    render: ({ draft }) => <ApplicationValues value={(draft as AnyRow).values as AnyRow} /> };
  if (field.key === "readme") return { ...field, type: "custom", span: 2, when: draft => !!(draft as AnyRow).readme,
    render: ({ draft }) => <Suspense fallback={<LoadingSkeleton variant="inline" />}><Markdown value={String((draft as AnyRow).readme ?? "")} label={commonText("resource-domain:market.readme")} /></Suspense> };
  return field;
}

/** Actual instance values are distinct from template parameter definitions. No inferred schema or resolved secrets. */
export function ApplicationValues({ value }: { value: Record<string, unknown> }) {
  if (!Object.keys(value).length) return <p className="text-sm text-muted-foreground">{commonText("resource-domain:template.noParameters")}</p>;
  return <dl className="grid min-w-0 gap-4 sm:grid-cols-2" data-application-values>{Object.entries(value).map(([name, item]) => <div key={name} className="min-w-0">
    <dt className="break-all text-sm text-muted-foreground">{name}</dt>
    <dd className="mt-1 min-h-11 whitespace-pre-wrap break-words rounded-md border border-input bg-background px-3 py-2 text-sm [overflow-wrap:anywhere]">{typeof item === "boolean" ? commonText(item ? "common.yes" : "common.no") : typeof item === "string" ? item || '""' : JSON.stringify(item)}</dd>
  </div>)}</dl>;
}
