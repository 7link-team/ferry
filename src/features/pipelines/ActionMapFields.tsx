import { useState } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { executionText, workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { FieldCaption } from "../../shared/forms/FieldHelp";

export function StepMap({ label, value, readOnly, change, output = false }: { label: string; value: unknown; readOnly: boolean; change: (value: Record<string, unknown>) => void; output?: boolean }) {
  const map = value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
  const [names, setNames] = useState<Record<string, string>>({});
  // An expression mapping cannot be expanded into rows without changing its meaning.
  if (value !== undefined && value !== null && (typeof value !== "object" || Array.isArray(value))) return <AutoField label={label} className="block space-y-2 text-sm"><Input aria-label={label} value={typeof value === "string" ? value : JSON.stringify(value)} readOnly /><p className="text-xs text-muted-foreground">{t("actions.mappingYaml")}</p></AutoField>;
  return <section aria-label={label} className="space-y-2"><h4 className="text-sm font-medium"><FieldCaption label={label} help={output ? t("actions.jobOutputsHelp") : undefined} /></h4>
    {Object.entries(map).map(([key, item]) => <div key={key} className="flex flex-wrap items-center justify-end gap-2">
      <Input className="min-w-0 flex-1 basis-24" placeholder={output ? t("actions.outputName") : undefined} aria-label={`${label} · ${output ? t("actions.outputName") : executionText("editor.parameterName")} ${key}`} aria-invalid={names[key] !== undefined && (!names[key]!.trim() || (names[key]!.trim() !== key && Object.hasOwn(map, names[key]!.trim())))} readOnly={readOnly} value={names[key] ?? key} onChange={event => setNames(current => ({ ...current, [key]: event.target.value }))} onBlur={() => {
        const name = names[key]?.trim();
        if (name !== undefined && (!name || (name !== key && Object.hasOwn(map, name)))) return;
        if (name && name !== key && !Object.hasOwn(map, name)) change(Object.fromEntries(Object.entries(map).map(([existing, v]) => [existing === key ? name : existing, v])));
        setNames(current => { const next = { ...current }; delete next[key]; return next; });
      }} />
      {names[key] !== undefined && (!names[key]!.trim() || (names[key]!.trim() !== key && Object.hasOwn(map, names[key]!.trim()))) && <p role="alert" className="w-full text-xs text-destructive">{t("actions.uniqueKey")}</p>}
      <Input className="min-w-0 flex-1 basis-24" placeholder={output ? t("actions.outputValue") : undefined} data-action-expression aria-label={`${label}.${key}`} readOnly={readOnly || (typeof item === "object" && item !== null)} value={typeof item === "object" && item !== null ? JSON.stringify(item) : String(item ?? "")} onChange={event => change({ ...map, [key]: event.target.value })} />
      {!readOnly && <Button type="button" variant="ghost" size="sm" aria-label={`${output ? t("actions.deleteOutput") : executionText("editor.deleteParameter")} ${label}.${key}`} onClick={() => change(Object.fromEntries(Object.entries(map).filter(([name]) => name !== key)))}>{output ? t("actions.deleteOutput") : t("delete")}</Button>}
    </div>)}
    {!readOnly && <div className="flex justify-end"><Button type="button" variant="outline" size="sm" onClick={() => { const prefix = output ? "output" : "key"; let key = prefix, index = 2; while (Object.hasOwn(map, key)) key = `${prefix}_${index++}`; change({ ...map, [key]: "" }); }}>{output ? t("actions.addOutput") : executionText("editor.addParameter")}</Button></div>}
  </section>;
}
