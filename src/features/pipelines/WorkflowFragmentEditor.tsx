import { Switch } from "../../shared/ui/switch";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { lazy, Suspense, useState, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { commonText, executionText, workflowText as t } from "../../shared/i18n/i18n";
import { AutoField } from "../../shared/forms/auto-form";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { actionFragment, defaultActionRun, editActionFragment, editActionStep, parseActions } from "./actions-workflow";
import { StepMap } from "./ActionMapFields";
import { ActionInputs } from "./ActionInputs";
import { actionPresets } from "./action-inputs";
import { ActionsConditionField, ActionsShellField, ActionsPhase } from "./ActionsExecutionFields";
import { FieldCaption } from "../../shared/forms/FieldHelp";
import { artifactNames } from "./action-references";
const YamlEditor = lazy(() => import("./YamlEditor").then(module => ({ default: module.YamlEditor })));

export function WorkflowFragmentEditor({ source, job, index, original, readOnly, checkoutBound = false, checkout, onChange, renderNotification }: {
  source: string; job: string; index: number; original: string; readOnly: boolean;
  checkoutBound?: boolean;
  checkout?: ReactNode;
  renderNotification?: (inputs: Record<string, unknown>, change: (inputs: Record<string, string>) => void) => ReactNode;
  onChange: (source: string) => void;
}) {
  const { jobs } = parseActions(source);
  const step = jobs.find(row => row.id === job)!.steps[index]!;
  const isNotification = step.uses === "ferry/notify@v1";
  const isCheckout = /^actions\/checkout@/.test(String(step.uses ?? ""));
  const [formError, setFormError] = useState("");
  const update = (values: Record<string, unknown>) => {
    try { onChange(editActionStep(source, job, index, values)); setFormError(""); }
    catch (cause) { setFormError(cause instanceof Error ? cause.message : String(cause)); }
  };
  const field = (key: string, label: string) => <AutoField key={key} label={label} className="block space-y-2 text-sm"><Input data-action-expression={key !== "id" ? "" : undefined} aria-label={label} value={String(step[key] ?? "")} disabled={readOnly} onChange={event => update({ [key]: event.target.value === "" ? null : key === "timeout-minutes" && /^\d+$/.test(event.target.value) ? Number(event.target.value) : event.target.value })} /></AutoField>;
  return <div className="space-y-2">
    {formError && <p role="alert" className="text-sm text-destructive">{formError}</p>}
    {field("name", executionText("editor.stepName"))}
    {field("id", t("actions.stepId"))}
    <ActionsPhase phase="before">
    <ActionsConditionField value={step.if} readOnly={readOnly} change={value => update({ if: value })} />
    <StepMap label={t("environmentVariables")} value={step.env} readOnly={readOnly} change={value => update({ env: value })} />
    {step.uses === undefined && <>
      <ActionsShellField value={String(step.shell ?? "")} readOnly={readOnly} change={value => update({ shell: value || null })} />
      {field("working-directory", t("workingDirectory"))}
    </>}
    </ActionsPhase>
    <ActionsPhase phase="run">
    <AutoField label={executionText("editor.stepType")} className="block space-y-2 text-sm"><select aria-label={executionText("editor.stepType")} className="h-10 w-full rounded-md border border-input bg-background px-3" disabled={readOnly} value={isNotification ? "notification" : isCheckout ? "checkout" : step.uses !== undefined ? "action" : "run"} onChange={event => update(event.target.value === "notification" ? { run: null, shell: null, "working-directory": null, with: { "channel-id": "", body: "" }, uses: "ferry/notify@v1" } : event.target.value === "checkout" ? { run: null, shell: null, "working-directory": null, with: null, uses: "actions/checkout@v4" } : event.target.value === "action" ? { run: null, shell: null, "working-directory": null, uses: "actions/setup-node@v4" } : { uses: null, with: null, run: defaultActionRun(index + 1) })}><option value="run">{executionText("shared.resource.cron.commandAction")}</option><option value="checkout">{t("actions.cloneCode")}</option><option value="notification">{commonText("notifications:send")}</option><option value="action">{executionText("editor.useAction")}</option></select></AutoField>
    {isNotification && renderNotification ? renderNotification((step.with ?? {}) as Record<string, unknown>, inputs => update({ with: inputs })) : isCheckout ? <>
      {checkout}
      {!checkoutBound && <details className="space-y-2 rounded-md border p-3"><summary className="cursor-pointer text-sm">{t("actions.nativeCheckout")}</summary><ActionInputs reference={String(step.uses)} value={step.with} readOnly={readOnly} change={value => update({ with: value })} /></details>}
    </> : step.uses !== undefined ? <>
      <AutoField label={t("actions.commonAction")} className="block space-y-2 text-sm"><select aria-label={t("actions.commonAction")} className="h-10 w-full rounded-md border border-input bg-background px-2" disabled={readOnly} value={actionPresets.some(preset => preset.uses === step.uses) ? String(step.uses) : ""} onChange={event => { if (event.target.value) update({ uses: event.target.value }); }}><option value="">{t("actions.customAction")}</option>{actionPresets.map(preset => <option key={preset.uses} value={preset.uses}>{preset.uses}</option>)}</select></AutoField>
      {field("uses", executionText("editor.actionReference"))}
      <ActionInputs reference={String(step.uses)} value={step.with} readOnly={readOnly} checkoutBound={checkoutBound} suggestions={/^actions\/download-artifact@/.test(String(step.uses)) ? { name: artifactNames(jobs, job, index) } : undefined} change={value => update({ with: value })} />
    </> : <>
      <AutoField label={executionText("shared.resource.cron.commandAction")} className="block space-y-2 text-sm"><textarea data-action-expression aria-label={executionText("shared.resource.cron.commandAction")} className="min-h-32 w-full rounded-md border border-input bg-background p-3 font-mono text-sm" value={String(step.run ?? "")} readOnly={readOnly} onChange={event => update({ run: event.target.value })} /></AutoField>
    </>}
    {field("timeout-minutes", executionText("editor.timeout"))}
    </ActionsPhase>
    <ActionsPhase phase="after">
    {typeof step["continue-on-error"] === "string" ? field("continue-on-error", t("continueOnError")) : <label className="flex min-h-10 items-center gap-2 text-sm"><Switch  disabled={readOnly} checked={step["continue-on-error"] === true} onCheckedChange={checked => update({ "continue-on-error": checked })} /><FieldCaption label={t("continueOnError")} /></label>}
    </ActionsPhase>
    <ActionFragmentYaml source={source} job={job} index={index} original={original} readOnly={readOnly} onChange={onChange} />
  </div>;
}

export function ActionFragmentYaml({ source, job, index, original, readOnly, onChange }: { source: string; job: string; index?: number; original: string; readOnly: boolean; onChange: (source: string) => void }) {
  const fragment = actionFragment(source, job, index);
  const [draft, setDraft] = useState(fragment);
  const [open, setOpen] = useState(false);
  let error = "", next = source;
  try { next = editActionFragment(source, job, index, draft); }
  catch (cause) { error = cause instanceof Error ? cause.message : String(cause); }
  let originalFragment = "";
  try { originalFragment = actionFragment(original, job, index); } catch { /* New jobs/steps have no saved baseline. */ }
  const title = index === undefined ? t("actions.jobYaml") : executionText("editor.stepYaml");
  return <>
    <details className="border-t pt-3"><summary className="cursor-pointer text-sm text-muted-foreground">{t("advancedYaml")}</summary>
    <p className="text-xs text-muted-foreground">{index === undefined ? t("actions.jobYamlHelp") : executionText("editor.fragmentHelp")}</p>
    <pre className="max-h-80 overflow-auto rounded-md border bg-muted/30 p-3 text-xs"><code>{fragment}</code></pre>
    <Button type="button" variant="outline" onClick={() => { setDraft(fragment); setOpen(true); }}>{title}</Button>
    </details>
    <Dialog.Root open={open} onOpenChange={setOpen}><Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
      <Dialog.Content aria-describedby={undefined} className="fixed left-1/2 top-1/2 z-50 flex h-[85dvh] w-[min(80rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 overflow-hidden rounded-lg border bg-background p-4 shadow-lg">
        <Dialog.Title className="shrink-0 text-lg font-semibold">{title}</Dialog.Title>
        <Suspense fallback={<LoadingSkeleton variant="editor" label={executionText("editor.loadingYaml")} />}><YamlEditor value={draft} originalValue={originalFragment} path={`file:///fragment-${encodeURIComponent(job)}-${index ?? "job"}.yaml`} label={title} fillHeight readOnly={readOnly} onChange={setDraft} /></Suspense>
        {error && <p role="alert" className="max-h-24 overflow-auto whitespace-pre-wrap text-sm text-destructive">{error}</p>}
        <div className="dialog-footer"><Button type="button" variant="outline" onClick={() => setOpen(false)}>{executionText("editor.closeFragment")}</Button>{!readOnly && <Button type="button" disabled={!!error} onClick={() => { onChange(next); setOpen(false); }}>{executionText("editor.applyFragment")}</Button>}</div>
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>
  </>;
}
