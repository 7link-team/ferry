import { lazy, Suspense, useEffect, useId, useMemo, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { AutoField, FieldControl } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Checkbox } from "../../shared/ui/checkbox";
import { Input } from "../../shared/ui/input";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import type { FormField } from "../../shared/resources/types";

export interface TemplateParameter {
  name: string;
  label?: string;
  type: "string" | "integer" | "boolean";
  default?: unknown;
  enum?: string[];
  min?: number;
  max?: number;
  min_length?: number;
  max_length?: number;
  secret?: boolean;
}


const CodeEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));
const t = (key: string) => commonText(`resource-domain:template.${key}`);

/** The stored JSON contract stays unchanged; ordinary parameters are edited by type. */
export function TemplateParametersEditor({ value, readOnly, onChange, onValidityChange }: {
  value: string; readOnly: boolean; onChange: (value: string) => void; onValidityChange: (blocked: boolean) => void;
}) {
  const id = useId();
  const [advanced, setAdvanced] = useState(false);
  const [preview, setPreview] = useState(false);
  const parsed = useMemo(() => {
    try {
      const rows: unknown = JSON.parse(value);
      if (!Array.isArray(rows) || rows.some(row => !row || typeof row !== "object" || Array.isArray(row)
        || typeof row.name !== "string" || !["string", "integer", "boolean"].includes(row.type))) return undefined;
      return rows as TemplateParameter[];
    } catch { return undefined; }
  }, [value]);
  const invalid = !parsed || parsed.length > 64 || parsed.some((row, index) =>
    !/^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$/.test(row.name) || row.name.length > 100
    || parsed.some((other, otherIndex) => index !== otherIndex && other.name === row.name));
  useEffect(() => {
    onValidityChange(invalid);
    return () => onValidityChange(false);
  }, [invalid, onValidityChange]);
  const update = (index: number, patch: { [K in keyof TemplateParameter]?: TemplateParameter[K] | undefined }) => {
    if (readOnly || !parsed) return;
    onChange(JSON.stringify(parsed.map((row, i) => i === index ? { ...row, ...patch } : row), null, 2));
  };
  return <section className="min-w-0 space-y-4" aria-label={t("parameters")}>
    <div className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-sm text-muted-foreground">{t("visualParametersHelp")}</p>
      <div className="flex flex-wrap gap-2">
        <Button type="button" size="sm" variant="outline" aria-expanded={preview} onClick={() => setPreview(current => !current)}>{t("formPreview")}</Button>
        <Button type="button" size="sm" variant="outline" aria-pressed={advanced} onClick={() => setAdvanced(!advanced)}>
          {advanced ? t("visualParameters") : t("advancedJson")}
        </Button>
      </div>
    </div>
    {preview && <div className="rounded-md border border-primary/30 bg-background p-3"><TemplateParameterPreview key={value} value={value} /></div>}
    {advanced || !parsed ? <div className="h-80 min-w-0 overflow-hidden rounded-md border">
      <Suspense fallback={<LoadingSkeleton variant="editor" />}><CodeEditor value={value} language="json" path={`template-parameters-${id}.json`}
        label={t("advancedJson")} readOnly={readOnly} onChange={onChange} /></Suspense>
    </div> : <>
      {!parsed.length && <p className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">{t("noParameters")}</p>}
      {parsed.map((row, index) => <fieldset key={index} data-template-parameter className="min-w-0 overflow-hidden rounded-lg border border-border border-l-4 border-l-primary/60 bg-muted/30">
        <legend className="sr-only">{row.label || row.name || `${t("parameter")} ${index + 1}`}</legend>
        <div className="flex min-w-0 flex-wrap items-center gap-2 border-b border-border bg-primary/10 px-3 py-2">
          <span aria-hidden="true" className="flex size-6 shrink-0 items-center justify-center rounded bg-primary text-xs font-semibold text-primary-foreground">{index + 1}</span>
          <h3 className="min-w-0 flex-1 break-words text-sm font-semibold [overflow-wrap:anywhere]">{row.label || row.name || t("parameter")}</h3>
          {!readOnly && <Button type="button" size="sm" variant="ghost" aria-label={`${commonText("common.delete")} ${row.label || row.name || index + 1}`} onClick={() => onChange(JSON.stringify(parsed.filter((_, i) => i !== index), null, 2))}><Trash2 className="size-4" />{commonText("common.delete")}</Button>}
        </div>
        <div className="min-w-0 space-y-3 p-3">
        {readOnly ? <ParameterDefinition row={row} /> : <>
        <div className="grid min-w-0 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          <AutoField label={t("parameterName")} required help={t("parameterNameHelp")}><Input value={row.name} required maxLength={100} disabled={readOnly} onChange={event => update(index, { name: event.target.value })} /></AutoField>
          <AutoField label={t("parameterLabel")}><Input value={row.label ?? ""} maxLength={100} disabled={readOnly} onChange={event => update(index, { label: event.target.value })} /></AutoField>
          <AutoField label={t("parameterType")}><select className="h-9 w-full rounded-md border bg-background px-3 text-sm" value={row.type} disabled={readOnly}
            onChange={event => update(index, { type: event.target.value as TemplateParameter["type"], default: undefined, enum: undefined, min: undefined, max: undefined, min_length: undefined, max_length: undefined, secret: undefined })}>
            <option value="string">{t("typeString")}</option><option value="integer">{t("typeInteger")}</option><option value="boolean">{t("typeBoolean")}</option>
          </select></AutoField>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-3">
            <label className="flex items-center gap-2 text-sm"><Checkbox checked={row.default !== undefined} disabled={readOnly} onCheckedChange={checked => update(index, { default: checked ? row.type === "boolean" ? false : row.type === "integer" ? 0 : "" : undefined })} />{t("useDefault")}</label>
            {row.default !== undefined && <AutoField label={t("defaultValue")} help={row.secret ? t("secretDefaultHelp") : undefined}>
              {row.type === "boolean" ? <select className="h-9 w-full rounded-md border bg-background px-3 text-sm" value={String(row.default)} disabled={readOnly} onChange={event => update(index, { default: event.target.value === "true" })}><option value="true">{commonText("common.yes")}</option><option value="false">{commonText("common.no")}</option></select>
                : <Input type={row.type === "integer" ? "number" : "text"} step={row.type === "integer" ? 1 : undefined} value={String(row.default)} disabled={readOnly} onChange={event => update(index, { default: row.type === "integer" ? event.target.value === "" ? "" : Number(event.target.value) : event.target.value })} />}
            </AutoField>}
          </div>
          {row.type === "string" && <label className="flex items-start gap-2 pt-1 text-sm"><Checkbox checked={!!row.secret} disabled={readOnly} onCheckedChange={checked => update(index, { secret: !!checked })} /><span>{t("secretReference")}<span className="mt-1 block text-xs text-muted-foreground">{t("secretDefaultHelp")}</span></span></label>}
        </div>
        {row.type !== "boolean" && <details className="space-y-3">
          <summary className="cursor-pointer text-sm font-medium">{t("validationRules")}</summary>
          <div className="grid gap-4 pt-3 sm:grid-cols-2">
            {(row.type === "integer" ? ["min", "max"] as const : ["min_length", "max_length"] as const).map(key => <AutoField key={key} label={t(key)}><Input type="number" step={1} min={row.type === "string" ? 0 : undefined} value={row[key] ?? ""} disabled={readOnly} onChange={event => update(index, { [key]: event.target.value === "" ? undefined : Number(event.target.value) })} /></AutoField>)}
            {row.type === "string" && <AutoField label={t("enumChoices")} help={t("enumChoicesHelp")} className="sm:col-span-2"><textarea className="min-h-24 w-full rounded-md border bg-background p-3 text-sm" value={row.enum?.join("\n") ?? ""} disabled={readOnly} onChange={event => update(index, { enum: event.target.value === "" ? undefined : event.target.value.split("\n") })} /></AutoField>}
          </div>
        </details>}
        </>}
        <code className="block min-w-0 break-all border-t border-border/70 pt-2 text-xs text-muted-foreground">{`{{ .Values.${row.name} }}`}</code>
        </div>
      </fieldset>)}
      {!readOnly && <Button type="button" variant="outline" disabled={parsed.length >= 64} onClick={() => {
        let number = parsed.length + 1;
        while (parsed.some(row => row.name === `parameter_${number}`)) number++;
        onChange(JSON.stringify([...parsed, { name: `parameter_${number}`, type: "string" }], null, 2));
      }}><Plus className="size-4" />{t("addParameter")}</Button>}
    </>}
    {invalid && <p role="alert" className="text-sm text-destructive">{t("parameterNamesInvalid")}</p>}
  </section>;
}

function ParameterDefinition({ row }: { row: TemplateParameter }) {
  const fields: [string, string][] = [
    [t("parameterName"), row.name],
    [t("parameterType"), t(row.type === "integer" ? "typeInteger" : row.type === "boolean" ? "typeBoolean" : "typeString")],
    [t("defaultValue"), row.default === undefined ? commonText("common.notSet") : row.type === "boolean" ? commonText(row.default ? "common.yes" : "common.no") : String(row.default)],
    ...(row.secret ? [[t("secretReference"), commonText("common.yes")] as [string, string]] : []),
    ...(["min", "max", "min_length", "max_length"] as const).filter(key => row[key] !== undefined).map(key => [t(key), String(row[key])] as [string, string]),
    ...(row.enum ? [[t("enumChoices"), row.enum.join(" · ")] as [string, string]] : []),
  ];
  return <dl className="grid min-w-0 grid-cols-2 gap-x-4 gap-y-2 text-sm">{fields.map(([label, value]) => <div key={label} className="min-w-0"><dt className="text-xs text-muted-foreground">{label}</dt><dd className="mt-0.5 break-words [overflow-wrap:anywhere]">{value}</dd></div>)}</dl>;
}

export function templateParameterFields(parameters: TemplateParameter[]): FormField<Record<string, unknown>>[] {
  return parameters.map((p) => ({
        key: p.name,
        get label() {
          return `${p.label || p.name}${p.secret ? commonText("resource-domain:template.secretNameSuffix") : ""}`;
        },
        type:
          p.enum || p.type === "boolean"
            ? ("select" as const)
            : p.type === "integer"
              ? ("number" as const)
              : ("text" as const),
        required: p.default === undefined,
        ...(p.enum
          ? { options: p.enum.map((value) => ({ value, label: value })) }
          : p.type === "boolean"
            ? {
                options: [
                  {
                    value: "true",
                    label: "是",
                    labelKey: "common-domain:common.yes",
                  },
                  {
                    value: "false",
                    label: "否",
                    labelKey: "common-domain:common.no",
                  },
                ],
              }
            : {}),
        get help() {
          return p.secret
            ? commonText("resource-domain:template.secretHelp")
            : `${p.name}${p.min !== undefined ? commonText("resource-domain:parameter.minimum", { value: p.min }) : ""}${p.max !== undefined ? commonText("resource-domain:parameter.maximum", { value: p.max }) : ""}${p.min_length !== undefined ? commonText("resource-domain:parameter.minLength", { value: p.min_length }) : ""}${p.max_length !== undefined ? commonText("resource-domain:parameter.maxLength", { value: p.max_length }) : ""}`;
        },
      }));
}

export function TemplateParameterPreview({ value }: { value: string }) {
  const [values, setValues] = useState<Record<string, unknown>>({});
  let parameters: TemplateParameter[];
  try { parameters = JSON.parse(value); if (!Array.isArray(parameters) || parameters.some(row => !row || typeof row.name !== "string" || !["string", "integer", "boolean"].includes(row.type))) throw new Error(); }
  catch { return <p role="alert" className="text-sm text-destructive">{t("parameterNamesInvalid")}</p>; }
  return <section className="space-y-4" aria-label={t("formPreview")}>
    <p className="text-sm text-muted-foreground">{t("formPreviewHelp")}</p>
    {!parameters.length && <p className="text-sm text-muted-foreground">{t("noParameters")}</p>}
    <div className="grid min-w-0 gap-4 sm:grid-cols-2">{templateParameterFields(parameters).map((field, index) => <AutoField key={field.key} label={field.label} help={field.help}>
      <FieldControl name={field.key} field={{ name: field.label, type: field.type === "select" ? "select" : field.type === "number" ? "number" : "text", config: { ...(Array.isArray(field.options) ? { options: field.options } : {}), ...(parameters[index]?.min === undefined ? {} : { min: parameters[index]!.min! }), ...(parameters[index]?.max === undefined ? {} : { max: parameters[index]!.max! }) } }} value={values[field.key] ?? parameters[index]?.default ?? ""} onChange={value => setValues(current => ({ ...current, [field.key]: value }))} />
    </AutoField>)}</div>
  </section>;
}
