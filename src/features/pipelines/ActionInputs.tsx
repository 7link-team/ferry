import { useId, useState } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { executionText, workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { ComboInput } from "../../shared/forms/ComboInput";
import { actionPreset, type ActionInput } from "./action-inputs";
import { StepMap } from "./ActionMapFields";

function SuggestedInput({ input, value, readOnly, suggestions, change }: { input: ActionInput; value: unknown; readOnly: boolean; suggestions?: readonly string[] | undefined; change: (value: string) => void }) {
  const id = useId();
  const [expression, setExpression] = useState(false);
  const text = value === undefined ? "" : typeof value === "object" ? JSON.stringify(value) : String(value);
  const complex = value !== undefined && typeof value === "object";
  const select = input.choices && !expression && (text === "" || input.choices.includes(text));
  return <AutoField label={input.key} help={<>{t("help.actionInput")} {input.choices?.join(" / ") || input.placeholder || input.key}</>} className="block space-y-2 text-sm">
    {select ? <div className="flex items-center gap-2"><select aria-label={`with.${input.key}`} className="h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-2" disabled={readOnly} value={text} onChange={event => change(event.target.value)}><option value="">{t("actions.actionDefault")}</option>{input.choices!.map(option => <option key={option} value={option}>{option}</option>)}</select>{!readOnly && <Button type="button" variant="ghost" size="sm" aria-label={`${t("actions.expression")} ${input.key}`} onClick={() => setExpression(true)}>{t("actions.expression")}</Button>}</div>
      : suggestions ? <ComboInput data-action-expression aria-label={`with.${input.key}`} options={suggestions} value={text} placeholder={input.placeholder} readOnly={readOnly || complex} onValueChange={change} />
      : input.multiline ? <textarea id={id} aria-label={`with.${input.key}`} className="min-h-20 w-full rounded-md border border-input bg-background p-2 font-mono text-sm" value={text} placeholder={input.placeholder} readOnly={readOnly || complex} onChange={event => change(event.target.value)} />
      : <Input id={id} aria-label={`with.${input.key}`} value={text} placeholder={input.placeholder} readOnly={readOnly || complex} onChange={event => change(event.target.value)} />}
  </AutoField>;
}

export function ActionInputs({ reference, value, readOnly, checkoutBound = false, suggestions, change }: { reference: string; value: unknown; readOnly: boolean; checkoutBound?: boolean; suggestions?: Record<string, readonly string[]> | undefined; change: (value: Record<string, unknown>) => void }) {
  const preset = actionPreset(reference);
  const map = value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
  if (checkoutBound) return <><p className="text-xs text-muted-foreground">{t("actions.boundCheckout")}</p><StepMap label={executionText("editor.parameters")} value={value} readOnly change={change} /></>;
  if (!preset || (value !== undefined && (value === null || typeof value !== "object" || Array.isArray(value)))) return <StepMap label={executionText("editor.parameters")} value={value} readOnly={readOnly} change={change} />;
  const known = new Set(preset.inputs.map(input => input.key));
  const custom = Object.fromEntries(Object.entries(map).filter(([key]) => !known.has(key)));
  return <section data-action-expression aria-label={executionText("editor.parameters")} className="space-y-3">
    <h4 className="text-sm font-medium">{executionText("editor.parameters")}</h4>
    <p className="text-xs text-muted-foreground">{t("actions.suggestedInputs")}</p>
    {preset.inputs.map(input => <SuggestedInput key={`${reference}:${input.key}`} input={input} value={map[input.key]} readOnly={readOnly} suggestions={suggestions?.[input.key]} change={text => {
      const next = { ...map };
      if (text === "") delete next[input.key]; else next[input.key] = text;
      change(next);
    }} />)}
    <details open={Object.keys(custom).length ? true : undefined} className="border-t pt-3"><summary className="cursor-pointer text-sm">{t("actions.customInputs")}</summary><div className="pt-3"><StepMap label={t("actions.customInputs")} value={custom} readOnly={readOnly} change={next => change({ ...Object.fromEntries(Object.entries(map).filter(([key]) => known.has(key))), ...next })} /></div></details>
  </section>;
}
