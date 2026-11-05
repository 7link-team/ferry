import { useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { actionEnvironment, expression, insertActionReference, outputSources, referenceProperty } from "./action-references";
import type { ActionJob } from "./actions-workflow";

export function ActionsVariableHelper({ jobs, job, step, envNames, target, close }: { jobs: ActionJob[]; job: string | undefined; step: number | undefined; envNames: string[]; target: RefObject<HTMLInputElement | HTMLTextAreaElement | null>; close: () => void }) {
  const [search, setSearch] = useState("");
  const [syntax, setSyntax] = useState("expression");
  const [outputName, setOutputName] = useState("");
  const sources = outputSources(jobs, job, step);
  const [selectedSource, setSelectedSource] = useState("");
  const [error, setError] = useState("");
  const source = sources.find(item => item.id === selectedSource) ?? sources[0];
  const insert = (value: string) => { if (!target.current || !insertActionReference(target.current, value)) setError(t("actions.focusTarget")); else setError(""); };
  const variables = actionEnvironment.flatMap(([key, context]) => {
    const value = syntax === "expression" ? (context ? expression(context) : "") : syntax === "pwsh" ? `$env:${key}` : syntax === "cmd" ? `%${key}%` : `$${key}`;
    return value ? [{ key, value }] : [];
  });
  const references = [...variables, ...envNames.map(key => ({ key: `env.${key}`, value: expression(`env${referenceProperty(key)}`) })), ...sources.flatMap(item => item.outputs.map(key => ({ key: `${item.id}.${key}`, value: expression(item.path + referenceProperty(key)) })))].filter(item => `${item.key} ${item.value}`.toLowerCase().includes(search.toLowerCase()));
  return createPortal(<aside data-action-helper role="region" aria-label={t("actions.variableHelper")} className="fixed bottom-4 right-4 z-[60] flex max-h-[min(70dvh,620px)] w-[min(23rem,calc(100vw-2rem))] flex-col gap-3 rounded-lg border bg-popover p-4 text-popover-foreground shadow-xl" onKeyDown={event => { if (event.key === "Escape") { event.stopPropagation(); close(); } }}>
    <div className="flex items-center justify-between gap-2"><h3 className="font-semibold">{t("actions.variableHelper")}</h3><Button type="button" variant="ghost" size="icon" aria-label={t("close")} onClick={close}><X className="size-4" /></Button></div>
    <p className="text-xs text-muted-foreground">{t("actions.variableHelp")}</p>
    <Input aria-label={t("actions.variableSearch")} placeholder={t("actions.variableSearch")} value={search} onChange={event => setSearch(event.target.value)} />
    <select aria-label={t("actions.variableSyntax")} className="h-9 rounded-md border border-input bg-background px-2 text-sm" value={syntax} onChange={event => setSyntax(event.target.value)}><option value="expression">${"{{ }}"} · {t("actions.expression")}</option><option value="bash">Bash / sh · $NAME</option><option value="pwsh">PowerShell · $env:NAME</option><option value="cmd">CMD · %NAME%</option></select>
    <p className="text-xs text-muted-foreground">{t("actions.variableAvailability")}</p>
    {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    <div className="min-h-0 flex-1 overflow-y-auto">{references.map(item => <button type="button" key={item.key} className="block w-full rounded px-2 py-2 text-left hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring" title={item.value} onClick={() => insert(item.value)}><span className="block text-sm font-medium">{item.key}</span><code className="block break-all text-xs text-muted-foreground">{item.value}</code></button>)}{!references.length && <p className="p-2 text-sm text-muted-foreground">{t("noNodes")}</p>}</div>
    <details className="border-t pt-3"><summary className="cursor-pointer text-sm">{t("actions.outputReference")}</summary><div className="mt-3 space-y-2">
      <p className="text-xs text-muted-foreground">{t("actions.outputReferenceHelp")}</p>
      {source && <><select aria-label={t("actions.outputSource")} className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm" value={source.id} onChange={event => setSelectedSource(event.target.value)}>{sources.map(item => <option key={item.id} value={item.id}>{item.id}</option>)}</select><Input aria-label={t("actions.outputName")} placeholder={t("actions.outputName")} value={outputName} onChange={event => setOutputName(event.target.value)} /><div className="flex justify-end"><Button type="button" size="sm" disabled={!outputName.trim()} onClick={() => insert(expression(source.path + referenceProperty(outputName.trim())))}>{t("actions.insertReference")}</Button></div></>}
    </div></details>
  </aside>, document.body);
}
