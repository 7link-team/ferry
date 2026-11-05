import { AutoField } from "../../shared/forms/auto-form";
import { executionText, workflowText as t } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";
import { Button } from "../../shared/ui/button";
import { ComboInput } from "../../shared/forms/ComboInput";
import { StepMap } from "./ActionMapFields";
import { ActionsShellField, ActionsPhase } from "./ActionsExecutionFields";

const mapping = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);

export function ActionsRunnerFields({ value, readOnly, change }: { value: unknown; readOnly: boolean; change: (value: unknown) => void }) {
  const structured = mapping(value);
  const labels = structured ? value.labels : value;
  const label = executionText("editor.executorLabels");
  const suggestions = ["self-hosted", "linux", "windows", "macOS", "X64", "ARM64", "ubuntu-latest", "windows-latest", "macos-latest"];
  const update = (next: unknown) => change(structured ? { ...value, labels: next } : next);
  return <section className="space-y-2" aria-label={label}>
    {structured && value.group !== undefined && <AutoField label={t("actions.runnerGroup")} className="block space-y-2 text-sm"><Input data-action-expression aria-label={t("actions.runnerGroup")} value={String(value.group)} disabled={readOnly || typeof value.group !== "string"} onChange={event => change({ ...value, group: event.target.value })} /></AutoField>}
    {Array.isArray(labels) ? <>
      <h4 className="text-sm">{label}</h4>
      {labels.map((item, index) => <div className="flex items-center gap-2" key={index}><ComboInput data-action-expression options={suggestions} aria-label={`${label} ${index + 1}`} value={typeof item === "string" ? item : JSON.stringify(item)} disabled={readOnly || typeof item !== "string"} onValueChange={value => update(labels.map((entry, i) => i === index ? value : entry))} />{!readOnly && <Button type="button" variant="ghost" size="sm" aria-label={`${t("delete")} ${label} ${index + 1}`} onClick={() => update(labels.filter((_, i) => i !== index))}>{t("delete")}</Button>}</div>)}
      {!readOnly && <div className="flex justify-end"><Button type="button" variant="outline" size="sm" onClick={() => update([...labels, ""])}>{t("actions.addRunnerLabel")}</Button></div>}
    </> : <AutoField label={label} className="block space-y-2 text-sm"><ComboInput data-action-expression options={suggestions} aria-label={label} value={typeof labels === "string" ? labels : ""} disabled={readOnly || (labels !== undefined && typeof labels !== "string")} onValueChange={update} /></AutoField>}
    <p className="text-xs text-muted-foreground">{t("actions.runnerLabelsHelp")}</p>
  </section>;
}

/** Command defaults remain YAML mappings, including fields outside this form. */
function CommandDefaults({ value, readOnly, change }: { value: unknown; readOnly: boolean; change: (value: Record<string, unknown>) => void }) {
  if (value !== undefined && !mapping(value)) return <p className="text-xs text-muted-foreground">{t("actions.mappingYaml")}</p>;
  const defaults = value ?? {};
  const run = defaults.run;
  if (run !== undefined && !mapping(run)) return <p className="text-xs text-muted-foreground">{t("actions.mappingYaml")}</p>;
  return <fieldset className="space-y-3"><legend className="text-sm font-medium">{t("actions.commandDefaults")}</legend>
    {(["shell", "working-directory"] as const).map(key => {
      const update = (text: string) => {
      const next = { ...run };
      if (text === "") delete next[key]; else next[key] = text;
      change({ ...defaults, run: next });
      };
      const props = { "data-action-expression": "", "aria-label": `${t("actions.commandDefaults")} · ${key}`, disabled: readOnly || (run?.[key] !== undefined && typeof run[key] !== "string"), value: typeof run?.[key] === "string" ? run[key] : "" };
      return key === "shell" ? <ActionsShellField key={key} value={props.value} readOnly={props.disabled} change={update} /> : <AutoField key={key} label={t("workingDirectory")} className="block space-y-2 text-sm"><Input {...props} onChange={event => update(event.target.value)} /></AutoField>;
    })}
  </fieldset>;
}

export function ActionsWorkflowSettings({ value, readOnly, update }: { value: Record<string, unknown>; readOnly: boolean; update: (values: Record<string, unknown>) => void }) {
  return <>
    <p className="text-xs text-muted-foreground">{t("actions.sharedSource")}</p>
    <AutoField label={t("actions.workflowName")} className="block space-y-2 text-sm"><Input data-action-expression aria-label={t("actions.workflowName")} value={String(value.name ?? "")} disabled={readOnly} onChange={event => update({ name: event.target.value || null })} /></AutoField>
    <ActionsPhase phase="before">
    <StepMap label={t("environmentVariables")} value={value.env} readOnly={readOnly} change={env => update({ env })} />
    <CommandDefaults value={value.defaults} readOnly={readOnly} change={defaults => update({ defaults })} />
    </ActionsPhase>
  </>;
}

export function ActionsJobSettings({ value, readOnly, update }: { value: Record<string, unknown>; readOnly: boolean; update: (values: Record<string, unknown>) => void }) {
  const reusable = value.uses !== undefined;
  return <>
    {reusable ? <>
      <AutoField label={t("actions.reusableReference")} className="block space-y-2 text-sm"><Input data-action-expression aria-label={t("actions.reusableReference")} value={String(value.uses ?? "")} disabled={readOnly} onChange={event => update({ uses: event.target.value })} /></AutoField>
      <StepMap label={t("actions.workflowInputs")} value={value.with} readOnly={readOnly} change={withValue => update({ with: withValue })} />
      <StepMap label={t("actions.workflowSecrets")} value={value.secrets} readOnly={readOnly} change={secrets => update({ secrets })} />
    </> : <>
      <CommandDefaults value={value.defaults} readOnly={readOnly} change={defaults => update({ defaults })} />
    </>}
  </>;
}
