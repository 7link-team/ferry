import { useState, type ReactNode } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";
import { actionShells } from "./action-inputs";

const selectClass = "h-10 w-full rounded-md border border-input bg-background px-3";
const conditions = ["success()", "failure()", "always()", "!cancelled()"] as const;

export function ActionsPhase({ phase, children }: { phase: "before" | "run" | "after"; children: ReactNode }) {
  const title = t(`actions.phase.${phase}`);
  return <section aria-label={title} className="min-w-0 space-y-4 border-t border-border pt-4">
    <h4 className="text-sm font-semibold">{title}</h4>
    {children}
  </section>;
}

/** Presets edit the same YAML fields; unrecognized expressions remain untouched. */
export function ActionsConditionField({ value, readOnly, change, job = false }: {
  value: unknown; readOnly: boolean; change: (value: string | null) => void; job?: boolean;
}) {
  const [custom, setCustom] = useState(false);
  const text = value === undefined || value === null ? "" : String(value);
  const expression = text.trim().replace(/^\$\{\{\s*([\s\S]*?)\s*\}\}$/, "$1").trim();
  const known = conditions.find(condition => condition === expression);
  const mode = custom || (text !== "" && !known) ? "custom" : known ?? "";
  return <AutoField label={t("runCondition")} className="block space-y-2 text-sm">
    <select className={selectClass} aria-label={t("runCondition")} value={mode} disabled={readOnly} onChange={event => {
      setCustom(event.target.value === "custom");
      if (event.target.value !== "custom") change(event.target.value || null);
    }}>
      <option value="">{t("actions.conditionDefault")}</option>
      <option value="success()">{t("actions.conditionSuccess")}</option>
      <option value="failure()">{t("actions.conditionFailure")}</option>
      <option value="always()">{t("actions.conditionAlways")}</option>
      <option value="!cancelled()">{t("actions.conditionNotCancelled")}</option>
      <option value="custom">{t("actions.conditionCustom")}</option>
    </select>
    <p className="text-xs text-muted-foreground">{t(job ? "actions.conditionJobHelp" : "actions.conditionStepHelp")}</p>
    {mode === "custom" && <>
      <Input data-action-expression aria-label={t("actions.conditionCustom")} value={text} disabled={readOnly} placeholder="github.ref == 'refs/heads/main'" onChange={event => change(event.target.value || null)} />
      <p className="text-xs text-muted-foreground">{t("actions.conditionCustomHelp")}</p>
    </>}
  </AutoField>;
}

export function ActionsShellField({ value, readOnly, change }: {
  value: string; readOnly: boolean; change: (value: string) => void;
}) {
  const [custom, setCustom] = useState(false);
  const mode = custom || (value !== "" && !actionShells.some(shell => shell === value)) ? "custom" : value;
  const labels = { bash: "Bash · Linux / macOS", sh: "sh · POSIX", pwsh: "PowerShell Core · Windows / Linux / macOS", powershell: "Windows PowerShell", cmd: "CMD · Windows", python: "Python" };
  return <AutoField label={t("actions.shell")} className="block space-y-2 text-sm">
    <select className={selectClass} aria-label={t("actions.shell")} value={mode} disabled={readOnly} onChange={event => {
      setCustom(event.target.value === "custom");
      if (event.target.value !== "custom") change(event.target.value);
    }}>
      <option value="">{t("actions.shellDefault")}</option>
      {actionShells.map(shell => <option key={shell} value={shell}>{labels[shell]}</option>)}
      <option value="custom">{t("actions.shellCustom")}</option>
    </select>
    {mode === "custom" && <Input data-action-expression aria-label={t("actions.shellCustom")} value={value} disabled={readOnly} placeholder="bash --noprofile --norc -eo pipefail {0}" onChange={event => change(event.target.value)} />}
    <p className="text-xs text-muted-foreground">{t("actions.shellHelp")}</p>
  </AutoField>;
}
